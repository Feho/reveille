// SPDX-License-Identifier: GPL-2.0-only

//! Bounded network orchestration. Per-server failures become data, not sweep errors.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::model::{
    BrowseReport, ClientCapacity, ClientsReported, DownloadFlags, GamePort, JoinWindowSeconds,
    MasterEndpoint, NonResult, NonResultReason, PingMillis, ProbeOutcome, ProbeStage,
    ReservedSlots, Server, SimulatedClientsReported, TargetGame,
};
use super::protocol::{
    CryptoError, FieldMap, OOB_SEND_HEADER, ParseError, build_master_query, parse_gamespy_status,
    parse_master_challenge, parse_master_response, parse_oob_getinfo, parse_oob_getstatus,
};
use crate::bsp::Checksum;

// spike_masterlist.py, originally master constants in the GameSpy integration.
const MASTER_HOST: &str = "master.333networks.com";
const MASTER_PORT: u16 = 28_900;
// GameSpy query protocol used by sv_gamespy.c.
const GAMESPY_STATUS_REQUEST: &[u8] = b"\\status\\";
const MAX_UDP_PACKET: usize = 65_535;
const MAX_MASTER_RESPONSE: usize = 4 * 1024 * 1024;

/// Runtime limits and target for one browse sweep.
#[derive(Clone, Copy, Debug)]
pub struct BrowseConfig {
    /// Game family to request from the master.
    pub target: TargetGame,
    /// Maximum endpoints to inspect; `None` inspects all registrations.
    pub limit: Option<usize>,
    /// Maximum simultaneous per-server probes.
    pub concurrency: usize,
    /// Deadline for each master-server I/O operation.
    pub master_timeout: Duration,
    /// Deadline for each per-server UDP request.
    pub probe_timeout: Duration,
}

impl Default for BrowseConfig {
    fn default() -> Self {
        Self {
            target: TargetGame::AlliedAssault,
            limit: None,
            concurrency: 16,
            master_timeout: Duration::from_secs(15),
            probe_timeout: Duration::from_millis(2_500),
        }
    }
}

