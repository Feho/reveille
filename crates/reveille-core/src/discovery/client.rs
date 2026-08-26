// SPDX-License-Identifier: GPL-2.0-only

//! Bounded network orchestration. Per-server failures become data, not sweep errors.

use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::model::{
    BotsReported, BrowseReport, ClientCapacity, ClientsReported, DownloadFlags, GamePort,
    JoinWindowSeconds, MasterEndpoint, NonResult, NonResultReason, PingMillis, ProbeOutcome,
    ProbeStage, ReportedOccupancy, ReservedSlots, RoundTripMillis, Server, TargetGame,
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

/// Incremental progress from one browse sweep.
///
/// Streamed outcomes are pre-deduplication: a registration reported here can still be demoted to
/// a [`NonResultReason::DuplicateEndpoint`] once the sweep completes. The [`BrowseReport`] returned
/// by [`browse_streaming`] is the authoritative result, and a consumer that displays streamed rows
/// must reconcile against it. Keeping dedup at the end is what makes retention deterministic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BrowseEvent {
    /// The master list was fetched. Emitted once, before any probe.
    Registered {
        /// Registrations returned by the master, before any limit is applied.
        registered: usize,
        /// Endpoints this sweep will actually inspect.
        inspected: usize,
    },
    /// One endpoint finished probing.
    Outcome(Box<ProbeOutcome>),
}

/// Fetch the master list and inspect registered servers with bounded concurrency.
///
/// # Errors
///
/// Returns an error only when the master list cannot be obtained or an internal task panics.
/// Unreachable, malformed, and duplicate individual registrations are retained in the report.
pub async fn browse(config: BrowseConfig) -> Result<BrowseReport, DiscoveryError> {
    sweep(config, None).await
}

/// Run a sweep, reporting progress on `sink` as it happens.
///
/// Closing the receiver cancels the sweep: no further endpoint is probed and the report describes
/// only the endpoints already inspected. This is the whole cancellation mechanism — dropping the
/// receiver is the signal, so no token type and no extra dependency is needed.
///
/// # Errors
///
/// Same as [`browse`].
pub async fn browse_streaming(
    config: BrowseConfig,
    sink: mpsc::Sender<BrowseEvent>,
) -> Result<BrowseReport, DiscoveryError> {
    sweep(config, Some(&sink)).await
}

