// SPDX-License-Identifier: GPL-2.0-only

//! `PakRadar` `filelist.txt` parsing and MD5-backed downloads.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use md5::{Digest, Md5};
use reqwest::Client;
use serde::Serialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::fs;

use super::archive::{DownloadedArchive, PakRadarIntegrity, validate_package_filename};

// PakRadar manifests are third-party HTTP resources; identify the launcher explicitly.
const USER_AGENT: &str = "Reveille/0.1 (MOHAA content resolver)";

/// A strict 128-bit MD5 digest published by `PakRadar`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Md5Digest([u8; 16]);

impl Md5Digest {
    /// Parse exactly 32 hexadecimal digits.
    ///
    /// # Errors
    ///
    /// Returns an error for any other length or non-hexadecimal byte.
    pub fn parse(value: &str) -> Result<Self, PakRadarError> {
        let value = value.trim();
        if value.len() != 32 {
            return Err(PakRadarError::InvalidMd5(value.to_owned()));
        }
        let mut digest = [0_u8; 16];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_digit(pair[0]).ok_or_else(|| PakRadarError::InvalidMd5(value.into()))?;
            let low = hex_digit(pair[1]).ok_or_else(|| PakRadarError::InvalidMd5(value.into()))?;
            digest[index] = (high << 4) | low;
        }
        Ok(Self(digest))
    }

    /// Return the raw digest bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Display for Md5Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// One server-published `PakRadar` package.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PakRadarEntry {
    /// Human-facing package alias.
    pub alias: String,
    /// Server-published archive MD5.
    pub md5: Md5Digest,
    /// Direct package URL.
    pub url: String,
}

/// Parse `PakRadar`'s repeated `map { alias/md5/url }` blocks.
///
/// # Errors
///
/// Returns an error for malformed blocks, missing required fields, or invalid MD5 text.
pub fn parse_filelist(input: &str) -> Result<Vec<PakRadarEntry>, PakRadarError> {
    let mut entries = Vec::new();
    let mut current: Option<Block> = None;
    for (line_index, raw_line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if line == "map {" {
            if current.is_some() {
                return Err(PakRadarError::Syntax { line: line_number });
            }
            current = Some(Block::default());
            continue;
        }
        if line == "}" {
            let block = current
                .take()
                .ok_or(PakRadarError::Syntax { line: line_number })?;
            entries.push(block.finish(line_number)?);
            continue;
        }
        let block = current
            .as_mut()
            .ok_or(PakRadarError::Syntax { line: line_number })?;
        let (key, value) = parse_property(line, line_number)?;
        match key {
            "alias" => block.alias = Some(value.to_owned()),
            "md5" => block.md5 = Some(value.to_owned()),
            "url" => block.url = Some(value.to_owned()),
            _ => {}
        }
    }
    if current.is_some() {
        return Err(PakRadarError::UnclosedBlock);
    }
    Ok(entries)
}

/// Fetch and parse one server-published `PakRadar` manifest.
///
/// # Errors
///
/// Returns an HTTP, body, or manifest grammar error for this source only.
pub async fn fetch_filelist(
    url: &str,
    timeout: Duration,
) -> Result<Vec<PakRadarEntry>, PakRadarError> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(PakRadarError::Network)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(PakRadarError::Network)?;
    if !response.status().is_success() {
        return Err(PakRadarError::HttpStatus(response.status().as_u16()));
    }
    let body = response.text().await.map_err(PakRadarError::Network)?;
    parse_filelist(&body)
}

