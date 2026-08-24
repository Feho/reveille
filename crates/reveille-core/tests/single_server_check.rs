// SPDX-License-Identifier: GPL-2.0-only

//! `inspect_endpoint` against a loopback stand-in, so the probe behind a single-server check is
//! exercised end to end without a third party (rules.md S4).
//!
//! The replies are the frozen wire captures. Only the `hostport` field is rewritten, because it
//! is the one value that must name a socket this test actually owns; the capture's own bytes are
//! asserted verbatim in `wire_captures.rs`.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use reveille_core::discovery::{MasterEndpoint, NonResultReason, ProbeStage, QueryPort};
use tokio::net::UdpSocket;

const DEADLINE: Duration = Duration::from_millis(1_500);

#[tokio::test]
async fn a_direct_check_builds_the_same_server_a_sweep_would() {
    let game = bind().await;
    let game_port = port(&game);
    let query = bind().await;
    let query_port = port(&query);

    let gamespy = with_hostport(
        &decode_hex(include_str!("fixtures/gamespy_status.hex")),
        game_port,
    );
    let status = decode_hex(include_str!("fixtures/oob_getstatus.hex"));
    let responders = tokio::spawn(async move {
        reply_once(&query, &gamespy).await;
        reply_once(&game, &status).await;
    });

    let outcome = reveille_core::discovery::inspect_endpoint(
        MasterEndpoint {
            address: Ipv4Addr::LOCALHOST,
            query_port: QueryPort::new(query_port),
        },
        DEADLINE,
    )
    .await;
    responders.await.expect("responders");

    assert!(outcome.gamespy_reachable);
    assert!(outcome.non_result.is_none());
    let server = outcome
        .server
        .expect("a server that answered both requests");
    // The authoritative game port comes from the reply, not from the query port that was asked.
    assert_eq!(server.game_port.get(), game_port);
    assert_ne!(server.game_port.get(), query_port);
    assert_eq!(server.endpoint.query_port.get(), query_port);
    // Both replies contributed: the rotation is only in the getstatus body, the client count only
    // in the GameSpy one.
    assert_eq!(server.rotation.len(), 14);
    assert_eq!(
        server
            .occupancy
            .clients_reported
            .map(reveille_core::discovery::ClientsReported::get),
        Some(1)
    );
    assert_eq!(server.occupancy.bots_reported, None);
}

#[tokio::test]
async fn a_server_that_never_answers_is_a_recorded_non_result() {
    // Bound but silent: the reason has to be timeout at the GameSpy stage, not an error thrown
    // out of the probe. A favourite that does not answer is information about that favourite.
    let query = bind().await;
    let query_port = port(&query);

    let outcome = reveille_core::discovery::inspect_endpoint(
        MasterEndpoint {
            address: Ipv4Addr::LOCALHOST,
            query_port: QueryPort::new(query_port),
        },
        Duration::from_millis(150),
    )
    .await;

    assert!(outcome.server.is_none());
    assert!(!outcome.gamespy_reachable);
    let non_result = outcome.non_result.expect("a recorded reason");
    assert_eq!(non_result.stage, ProbeStage::GameSpyStatus);
    assert_eq!(non_result.reason, NonResultReason::Timeout);
}

#[tokio::test]
async fn a_reply_without_a_usable_game_port_is_recorded_rather_than_guessed() {
    let query = bind().await;
    let query_port = port(&query);
    let gamespy = with_hostport(&decode_hex(include_str!("fixtures/gamespy_status.hex")), 0);
    let responder = tokio::spawn(async move { reply_once(&query, &gamespy).await });

    let outcome = reveille_core::discovery::inspect_endpoint(
        MasterEndpoint {
            address: Ipv4Addr::LOCALHOST,
            query_port: QueryPort::new(query_port),
        },
        DEADLINE,
    )
    .await;
    responder.await.expect("responder");

    assert!(outcome.server.is_none());
    assert!(outcome.gamespy_reachable);
    let non_result = outcome.non_result.expect("a recorded reason");
    assert_eq!(non_result.stage, ProbeStage::HostPort);
    assert!(matches!(
        non_result.reason,
        NonResultReason::Malformed { .. }
    ));
}

async fn bind() -> UdpSocket {
    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("loopback socket")
}

fn port(socket: &UdpSocket) -> u16 {
    socket.local_addr().expect("bound address").port()
}

/// Wait for one request and answer it, so the probe measures a real round trip.
async fn reply_once(socket: &UdpSocket, response: &[u8]) {
    let mut request = vec![0_u8; 2048];
    let (_, from) = socket.recv_from(&mut request).await.expect("a request");
    socket.send_to(response, from).await.expect("a reply");
}

/// Point the capture's `hostport` at a socket this test owns.
fn with_hostport(capture: &[u8], port: u16) -> Vec<u8> {
    let text = String::from_utf8(capture.to_vec()).expect("the GameSpy capture is text");
    let (before, rest) = text
        .split_once("\\hostport\\")
        .expect("the capture publishes a hostport");
    let (_, after) = rest.split_once('\\').expect("a terminated hostport value");
    format!("{before}\\hostport\\{port}\\{after}").into_bytes()
}

fn decode_hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .flat_map(|line| line.as_bytes().chunks(2))
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex text");
            u8::from_str_radix(pair, 16).expect("hex byte")
        })
        .collect()
}
