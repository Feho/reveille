// SPDX-License-Identifier: GPL-2.0-only

//! Typed discovery results. Names deliberately preserve what servers actually report.

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;

use serde::Serialize;

use crate::bsp::Checksum;

macro_rules! numeric_newtype {
    ($(#[$meta:meta])* $name:ident($inner:ty);) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// Construct the value from its wire representation.
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// Return the underlying wire value.
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

numeric_newtype!(
    /// UDP query port returned by the `GameSpy` master.
    QueryPort(u16);
);
numeric_newtype!(
    /// Authoritative game port read from the `GameSpy` `hostport` field.
    GamePort(u16);
);
numeric_newtype!(
    /// Every non-free client slot reported by `SV_NumClients()`.
    ClientsReported(u32);
);
numeric_newtype!(
    /// Simulated clients inferred from `OpenMoHAA`'s `GameSpy` `minplayers` field.
    SimulatedClientsReported(u32);
);
numeric_newtype!(
    /// Public client capacity reported by the server.
    ClientCapacity(u32);
);
numeric_newtype!(
    /// Bitmask published as `sv_allowDownload`.
    DownloadFlags(u32);
);
numeric_newtype!(
    /// Server-side ping gate in milliseconds.
    PingMillis(u32);
);
numeric_newtype!(
    /// Join window after a round starts, in seconds.
    JoinWindowSeconds(u32);
);
numeric_newtype!(
    /// Slots reserved behind the private password.
    ReservedSlots(u32);
);

/// A game family registered with the master server.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetGame {
    /// Medal of Honor: Allied Assault.
    AlliedAssault,
    /// Spearhead.
    Spearhead,
    /// Breakthrough.
    Breakthrough,
}

impl TargetGame {
    /// `GameSpy` registration name.
    #[must_use]
    pub const fn game_name(self) -> &'static str {
        // sv_gamespy.c:50, index-aligned with SECRET_GS_KEYS.
        match self {
            Self::AlliedAssault => "mohaa",
            Self::Spearhead => "mohaas",
            Self::Breakthrough => "mohaab",
        }
    }

    /// Human-readable product label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AlliedAssault => "Allied Assault",
            Self::Spearhead => "Spearhead",
            Self::Breakthrough => "Breakthrough",
        }
    }

    pub(crate) const fn secret_key(self) -> &'static [u8] {
        // sv_gamespy.c:42, index-aligned with GS_GAME_NAME.
        match self {
            Self::AlliedAssault => b"M5Fdwc",
            Self::Spearhead => b"h2P1c9",
            Self::Breakthrough => b"y32FDc",
        }
    }
}

impl fmt::Display for TargetGame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// An address registered by the `GameSpy` master.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MasterEndpoint {
    /// IPv4 address returned by the master.
    pub address: Ipv4Addr,
    /// UDP query port returned by the master, not the game port.
    pub query_port: QueryPort,
}

impl fmt::Display for MasterEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.address, self.query_port)
    }
}

/// A fully inspected server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Server {
    /// Master-list address and query port.
    pub endpoint: MasterEndpoint,
    /// Game port read from `hostport` in the `GameSpy` reply.
    pub game_port: GamePort,
    /// Server display name.
    pub hostname: String,
    /// `GameSpy` game family, when published.
    pub game_name: Option<String>,
    /// `GameSpy` engine/patch label.
    pub game_version: Option<String>,
    /// Full engine version from MOHAA serverinfo.
    pub version: Option<String>,
    /// Exact protocol string from MOHAA serverinfo.
    pub protocol: Option<String>,
    /// Current map spelling published by the server.
    pub current_map: Option<String>,
    /// Full `sv_maplist`, preserving server spelling.
    pub rotation: Vec<String>,
    /// Download permission bitmask, not a boolean.
    pub allow_download: Option<DownloadFlags>,
    /// Current-map checksum, when exposed.
    pub map_checksum: Option<Checksum>,
    /// `PakRadar` manifest location, when exposed.
    pub pr_downloads: Option<String>,
    /// Minimum accepted ping.
    pub minimum_ping: Option<PingMillis>,
    /// Maximum accepted ping.
    pub maximum_ping: Option<PingMillis>,
    /// Join window after round start.
    pub join_window: Option<JoinWindowSeconds>,
    /// Password-reserved slots.
    pub reserved_slots: Option<ReservedSlots>,
    /// Every non-free client slot, never labelled players or humans.
    pub clients_reported: Option<ClientsReported>,
    /// Simulated clients inferred when `OpenMoHAA` publishes `minplayers`.
    pub simulated_clients_reported: Option<SimulatedClientsReported>,
    /// Public capacity reported by GameSpy/serverinfo.
    pub client_capacity: Option<ClientCapacity>,
    /// Raw `pure` value, when exposed.
    pub pure: Option<String>,
}