async fn sweep(
    config: BrowseConfig,
    sink: Option<&mpsc::Sender<BrowseEvent>>,
) -> Result<BrowseReport, DiscoveryError> {
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
    let mut cancelled = !emit(
        sink,
        BrowseEvent::Registered {
            registered,
            inspected: limit,
        },
    )
    .await;

    loop {
        while !cancelled && tasks.len() < concurrency {
            let Some(endpoint) = endpoints.next() else {
                break;
            };
            tasks.spawn(probe_server(endpoint, config.probe_timeout));
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        let outcome = result?;
        if !emit(sink, BrowseEvent::Outcome(Box::new(outcome.clone()))).await {
            cancelled = true;
        }
        outcomes.push(outcome);
    }
    record_duplicate_game_endpoints(&mut outcomes);

    Ok(BrowseReport {
        target: config.target,
        registered,
        outcomes,
    })
}

/// Deliver one event. Returns `false` once the consumer has stopped listening.
async fn emit(sink: Option<&mpsc::Sender<BrowseEvent>>, event: BrowseEvent) -> bool {
    match sink {
        None => true,
        Some(sink) => sink.send(event).await.is_ok(),
    }
}

fn record_duplicate_game_endpoints(outcomes: &mut [ProbeOutcome]) {
    outcomes.sort_by_key(|outcome| outcome.endpoint);
    let mut seen = HashSet::new();
    for outcome in outcomes {
        let Some(server) = outcome.server.as_ref() else {
            continue;
        };
        let key = (server.endpoint.address, server.game_port);
        if seen.insert(key) {
            continue;
        }
        let game_port = server.game_port;
        outcome.server = None;
        outcome.non_result = Some(NonResult {
            stage: ProbeStage::EndpointDeduplication,
            reason: NonResultReason::DuplicateEndpoint { game_port },
        });
    }
}

/// Inspect one already-known registration, without asking the master for a list.
///
/// This is exactly the probe [`browse`] runs per endpoint, at the caller's deadline, so a server
/// that answers here answered the same question as every server in a sweep. That matters: a
/// remembered server checked on gentler terms than the list would be listed on terms the list
/// never offered.
///
/// A failure is carried in the outcome's `non_result` rather than returned as an error — one
/// endpoint failing is information about that endpoint.
pub async fn inspect_endpoint(endpoint: MasterEndpoint, deadline: Duration) -> ProbeOutcome {
    probe_server(endpoint, deadline).await
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
    query_getstatus_measured(address, port, deadline)
        .await
        .map(|(fields, _)| fields)
}

/// `getstatus`, keeping the round trip the request already had to wait for.
///
/// The sweep is the only caller that needs the timing, and it gets it for free: this is the same
/// single request, not an extra probe aimed at a stranger's server.
async fn query_getstatus_measured(
    address: Ipv4Addr,
    port: GamePort,
    deadline: Duration,
) -> Result<(FieldMap, RoundTripMillis), RequestError> {
    let mut request = Vec::from(OOB_SEND_HEADER);
    request.extend_from_slice(b"getstatus");
    let (response, round_trip) = udp_request(address, port.get(), &request, deadline).await?;
    let fields = parse_oob_getstatus(&response).map_err(RequestError::from)?;
    Ok((fields, round_trip))
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
    let (response, _) = udp_request(address, port.get(), &request, deadline).await?;
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

    let (status, round_trip) =
        match query_getstatus_measured(endpoint.address, game_port, deadline).await {
            Ok(measured) => measured,
            Err(error) => return partial(endpoint, true, ProbeStage::GetStatus, error.into()),
        };
    ProbeOutcome {
        endpoint,
        gamespy_reachable: true,
        server: Some(build_server(
            endpoint, game_port, &gamespy, &status, round_trip,
        )),
        non_result: None,
    }
}

async fn query_gamespy_status(
    endpoint: MasterEndpoint,
    deadline: Duration,
) -> Result<FieldMap, RequestError> {
    let (response, _) = udp_request(
        endpoint.address,
        endpoint.query_port.get(),
        GAMESPY_STATUS_REQUEST,
        deadline,
    )
    .await?;
    parse_gamespy_status(&response).map_err(RequestError::from)
}

/// One request/reply exchange, with the time the reply took to come back.
///
/// The clock starts after the socket is bound and connected, so the measurement covers the wire
/// and the server's own turnaround, not this process's setup cost.
async fn udp_request(
    address: Ipv4Addr,
    port: u16,
    request: &[u8],
    deadline: Duration,
) -> Result<(Vec<u8>, RoundTripMillis), RequestError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(RequestError::Network)?;
    socket
        .connect(SocketAddr::V4(SocketAddrV4::new(address, port)))
        .await
        .map_err(RequestError::Network)?;
    let sent_at = Instant::now();
    timeout(deadline, socket.send(request))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;
    let mut response = vec![0_u8; MAX_UDP_PACKET];
    let length = timeout(deadline, socket.recv(&mut response))
        .await
        .map_err(|_| RequestError::Timeout)?
        .map_err(RequestError::Network)?;
    let round_trip = round_trip_millis(sent_at.elapsed());
    response.truncate(length);
    Ok((response, round_trip))
}

/// Saturate rather than wrap. `probe_timeout` keeps real values in the low thousands, so the
/// clamp only ever fires if a clock jumps.
fn round_trip_millis(elapsed: Duration) -> RoundTripMillis {
    RoundTripMillis::new(u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX))
}

