// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly opt-in smoke check; default and CI test runs never open a socket.

use reveille_core::discovery::{BrowseConfig, browse};

#[tokio::test]
#[ignore = "requires the live third-party GameSpy master and public UDP servers"]
async fn live_discovery_smoke_check_skips_when_unavailable() {
    let config = BrowseConfig {
        limit: Some(10),
        ..BrowseConfig::default()
    };
    let Ok(report) = browse(config).await else {
        return;
    };
    assert!(report.registered > 0);
    assert_eq!(report.outcomes.len(), 10.min(report.registered));
}