/// Failure of one request. Per-server instances are converted to [`NonResult`].
#[derive(Debug, Error)]
pub enum RequestError {
    /// The remote endpoint did not complete an operation before its deadline.
    #[error("request timed out")]
    Timeout,
    /// Socket I/O failed.
    #[error("network request failed")]
    Network(#[source] io::Error),
    /// A reply was received but could not be parsed.
    #[error("malformed reply: {0}")]
    Parse(#[from] ParseError),
    /// The master query could not be encoded.
    #[error("could not encode master validation: {0}")]
    Crypto(#[from] CryptoError),
    /// The master closed the connection before sending a greeting.
    #[error("master closed the connection before its greeting")]
    EmptyMasterGreeting,
    /// The master response exceeded a defensive bound.
    #[error("master response exceeded {MAX_MASTER_RESPONSE} bytes")]
    MasterResponseTooLarge,
}

/// Sweep-level failure. Individual server failures never use this type.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// The master list itself could not be obtained.
    #[error("could not fetch the {game} master list: {source}", game = target.label())]
    Master {
        /// Requested family.
        target: TargetGame,
        /// Master request failure.
        #[source]
        source: RequestError,
    },
    /// An internal probe task failed unexpectedly.
    #[error("internal discovery task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// Fetch the master list and inspect registered servers with bounded concurrency.
///
/// # Errors
///
/// Returns an error only when the master list cannot be obtained or an internal task panics.
/// Unreachable and malformed individual servers are retained in the returned report.
pub async fn browse(config: BrowseConfig) -> Result<BrowseReport, DiscoveryError> {
    let endpoints = fetch_master(config.target, config.master_timeout)
        .await
        .map_err(|source| DiscoveryError::Master {
            target: config.target,
            source,
        })?;
    let registered = endpoints.len();
    let limit = config.limit.unwrap_or(registered).min(registered);
    let mut endpoints = endpoints.into_iter().take(limit);
    let concurrency = config.concurrency.max(1);
    let mut tasks = JoinSet::new();
    let mut outcomes = Vec::with_capacity(limit);

    loop {
        while tasks.len() < concurrency {
            let Some(endpoint) = endpoints.next() else {
                break;
            };
            tasks.spawn(probe_server(endpoint, config.probe_timeout));
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        outcomes.push(result?);
    }
    outcomes.sort_by_key(|outcome| outcome.endpoint);

    Ok(BrowseReport {
        target: config.target,
        registered,
        outcomes,
    })
}

/// Send MOHAA `getstatus` with the required five-byte directional header.
///
/// # Errors
///
/// Returns a timeout, network, or parse error for this one request.
pub async fn query_getstatus(
    address: Ipv4Addr,
    port: GamePort,
    deadline: Duration,
) -> Result<FieldMap, RequestError> {
    let mut request = Vec::from(OOB_SEND_HEADER);
    request.extend_from_slice(b"getstatus");
    let response = udp_request(address, port.get(), &request, deadline).await?;
    parse_oob_getstatus(&response).map_err(RequestError::from)
}

/// Send MOHAA `getinfo` with the required five-byte directional header.
///
/// # Errors
///
/// Returns a timeout, network, or parse error for this one request.
pub async fn query_getinfo(
    address: Ipv4Addr,
    port: GamePort,
    challenge: &str,
    deadline: Duration,
) -> Result<FieldMap, RequestError> {
    let mut request = Vec::from(OOB_SEND_HEADER);
    request.extend_from_slice(b"getinfo ");
    request.extend_from_slice(challenge.as_bytes());
    let response = udp_request(address, port.get(), &request, deadline).await?;
    parse_oob_getinfo(&response).map_err(RequestError::from)
}

async fn fetch_master(
    target: TargetGame,
    deadline: Duration,
) -> Result<Vec<MasterEndpoint>, RequestError> {
    let mut stream = timeout(deadline, TcpStream::connect((MASTER_HOST, MASTER_PORT)))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;

    let mut greeting = [0_u8; 256];
    let length = timeout(deadline, stream.read(&mut greeting))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;
    if length == 0 {
        return Err(RequestError::EmptyMasterGreeting);
    }
    let challenge = parse_master_challenge(&greeting[..length])?;
    let query = build_master_query(target, &challenge)?;
    timeout(deadline, stream.write_all(query.as_bytes()))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;

    let mut response = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let length = timeout(deadline, stream.read(&mut chunk))
            .await
            .map_err(|_| RequestError::Timeout)?
            .map_err(RequestError::Network)?;
        if length == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..length]);
        if response.len() > MAX_MASTER_RESPONSE {
            return Err(RequestError::MasterResponseTooLarge);
        }
        if response
            .windows(b"\\final\\".len())
            .any(|window| window == b"\\final\\")
        {
            break;
        }
    }
    parse_master_response(&response).map_err(RequestError::from)
}

async fn probe_server(endpoint: MasterEndpoint, deadline: Duration) -> ProbeOutcome {
    let gamespy = match query_gamespy_status(endpoint, deadline).await {
        Ok(fields) => fields,
        Err(error) => return partial(endpoint, false, ProbeStage::GameSpyStatus, error.into()),
    };
    let game_port = match game_port_from_gamespy(&gamespy) {
        Ok(port) => port,
        Err(reason) => return partial(endpoint, true, ProbeStage::HostPort, reason),
    };

    let status = match query_getstatus(endpoint.address, game_port, deadline).await {
        Ok(fields) => fields,
        Err(error) => return partial(endpoint, true, ProbeStage::GetStatus, error.into()),
    };
    ProbeOutcome {
        endpoint,
        gamespy_reachable: true,
        server: Some(build_server(endpoint, game_port, &gamespy, &status)),
        non_result: None,
    }
}

async fn query_gamespy_status(
    endpoint: MasterEndpoint,
    deadline: Duration,
) -> Result<FieldMap, RequestError> {
    let response = udp_request(
        endpoint.address,
        endpoint.query_port.get(),
        GAMESPY_STATUS_REQUEST,
        deadline,
    )
    .await?;
    parse_gamespy_status(&response).map_err(RequestError::from)
}

async fn udp_request(
    address: Ipv4Addr,
    port: u16,
    request: &[u8],
    deadline: Duration,
) -> Result<Vec<u8>, RequestError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(RequestError::Network)?;
    socket
        .connect(SocketAddr::V4(SocketAddrV4::new(address, port)))
        .await
        .map_err(RequestError::Network)?;
    timeout(deadline, socket.send(request))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;
    let mut response = vec![0_u8; MAX_UDP_PACKET];
    let length = timeout(deadline, socket.recv(&mut response))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;
    response.truncate(length);
    Ok(response)
}