fn build_server(
    endpoint: MasterEndpoint,
    game_port: GamePort,
    gamespy: &FieldMap,
    status: &FieldMap,
    status_round_trip: RoundTripMillis,
) -> Server {
    let version = nonempty(status, "version");
    // sv_gamespy.c:164 publishes SV_NumClients() as numplayers.
    let clients_reported = parse_u32(gamespy, "numplayers").map(ClientsReported::new);
    // sv_gamespy.c:189 publishes numBots + SV_NumClients() as minplayers. This meaning is
    // OpenMoHAA-specific; retail GameSpy's minplayers field must not be interpreted as bots.
    let bots_reported = version
        .as_deref()
        .filter(|version| is_openmohaa_version(version))
        .and_then(|_| parse_u32(gamespy, "minplayers"))
        .and_then(|minimum| {
            clients_reported.map(|clients| BotsReported::new(minimum.saturating_sub(clients.get())))
        });
    Server {
        endpoint,
        game_port,
        hostname: nonempty(status, "sv_hostname")
            .or_else(|| nonempty(gamespy, "hostname"))
            .unwrap_or_default(),
        game_name: nonempty(gamespy, "gamename"),
        game_version: nonempty(gamespy, "gamever"),
        version,
        protocol: nonempty(status, "protocol"),
        current_map: nonempty(status, "mapname").or_else(|| nonempty(gamespy, "mapname")),
        // gamecvars.cpp:499 registers g_gametypestring as CVAR_SERVERINFO, so sv_main.c:524
        // carries it verbatim in the getstatus reply; sv_gamespy.c:163 publishes the same string
        // as the GameSpy `gametype` field. The numeric g_gametype is a different quantity and is
        // never substituted for it.
        game_type: nonempty(status, "g_gametypestring").or_else(|| nonempty(gamespy, "gametype")),
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
        occupancy: ReportedOccupancy::new(clients_reported, bots_reported),
        client_capacity: parse_u32(gamespy, "maxplayers")
            .or_else(|| parse_u32(status, "sv_maxclients"))
            .map(ClientCapacity::new),
        pure: nonempty(status, "pure"),
        status_round_trip,
    }
}

