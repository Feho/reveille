// SPDX-License-Identifier: GPL-2.0-only

use std::net::Ipv4Addr;

use reveille_core::discovery::{
    GamePort, ParseError, parse_gamespy_status, parse_master_response, parse_oob_getinfo,
    parse_oob_getstatus,
};

#[test]
fn parses_the_frozen_master_response_body() {
    let response = decode_hex(include_str!("fixtures/master_response.hex"));
    let endpoints = parse_master_response(&response).expect("captured master response");

    assert_eq!(endpoints.len(), 177);
    let tfc = endpoints
        .iter()
        .find(|endpoint| endpoint.address == Ipv4Addr::new(173, 249, 214, 104))
        .expect("captured TFC registration");
    assert_eq!(tfc.query_port.get(), 12_300);
}

#[test]
fn reads_hostport_from_the_frozen_gamespy_reply() {
    let response = decode_hex(include_str!("fixtures/gamespy_status.hex"));
    let fields = parse_gamespy_status(&response).expect("captured GameSpy status reply");
    let game_port = GamePort::new(fields["hostport"].parse().expect("numeric hostport"));

    assert_eq!(game_port.get(), 12_203);
    assert_ne!(game_port.get(), 12_300, "query port is not the game port");
    assert_eq!(fields["numplayers"], "1");
}

#[test]
fn parses_the_five_byte_oob_getstatus_capture() {
    let response = decode_hex(include_str!("fixtures/oob_getstatus.hex"));
    let fields = parse_oob_getstatus(&response).expect("captured MOHAA getstatus reply");

    assert_eq!(fields["protocol"], "8");
    assert_eq!(fields["sv_allowDownload"], "0");
    assert_eq!(fields["sv_privateClients"], "4");
    assert_eq!(fields["g_allowjointime"], "10");
    assert_eq!(fields["sv_maplist"].split_whitespace().count(), 14);
    assert!(!fields.contains_key("sv_mapChecksum"));
    assert!(!fields.contains_key("pure"));
}

#[test]
fn rejects_the_plain_quake_three_header() {
    let mut response = vec![0xff; 4];
    response.extend_from_slice(b"statusResponse\n\\protocol\\8");
    assert_eq!(
        parse_oob_getstatus(&response),
        Err(ParseError::InvalidOobHeader)
    );
}

#[test]
fn parses_a_five_byte_oob_getinfo_reply() {
    let mut response = vec![0xff, 0xff, 0xff, 0xff, 0x01];
    response.extend_from_slice(b"infoResponse\n\\protocol\\8\\challenge\\frozen");
    let fields = parse_oob_getinfo(&response).expect("MOHAA getinfo response");
    assert_eq!(fields["protocol"], "8");
    assert_eq!(fields["challenge"], "frozen");
}

fn decode_hex(encoded: &str) -> Vec<u8> {
    let digits = encoded
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(digits.len() % 2, 0, "hex fixture has complete bytes");
    digits
        .chunks_exact(2)
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

fn nibble(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => panic!("invalid hex digit in frozen fixture"),
    }
}
