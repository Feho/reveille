// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly opt-in smoke check; default and CI test runs never open a socket.

use reveille_core::discovery::{BrowseConfig, BrowseEvent, browse, browse_streaming};
use tokio::sync::mpsc;

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

#[tokio::test]
#[ignore = "requires the live third-party GameSpy master and public UDP servers"]
async fn streaming_browse_reports_every_outcome_and_agrees_with_the_report() {
    let config = BrowseConfig {
        limit: Some(10),
        ..BrowseConfig::default()
    };
    let (sink, mut events) = mpsc::channel(64);
    let sweep = tokio::spawn(browse_streaming(config, sink));

    let mut registered = None;
    let mut streamed = 0_usize;
    while let Some(event) = events.recv().await {
        match event {
            BrowseEvent::Registered { inspected, .. } => registered = Some(inspected),
            BrowseEvent::Outcome(_) => streamed += 1,
        }
    }

    let Ok(Ok(report)) = sweep.await else {
        return;
    };
    assert_eq!(registered, Some(report.outcomes.len()));
    assert_eq!(streamed, report.outcomes.len());
}

#[tokio::test]
#[ignore = "requires the live third-party GameSpy master and public UDP servers"]
async fn dropping_the_receiver_cancels_the_sweep() {
    let config = BrowseConfig {
        limit: Some(200),
        ..BrowseConfig::default()
    };
    let (sink, mut events) = mpsc::channel(1);
    let sweep = tokio::spawn(browse_streaming(config, sink));

    // Take the header and one outcome, then stop listening. The sweep must finish promptly with
    // far fewer outcomes than the 200 it was asked for, rather than running to completion.
    let _ = events.recv().await;
    let _ = events.recv().await;
    drop(events);

    let Ok(Ok(report)) = sweep.await else {
        return;
    };
    assert!(report.outcomes.len() < 200);
}