fn is_openmohaa_version(version: &str) -> bool {
    let version = version.to_ascii_lowercase();
    // OpenMoHAA builds use either the project name or the OPM marker in serverinfo.
    version.contains("openmohaa") || version.contains("(opm)")
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

    use super::{
        build_server, game_port_from_gamespy, record_duplicate_game_endpoints, round_trip_millis,
    };
    use crate::discovery::{
        BrowseReport, FieldMap, GamePort, MasterEndpoint, NonResultReason, ProbeOutcome,
        ProbeStage, QueryPort, RoundTripMillis, TargetGame,
    };

    /// Stand-in for a measurement the tests do not make; no default exists in production.
    const MEASURED: RoundTripMillis = RoundTripMillis::new(41);

    fn complete_outcome(query_port: u16, game_port: u16) -> ProbeOutcome {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 7),
            query_port: QueryPort::new(query_port),
        };
        let gamespy = FieldMap::from([
            ("hostport".to_owned(), game_port.to_string()),
            ("numplayers".to_owned(), "3".to_owned()),
        ]);
        let server = build_server(
            endpoint,
            GamePort::new(game_port),
            &gamespy,
            &FieldMap::new(),
            MEASURED,
        );
        ProbeOutcome {
            endpoint,
            gamespy_reachable: true,
            server: Some(server),
            non_result: None,
        }
    }

    #[test]
    fn records_later_master_registration_for_one_game_endpoint_as_duplicate() {
        let mut outcomes = vec![
            complete_outcome(12_301, 12_203),
            complete_outcome(12_300, 12_203),
        ];

        record_duplicate_game_endpoints(&mut outcomes);

        let report = BrowseReport {
            target: TargetGame::AlliedAssault,
            registered: 2,
            outcomes,
        };
        let summary = report.summary();

        assert_eq!(summary.registered, 2);
        assert_eq!(summary.inspected, 2);
        assert_eq!(summary.getstatus_reachable, 1);
        assert_eq!(summary.clients_reported, 3);
        assert_eq!(summary.non_results, 1);
        assert_eq!(
            report.outcomes[0].endpoint.query_port,
            QueryPort::new(12_300)
        );
        assert!(report.outcomes[0].server.is_some());
        assert!(report.outcomes[0].non_result.is_none());
        assert!(report.outcomes[1].server.is_none());
        assert_eq!(
            report.outcomes[1]
                .non_result
                .as_ref()
                .map(|result| (&result.stage, &result.reason)),
            Some((
                &ProbeStage::EndpointDeduplication,
                &NonResultReason::DuplicateEndpoint {
                    game_port: GamePort::new(12_203),
                },
            ))
        );
    }

    #[test]
    fn preserves_distinct_game_ports_on_one_address() {
        let mut outcomes = vec![
            complete_outcome(12_300, 12_203),
            complete_outcome(12_301, 12_204),
        ];

        record_duplicate_game_endpoints(&mut outcomes);

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| outcome.server.is_some())
                .count(),
            2
        );
        assert!(outcomes.iter().all(|outcome| outcome.non_result.is_none()));
    }

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
            MEASURED,
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
            (
                "version".to_owned(),
                "Medal of Honor Allied Assault 1.12+0.83.0 (OPM)".to_owned(),
            ),
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
            ("g_gametypestring".to_owned(), "Objective-Match".to_owned()),
        ]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
            MEASURED,
        );
        assert_eq!(
            server
                .occupancy
                .clients_reported
                .map(crate::discovery::ClientsReported::get),
            Some(3)
        );
        assert_eq!(
            server
                .occupancy
                .bots_reported
                .map(crate::discovery::BotsReported::get),
            Some(2)
        );
        assert_eq!(server.occupancy.total_occupancy(), Some(5));
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
        assert_eq!(server.game_type.as_deref(), Some("Objective-Match"));
    }

    #[test]
    fn falls_back_to_the_gamespy_gametype_when_serverinfo_omits_it() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 11),
            query_port: QueryPort::new(12_300),
        };
        // sv_gamespy.c:163 publishes g_gametypestring under the shorter `gametype` key.
        let gamespy = FieldMap::from([
            ("hostport".to_owned(), "12203".to_owned()),
            ("gametype".to_owned(), "Tug-of-War".to_owned()),
        ]);
        let status = FieldMap::from([("protocol".to_owned(), "8".to_owned())]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
            MEASURED,
        );

        assert_eq!(server.game_type.as_deref(), Some("Tug-of-War"));
    }

    #[test]
    fn reports_no_game_type_when_neither_reply_publishes_one() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 12),
            query_port: QueryPort::new(12_300),
        };
        let gamespy = FieldMap::from([("hostport".to_owned(), "12203".to_owned())]);
        // sv_main.c:622 also publishes the numeric g_gametype, which is not this quantity.
        let status = FieldMap::from([("g_gametype".to_owned(), "4".to_owned())]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
            MEASURED,
        );

        assert_eq!(server.game_type, None);
    }

    #[test]
    fn does_not_infer_bots_from_retail_minplayers() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 9),
            query_port: QueryPort::new(12_300),
        };
        let gamespy = FieldMap::from([
            ("hostport".to_owned(), "12203".to_owned()),
            ("numplayers".to_owned(), "3".to_owned()),
            ("minplayers".to_owned(), "11".to_owned()),
        ]);
        let status = FieldMap::from([("version".to_owned(), "Medal of Honor 1.11".to_owned())]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
            MEASURED,
        );

        assert_eq!(server.occupancy.bots_reported, None);
        assert_eq!(server.occupancy.total_occupancy(), None);
    }

    #[test]
    fn keeps_the_measured_round_trip_apart_from_the_servers_own_ping_gate() {
        let endpoint = MasterEndpoint {
            address: Ipv4Addr::new(203, 0, 113, 10),
            query_port: QueryPort::new(12_300),
        };
        let gamespy = FieldMap::from([("hostport".to_owned(), "12203".to_owned())]);
        // A server whose gate happens to be a round number the round trip is not.
        let status = FieldMap::from([
            ("sv_minPing".to_owned(), "0".to_owned()),
            ("sv_maxPing".to_owned(), "200".to_owned()),
        ]);

        let server = build_server(
            endpoint,
            game_port_from_gamespy(&gamespy).expect("reply carries hostport"),
            &gamespy,
            &status,
            RoundTripMillis::new(137),
        );

        assert_eq!(server.status_round_trip, RoundTripMillis::new(137));
        assert_eq!(
            server.maximum_ping.map(crate::discovery::PingMillis::get),
            Some(200)
        );
    }

    #[test]
    fn saturates_an_implausible_round_trip_instead_of_wrapping() {
        assert_eq!(
            round_trip_millis(std::time::Duration::from_millis(1_500)),
            RoundTripMillis::new(1_500)
        );
        assert_eq!(
            round_trip_millis(std::time::Duration::from_secs(60 * 60 * 24 * 100)),
            RoundTripMillis::new(u32::MAX)
        );
    }
}
