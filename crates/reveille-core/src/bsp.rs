// SPDX-License-Identifier: GPL-2.0-only

//! Minimal Medal of Honor BSP header parsing.

use std::io::{self, Read};

use thiserror::Error;

/// Checksum copied verbatim from offset eight of a MOHAA BSP header.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct Checksum(i32);

impl Checksum {
    /// Construct a checksum from its signed wire/header representation.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Return the signed value used by `sv_mapChecksum`.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl std::fmt::Display for Checksum {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The fixed portion of a MOHAA BSP header needed to identify a map checksum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Header {
    /// Header marker used by the map.
    pub ident: Ident,
    /// BSP format version, already validated as `17..=21` like `CM_LoadMap`
    /// (`code/qcommon/cm_load.c:894`).
    pub version: i32,
    /// Checksum used by `CM_Checksum` and published as `sv_mapChecksum`.
    pub checksum: Checksum,
}

/// BSP header markers found in MOHAA-family retail content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ident {
    /// Allied Assault's `2015` marker (`qfiles.h:358`).
    AlliedAssault,
    /// `EALA`, used by EA expansion maps converted into AA server packages.
    EaExpansion,
    /// Any other marker. The runtime engine treats the identifier as informational.
    Unknown([u8; 4]),
}

/// An error encountered while reading a BSP header.
#[derive(Debug, Error)]
pub enum Error {
    /// Fewer than twelve bytes were available.
    #[error("could not read the 12-byte BSP header")]
    Read(#[source] io::Error),
    /// The engine rejects BSP versions outside its supported range.
    #[error("unsupported MOHAA BSP version {version}; expected {MIN_VERSION}..={MAX_VERSION}")]
    UnsupportedVersion {
        /// Version read from the header.
        version: i32,
    },
}

// qfiles.h:361-364; enforced by CM_LoadMap in cm_load.c:894.
/// Oldest BSP version accepted by the runtime engine.
pub const MIN_VERSION: i32 = 17;
/// Newest BSP version accepted by the runtime engine.
pub const MAX_VERSION: i32 = 21;

/// Read the twelve-byte header without consuming or inflating the rest of the BSP.
///
/// # Errors
///
/// Returns [`Error::Read`] for a short or unreadable header, or [`Error::UnsupportedVersion`]
/// when the runtime engine would reject the map. The four-byte identifier is informational.
pub fn read_header(mut reader: impl Read) -> Result<Header, Error> {
    let mut bytes = [0_u8; 12];
    reader.read_exact(&mut bytes).map_err(Error::Read)?;

    let ident_bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
    let ident = match ident_bytes {
        value if value == *b"2015" => Ident::AlliedAssault,
        value if value == *b"EALA" => Ident::EaExpansion,
        other => Ident::Unknown(other),
    };
    let version = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if !(MIN_VERSION..=MAX_VERSION).contains(&version) {
        return Err(Error::UnsupportedVersion { version });
    }

    Ok(Header {
        ident,
        version,
        checksum: Checksum::new(i32::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
        ])),
    })
}

#[cfg(test)]
mod tests {
    use super::{Checksum, Error, Header, Ident, read_header};

    #[test]
    fn reads_version_and_checksum_as_little_endian() {
        let mut bytes = *b"2015\x13\0\0\0\0\0\0\0";
        bytes[8..12].copy_from_slice(&1_974_169_620_i32.to_le_bytes());

        assert_eq!(
            read_header(bytes.as_slice()).expect("valid header"),
            Header {
                ident: Ident::AlliedAssault,
                version: 19,
                checksum: Checksum::new(1_974_169_620),
            }
        );
    }

    #[test]
    fn accepts_the_expansion_marker_found_in_retail_map_packs() {
        let mut bytes = *b"EALA\x13\0\0\0\0\0\0\0";
        bytes[8..12].copy_from_slice(&(-1_071_346_343_i32).to_le_bytes());

        assert_eq!(
            read_header(bytes.as_slice()).expect("expansion header"),
            Header {
                ident: Ident::EaExpansion,
                version: 19,
                checksum: Checksum::new(-1_071_346_343),
            }
        );
    }

    #[test]
    fn preserves_unknown_idents_without_rejecting_the_map() {
        let header = read_header(b"IBSP\x13\0\0\0\0\0\0\0".as_slice()).expect("valid version");
        assert_eq!(header.ident, Ident::Unknown(*b"IBSP"));
    }

    #[test]
    fn rejects_versions_outside_the_engine_range() {
        for version in [16_i32, 22] {
            let mut bytes = *b"2015\0\0\0\0\0\0\0\0";
            bytes[4..8].copy_from_slice(&version.to_le_bytes());
            let error = read_header(bytes.as_slice()).expect_err("unsupported version");
            assert!(matches!(
                error,
                Error::UnsupportedVersion { version: actual } if actual == version
            ));
        }
    }

    #[test]
    fn rejects_short_headers() {
        let error = read_header(b"2015".as_slice()).expect_err("short header");
        assert!(matches!(error, Error::Read(_)));
    }
}