/// Network/protocol stage at which one server stopped producing useful data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStage {
    /// UDP `GameSpy` `\\status\\` on the master-provided query port.
    GameSpyStatus,
    /// Validation of the authoritative `hostport` field.
    HostPort,
    /// MOHAA out-of-band `getstatus` on the authoritative game port.
    GetStatus,
}

/// Why one registered server did not yield a complete model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum NonResultReason {
    /// The server did not answer before its per-stage deadline.
    Timeout,
    /// The socket operation failed.
    Network {
        /// Operating-system detail.
        message: String,
    },
    /// The reply was present but malformed.
    Malformed {
        /// Parser detail.
        message: String,
    },
    /// `GameSpy` answered without a usable `hostport`.
    MissingHostPort,
}

/// Recorded partial-result reason for a registered server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NonResult {
    /// Stage that could not complete.
    pub stage: ProbeStage,
    /// Structured reason; this never aborts the surrounding sweep.
    pub reason: NonResultReason,
}

/// Per-master-entry result, complete or partial.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProbeOutcome {
    /// Registered address being inspected.
    pub endpoint: MasterEndpoint,
    /// Whether a parseable `GameSpy` status reply was received.
    pub gamespy_reachable: bool,
    /// Complete server model when out-of-band status also succeeded.
    pub server: Option<Server>,
    /// Recorded reason when inspection stopped early.
    pub non_result: Option<NonResult>,
}

/// Complete master sweep with partial results retained.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowseReport {
    /// Requested game family.
    pub target: TargetGame,
    /// Total endpoints registered by the master before any CLI limit.
    pub registered: usize,
    /// Inspected outcomes in address/query-port order.
    pub outcomes: Vec<ProbeOutcome>,
}

impl BrowseReport {
    /// Derive stable acceptance and display counts.
    #[must_use]
    pub fn summary(&self) -> BrowseSummary {
        let servers = self
            .outcomes
            .iter()
            .filter_map(|outcome| outcome.server.as_ref())
            .collect::<Vec<_>>();
        let mut protocols = BTreeMap::new();
        for protocol in servers.iter().filter_map(|server| server.protocol.as_ref()) {
            *protocols.entry(protocol.clone()).or_insert(0) += 1;
        }
        BrowseSummary {
            registered: self.registered,
            inspected: self.outcomes.len(),
            gamespy_reachable: self
                .outcomes
                .iter()
                .filter(|outcome| outcome.gamespy_reachable)
                .count(),
            getstatus_reachable: servers.len(),
            clients_reported: servers
                .iter()
                .filter_map(|server| server.clients_reported)
                .map(ClientsReported::get)
                .sum(),
            rotations_published: servers
                .iter()
                .filter(|server| !server.rotation.is_empty())
                .count(),
            map_checksums_published: servers
                .iter()
                .filter(|server| server.map_checksum.is_some())
                .count(),
            pakradar_published: servers
                .iter()
                .filter(|server| server.pr_downloads.is_some())
                .count(),
            pure_published: servers
                .iter()
                .filter(|server| server.pure.is_some())
                .count(),
            protocols,
        }
    }
}

/// Counts derived from a browse report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowseSummary {
    /// Master registrations before a CLI limit.
    pub registered: usize,
    /// Endpoints actually inspected.
    pub inspected: usize,
    /// Parseable `GameSpy` status replies.
    pub gamespy_reachable: usize,
    /// Parseable MOHAA getstatus replies.
    pub getstatus_reachable: usize,
    /// Sum of non-free slots reported by complete servers.
    pub clients_reported: u32,
    /// Servers publishing a non-empty `sv_maplist`.
    pub rotations_published: usize,
    /// Servers publishing `sv_mapChecksum`.
    pub map_checksums_published: usize,
    /// Servers publishing `pr_downloads`.
    pub pakradar_published: usize,
    /// Servers publishing `pure`.
    pub pure_published: usize,
    /// Complete-server counts by protocol string.
    pub protocols: BTreeMap<String, usize>,
}