/// Download a `PakRadar` package and require its server-published MD5 before staging succeeds.
///
/// # Errors
///
/// Returns an error for an unsafe URL filename, HTTP/I/O failure, or digest mismatch.
pub async fn download_pakradar_archive(
    client: &Client,
    entry: &PakRadarEntry,
    staging_directory: &Path,
) -> Result<DownloadedArchive<PakRadarIntegrity>, PakRadarError> {
    let filename = entry
        .url
        .split(['/', '\\'])
        .next_back()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| PakRadarError::UnsafeUrl(entry.url.clone()))?;
    validate_package_filename(filename).map_err(PakRadarError::Archive)?;
    let response = client
        .get(&entry.url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(PakRadarError::Network)?;
    if !response.status().is_success() {
        return Err(PakRadarError::HttpStatus(response.status().as_u16()));
    }
    let bytes = response.bytes().await.map_err(PakRadarError::Network)?;
    let actual = Md5::digest(&bytes);
    if actual.as_slice() != entry.md5.bytes() {
        return Err(PakRadarError::DigestMismatch {
            expected: entry.md5,
            actual: Md5Digest(actual.into()),
        });
    }
    fs::create_dir_all(staging_directory)
        .await
        .map_err(|source| PakRadarError::Staging {
            path: staging_directory.to_path_buf(),
            source,
        })?;
    let temporary =
        NamedTempFile::new_in(staging_directory).map_err(|source| PakRadarError::Staging {
            path: staging_directory.to_path_buf(),
            source,
        })?;
    fs::write(temporary.path(), &bytes)
        .await
        .map_err(|source| PakRadarError::Staging {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    let target = staging_directory.join(filename);
    temporary
        .persist_noclobber(&target)
        .map_err(|error| PakRadarError::Staging {
            path: target.clone(),
            source: error.error,
        })?;
    Ok(DownloadedArchive {
        path: target,
        filename: filename.to_owned(),
        integrity: PakRadarIntegrity::VerifiedMd5(entry.md5.to_string()),
    })
}

#[derive(Default)]
struct Block {
    alias: Option<String>,
    md5: Option<String>,
    url: Option<String>,
}

impl Block {
    fn finish(self, line: usize) -> Result<PakRadarEntry, PakRadarError> {
        let alias = self.alias.ok_or(PakRadarError::MissingField {
            field: "alias",
            line,
        })?;
        let md5 = Md5Digest::parse(
            &self
                .md5
                .ok_or(PakRadarError::MissingField { field: "md5", line })?,
        )?;
        let url = self
            .url
            .ok_or(PakRadarError::MissingField { field: "url", line })?;
        Ok(PakRadarEntry { alias, md5, url })
    }
}

fn parse_property(line: &str, line_number: usize) -> Result<(&str, &str), PakRadarError> {
    let Some((key, rest)) = line.split_once(char::is_whitespace) else {
        return Err(PakRadarError::Syntax { line: line_number });
    };
    let rest = rest.trim();
    let value = rest
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or(PakRadarError::Syntax { line: line_number })?;
    Ok((key, value))
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `PakRadar` manifest or package error.
#[derive(Debug, Error)]
pub enum PakRadarError {
    /// A line does not follow the block grammar.
    #[error("invalid PakRadar syntax at line {line}")]
    Syntax {
        /// One-based line number.
        line: usize,
    },
    /// The final block did not close.
    #[error("unclosed PakRadar map block")]
    UnclosedBlock,
    /// A required property was absent.
    #[error("missing PakRadar field {field:?} near line {line}")]
    MissingField {
        /// Property name.
        field: &'static str,
        /// Closing line number.
        line: usize,
    },
    /// MD5 text was not exactly 32 hexadecimal digits.
    #[error("invalid PakRadar MD5 {0:?}")]
    InvalidMd5(String),
    /// The URL has no safe final package filename.
    #[error("PakRadar URL has no safe package filename: {0}")]
    UnsafeUrl(String),
    /// Manifest or package request failed.
    #[error("PakRadar HTTP request failed")]
    Network(#[source] reqwest::Error),
    /// Package URL returned an unsuccessful response.
    #[error("PakRadar download returned HTTP {0}")]
    HttpStatus(u16),
    /// Download bytes did not match the server-published MD5.
    #[error("PakRadar MD5 mismatch: expected {expected}, received {actual}")]
    DigestMismatch {
        /// Manifest digest.
        expected: Md5Digest,
        /// Download digest.
        actual: Md5Digest,
    },
    /// Archive filename safety rejected the URL basename.
    #[error(transparent)]
    Archive(#[from] super::archive::ArchiveError),
    /// Staging filesystem operation failed.
    #[error("could not write staged PakRadar archive {path}")]
    Staging {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::parse_filelist;

    #[test]
    fn parses_real_filelist_shape_and_md5_values() {
        let entries = parse_filelist(include_str!("../../tests/fixtures/pakradar_filelist.txt"))
            .expect("valid PakRadar sample");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].alias, "LuV Map Pack 1");
        assert_eq!(
            entries[0].md5.to_string(),
            "1cdd05c74995132c64747650fa3eebb6"
        );
        assert_eq!(entries[1].alias, "LuV Final Map Pack");
    }
}