fn build_server(
    endpoint: MasterEndpoint,
    game_port: GamePort,
    gamespy: &FieldMap,
    status: &FieldMap,
) -> Server {
    let clients_reported = parse_u32(gamespy, "numplayers").map(ClientsReported::new);
    let simulated_clients_reported = parse_u32(gamespy, "minplayers").and_then(|minimum| {
        clients_reported
            .map(|clients| SimulatedClientsReported::new(minimum.saturating_sub(clients.get())))
    });
    Server {
        endpoint,
        game_port,
        hostname: nonempty(status, "sv_hostname")
            .or_else(|| nonempty(gamespy, "hostname"))
            .unwrap_or_default(),
        game_name: nonempty(gamespy, "gamename"),
        game_version: nonempty(gamespy, "gamever"),
        version: nonempty(status, "version"),
        protocol: nonempty(status, "protocol"),
        current_map: nonempty(status, "mapname").or_else(|| nonempty(gamespy, "mapname")),
        rotation: status
            .get("sv_maplist")
            .map(|rotation| rotation.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        allow_download: parse_u32(status, "sv_allowDownload").map(DownloadFlags::new),
        map_checksum: status
            .get("sv_mapChecksum")
            .and_then(|value| value.parse::<i32>().ok())
            .map(Checksum::new),
        pr_downloads: nonempty(status, "pr_downloads"),
        minimum_ping: parse_u32(status, "sv_minPing").map(PingMillis::new),
        maximum_ping: parse_u32(status, "sv_maxPing").map(PingMillis::new),
        join_window: parse_u32(status, "g_allowjointime").map(JoinWindowSeconds::new),
        reserved_slots: parse_u32(status, "sv_privateClients").map(ReservedSlots::new),
        clients_reported,
        simulated_clients_reported,
        client_capacity: parse_u32(gamespy, "maxplayers")
            .or_else(|| parse_u32(status, "sv_maxclients"))
            .map(ClientCapacity::new),
        pure: nonempty(status, "pure"),
    }
}

fn game_port_from_gamespy(fields: &FieldMap) -> Result<GamePort, NonResultReason> {
    let Some(value) = fields.get("hostport").filter(|value| !value.is_empty()) else {
        return Err(NonResultReason::MissingHostPort);
    };
    match value.parse::<u16>() {
        Ok(0) | Err(_) => Err(NonResultReason::Malformed {
            message: format!("invalid hostport {value:?}"),
        }),
        Ok(port) => Ok(GamePort::new(port)),
    }
}

fn parse_u32(fields: &FieldMap, key: &str) -> Option<u32> {
    fields.get(key)?.parse().ok()
}

fn nonempty(fields: &FieldMap, key: &str) -> Option<String> {
    fields.get(key).filter(|value| !value.is_empty()).cloned()
}

fn partial(
    endpoint: MasterEndpoint,
    gamespy_reachable: bool,
    stage: ProbeStage,
    reason: NonResultReason,
) -> ProbeOutcome {
    ProbeOutcome {
        endpoint,
        gamespy_reachable,
        server: None,
        non_result: Some(NonResult { stage, reason }),
    }
}

impl From<RequestError> for NonResultReason {
    fn from(error: RequestError) -> Self {
        match error {
            RequestError::Timeout => Self::Timeout,
            RequestError::Network(source) => Self::Network {
                message: source.to_string(),
            },
            RequestError::Parse(source) => Self::Malformed {
                message: source.to_string(),
            },
            RequestError::Crypto(source) => Self::Malformed {
                message: source.to_string(),
            },
            RequestError::EmptyMasterGreeting => Self::Malformed {
                message: "empty master greeting".to_owned(),
            },
            RequestError::MasterResponseTooLarge => Self::Malformed {
                message: "master response too large".to_owned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{build_server, game_port_from_gamespy};
    use crate::discovery::{FieldMap, MasterEndpoint, QueryPort};

    #[test]
    fn uses_hostport_data_without_deriving_from_the_query_port() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 7),
            query_port: QueryPort::new(12_300),
        };
        let gamespy = FieldMap::from([
            ("hostport".to_owned(), "23900".to_owned()),
            ("numplayers".to_owned(), "3".to_owned()),
        ]);
        let status = FieldMap::new();

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
        );
        assert_eq!(server.endpoint.query_port.get(), 12_300);
        assert_eq!(server.game_port.get(), 23_900);
    }

    #[test]
    fn records_missing_and_malformed_hostports_as_non_results() {
        assert_eq!(
            game_port_from_gamespy(&FieldMap::new()),
            Err(crate::discovery::NonResultReason::MissingHostPort)
        );
        let malformed = FieldMap::from([("hostport".to_owned(), "not-a-port".to_owned())]);
        assert!(matches!(
            game_port_from_gamespy(&malformed),
            Err(crate::discovery::NonResultReason::Malformed { .. })
        ));
    }

    #[test]
    fn builds_the_compatibility_model_from_reported_fields() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 8),
            query_port: QueryPort::new(12_300),
        };
        let gamespy = FieldMap::from([
            ("hostport".to_owned(), "23900".to_owned()),
            ("gamename".to_owned(), "mohaa".to_owned()),
            ("gamever".to_owned(), "1.11".to_owned()),
            ("hostname".to_owned(), "Frozen".to_owned()),
            ("numplayers".to_owned(), "3".to_owned()),
            ("minplayers".to_owned(), "5".to_owned()),
            ("maxplayers".to_owned(), "16".to_owned()),
        ]);
        let status = FieldMap::from([
            ("version".to_owned(), "engine build".to_owned()),
            ("protocol".to_owned(), "8".to_owned()),
            ("mapname".to_owned(), "dm/current".to_owned()),
            ("sv_maplist".to_owned(), "dm/current obj/next".to_owned()),
            ("sv_allowDownload".to_owned(), "5".to_owned()),
            ("sv_mapChecksum".to_owned(), "-42".to_owned()),
            (
                "pr_downloads".to_owned(),
                "https://example.invalid/list".to_owned(),
            ),
            ("sv_minPing".to_owned(), "20".to_owned()),
            ("sv_maxPing".to_owned(), "250".to_owned()),
            ("g_allowjointime".to_owned(), "10".to_owned()),
            ("sv_privateClients".to_owned(), "2".to_owned()),
        ]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
        );
        assert_eq!(
            server
                .clients_reported
                .map(crate::discovery::ClientsReported::get),
            Some(3)
        );
        assert_eq!(
            server
                .simulated_clients_reported
                .map(crate::discovery::SimulatedClientsReported::get),
            Some(2)
        );
        assert_eq!(
            server
                .client_capacity
                .map(crate::discovery::ClientCapacity::get),
            Some(16)
        );
        assert_eq!(
            server
                .allow_download
                .map(crate::discovery::DownloadFlags::get),
            Some(5)
        );
        assert_eq!(
            server.map_checksum.map(crate::bsp::Checksum::get),
            Some(-42)
        );
        assert_eq!(server.rotation, ["dm/current", "obj/next"]);
        assert_eq!(
            server.minimum_ping.map(crate::discovery::PingMillis::get),
            Some(20)
        );
        assert_eq!(
            server.maximum_ping.map(crate::discovery::PingMillis::get),
            Some(250)
        );
        assert_eq!(
            server
                .join_window
                .map(crate::discovery::JoinWindowSeconds::get),
            Some(10)
        );
        assert_eq!(
            server
                .reserved_slots
                .map(crate::discovery::ReservedSlots::get),
            Some(2)
        );
    }
}
