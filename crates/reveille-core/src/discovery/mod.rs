// SPDX-License-Identifier: GPL-2.0-only

//! Public-server discovery over the `GameSpy` and MOHAA protocols.

mod client;
mod model;
mod protocol;

pub use client::{
    BrowseConfig, BrowseEvent, DiscoveryError, RequestError, browse, browse_streaming,
    query_getinfo, query_getstatus,
};
pub use model::{
    BotsReported, BrowseReport, BrowseSummary, ClientCapacity, ClientsReported, DownloadFlags,
    GamePort, JoinWindowSeconds, MasterEndpoint, NonResult, NonResultReason, PingMillis,
    ProbeOutcome, ProbeStage, QueryPort, ReportedOccupancy, ReservedSlots, Server, TargetGame,
};
pub use protocol::{
    CryptoError, FieldMap, ParseError, build_master_query, gs_encode, gs_encrypt,
    parse_gamespy_status, parse_master_challenge, parse_master_response, parse_oob_getinfo,
    parse_oob_getstatus,
};
