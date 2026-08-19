// SPDX-License-Identifier: GPL-2.0-only

//! Digest-gated `OpenMoHAA` GitHub Release selection, download, installation, and update.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Cursor, Write as IoWrite};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;
use zip::ZipArchive;

// GitHub REST Releases API for the official openmoh/openmohaa repository.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openmoh/openmohaa/releases/latest";
const USER_AGENT: &str = "Reveille/0.1 (+https://github.com/openmoh/openmohaa)";
const MAX_RELEASE_BYTES: u64 = 128 * 1024 * 1024;

/// SHA-256 digest published in a GitHub Release asset's `digest` field.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct PublishedSha256([u8; 32]);

impl PublishedSha256 {
    /// Parse GitHub's `sha256:<64 lowercase-or-uppercase hex digits>` representation.
    ///
    /// # Errors
    ///
    /// Returns an error for another algorithm or malformed hexadecimal data.
    pub fn parse(value: &str) -> Result<Self, OpenMohaaError> {
        let hex = value
            .strip_prefix("sha256:")
            .ok_or_else(|| OpenMohaaError::UnsupportedDigest(value.to_owned()))?;
        if hex.len() != 64 {
            return Err(OpenMohaaError::InvalidDigest(value.to_owned()));
        }
        let mut digest = [0_u8; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_nibble(pair[0])
                .ok_or_else(|| OpenMohaaError::InvalidDigest(value.to_owned()))?;
            let low = hex_nibble(pair[1])
                .ok_or_else(|| OpenMohaaError::InvalidDigest(value.to_owned()))?;
            digest[index] = (high << 4) | low;
        }
        Ok(Self(digest))
    }

    #[must_use]
    fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Return lowercase hexadecimal without changing its verified/published meaning.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut rendered = String::with_capacity(64);
        for byte in self.0 {
            let _ = write!(rendered, "{byte:02x}");
        }
        rendered
    }
}

impl fmt::Display for PublishedSha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Portable archive target selected from the release asset list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseTarget {
    WindowsX64,
    WindowsX86,
    WindowsArm64,
}

impl ReleaseTarget {
    const fn asset_suffix(self) -> &'static str {
        match self {
            Self::WindowsX64 => "-windows-x64.zip",
            Self::WindowsX86 => "-windows-x86.zip",
            Self::WindowsArm64 => "-windows-arm64.zip",
        }
    }
}

/// One digest-bearing portable release archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReleasePackage {
    /// Git tag returned by the latest-release endpoint.
    pub version: String,
    /// Exact release asset filename.
    pub asset_name: String,
    /// Browser download URL returned by GitHub.
    pub download_url: String,
    /// Publisher-reported archive size.
    pub size: u64,
    /// Publisher-reported SHA-256, required rather than optional.
    pub digest: PublishedSha256,
}

/// What the platform layer knows about client activity before replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientActivity {
    /// A Windows process check proved that the client is stopped.
    ConfirmedStopped,
    /// The client is currently running.
    Running,
    /// No process check has been performed.
    Unknown,
}

/// Why an otherwise valid update made no filesystem changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateDeferredReason {
    ClientRunning,
    ClientStateUnknown,
}

/// Result of applying a verified archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpdateOutcome {
    /// No release-owned target existed, so files were added without overwriting.
    Installed { files: usize },
    /// Existing release-owned files were atomically replaced after a stopped-client confirmation.
    Updated { files: usize, replaced: usize },
    /// Replacement was required but forbidden before any installation writes occurred.
    Deferred { reason: UpdateDeferredReason },
}

/// Parse a frozen or live GitHub latest-release response and select a portable Windows archive.
///
/// # Errors
///
/// Returns an error for malformed JSON, no matching asset, or a missing/invalid published digest.
pub fn parse_latest_release(
    json: &str,
    target: ReleaseTarget,
) -> Result<ReleasePackage, OpenMohaaError> {
    let release: RawRelease =
        serde_json::from_str(json).map_err(OpenMohaaError::MalformedRelease)?;
    let suffix = target.asset_suffix();
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name.ends_with(suffix))
        .ok_or(OpenMohaaError::MissingAsset(target))?;
    let digest = asset
        .digest
        .ok_or_else(|| OpenMohaaError::MissingDigest(asset.name.clone()))
        .and_then(|value| PublishedSha256::parse(&value))?;
    if asset.size > MAX_RELEASE_BYTES {
        return Err(OpenMohaaError::AssetTooLarge {
            size: asset.size,
            maximum: MAX_RELEASE_BYTES,
        });
    }
    Ok(ReleasePackage {
        version: release.tag_name,
        asset_name: asset.name,
        download_url: asset.browser_download_url,
        size: asset.size,
        digest,
    })
}

/// GitHub client for the official `OpenMoHAA` latest release.
#[derive(Clone, Debug)]
pub struct OpenMohaaReleaseClient {
    client: Client,
}

impl OpenMohaaReleaseClient {
    /// Construct a client with an identifying user agent and finite request deadline.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be configured.
    pub fn new(timeout: Duration) -> Result<Self, OpenMohaaError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()
            .map_err(OpenMohaaError::Client)?;
        Ok(Self { client })
    }

    /// Fetch and select the latest portable archive for a Windows target.
    ///
    /// # Errors
    ///
    /// Returns an error for HTTP, JSON, asset-selection, or digest failures.
    pub async fn latest_release(
        &self,
        target: ReleaseTarget,
    ) -> Result<ReleasePackage, OpenMohaaError> {
        let response = self
            .client
            .get(LATEST_RELEASE_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(OpenMohaaError::Network)?;
        let status = response.status();
        if !status.is_success() {
            return Err(OpenMohaaError::HttpStatus(status.as_u16()));
        }
        let text = response.text().await.map_err(OpenMohaaError::Network)?;
        parse_latest_release(&text, target)
    }

    /// Download, verify, and transactionally overlay one release archive.
    ///
    /// # Errors
    ///
    /// Returns an error before installation for network, size, or digest failures, and retains
    /// existing target files if an individual atomic replacement fails.
    pub async fn download_and_install(
        &self,
        package: &ReleasePackage,
        destination: impl AsRef<Path>,
        activity: ClientActivity,
    ) -> Result<UpdateOutcome, OpenMohaaError> {
        let mut response = self
            .client
            .get(&package.download_url)
            .send()
            .await
            .map_err(OpenMohaaError::Network)?;
        let status = response.status();
        if !status.is_success() {
            return Err(OpenMohaaError::HttpStatus(status.as_u16()));
        }
        if let Some(size) = response
            .content_length()
            .filter(|length| *length > MAX_RELEASE_BYTES)
        {
            return Err(OpenMohaaError::AssetTooLarge {
                size,
                maximum: MAX_RELEASE_BYTES,
            });
        }
        let mut bytes = Vec::with_capacity(usize::try_from(package.size).unwrap_or_default());
        while let Some(chunk) = response.chunk().await.map_err(OpenMohaaError::Network)? {
            let next_size = bytes.len().saturating_add(chunk.len());
            if next_size as u64 > MAX_RELEASE_BYTES {
                return Err(OpenMohaaError::AssetTooLarge {
                    size: next_size as u64,
                    maximum: MAX_RELEASE_BYTES,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        install_verified_archive(package, &bytes, destination, activity)
    }
}

/// Verify and apply already-downloaded release bytes. Useful to keep tests entirely offline.
///
/// # Errors
///
/// Returns an error for size/digest mismatch, unsafe or malformed ZIP content, or filesystem
/// failures. A deferred result is not an error and performs no installation writes.
pub fn install_verified_archive(
    package: &ReleasePackage,
    bytes: &[u8],
    destination: impl AsRef<Path>,
    activity: ClientActivity,
) -> Result<UpdateOutcome, OpenMohaaError> {
    if bytes.len() as u64 != package.size {
        return Err(OpenMohaaError::SizeMismatch {
            expected: package.size,
            actual: bytes.len() as u64,
        });
    }
    let actual = PublishedSha256::from_bytes(bytes);
    if actual != package.digest {
        return Err(OpenMohaaError::DigestMismatch {
            expected: package.digest,
            actual,
        });
    }
    let destination = destination.as_ref();
    let entries = inspect_archive(bytes)?;
    let replacements = entries
        .iter()
        .filter(|entry| destination.join(&entry.path).exists())
        .count();
    if replacements > 0 {
        match activity {
            ClientActivity::Running => {
                return Ok(UpdateOutcome::Deferred {
                    reason: UpdateDeferredReason::ClientRunning,
                });
            }
            ClientActivity::Unknown => {
                return Ok(UpdateOutcome::Deferred {
                    reason: UpdateDeferredReason::ClientStateUnknown,
                });
            }
            ClientActivity::ConfirmedStopped => {}
        }
    }

    let parent = destination
        .parent()
        .ok_or_else(|| OpenMohaaError::NoDestinationParent(destination.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| OpenMohaaError::Filesystem {
        path: parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".reveille-openmohaa-")
        .tempdir_in(parent)
        .map_err(|source| OpenMohaaError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
    extract_archive(bytes, staging.path())?;
    apply_staged_files(&staging, destination, &entries, replacements)
}

#[derive(Clone)]
struct ArchiveEntry {
    path: PathBuf,
}

fn inspect_archive(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, OpenMohaaError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(OpenMohaaError::InvalidZip)?;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(OpenMohaaError::InvalidZip)?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(OpenMohaaError::UnsafeArchiveEntry(entry.name().to_owned()));
        }
        let path = safe_archive_path(entry.name())?;
        let key = path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(OpenMohaaError::DuplicateArchiveEntry(
                entry.name().to_owned(),
            ));
        }
        entries.push(ArchiveEntry { path });
    }
    if entries.is_empty() {
        return Err(OpenMohaaError::EmptyArchive);
    }
    Ok(entries)
}

fn safe_archive_path(value: &str) -> Result<PathBuf, OpenMohaaError> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return Err(OpenMohaaError::UnsafeArchiveEntry(value.to_owned()));
    }
    let path = PathBuf::from(&normalized);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(OpenMohaaError::UnsafeArchiveEntry(value.to_owned()));
    }
    Ok(path)
}

fn extract_archive(bytes: &[u8], staging: &Path) -> Result<(), OpenMohaaError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(OpenMohaaError::InvalidZip)?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(OpenMohaaError::InvalidZip)?;
        if entry.is_dir() {
            continue;
        }
        let relative = safe_archive_path(entry.name())?;
        let target = staging.join(relative);
        let parent = target
            .parent()
            .ok_or_else(|| OpenMohaaError::NoDestinationParent(target.clone()))?;
        fs::create_dir_all(parent).map_err(|source| OpenMohaaError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
        let mut output = File::create(&target).map_err(|source| OpenMohaaError::Filesystem {
            path: target.clone(),
            source,
        })?;
        io::copy(&mut entry, &mut output).map_err(|source| OpenMohaaError::Filesystem {
            path: target,
            source,
        })?;
    }
    Ok(())
}

struct PreparedFile {
    target: PathBuf,
    replacement: Option<NamedTempFile>,
    backup: Option<NamedTempFile>,
}

fn apply_staged_files(
    staging: &TempDir,
    destination: &Path,
    entries: &[ArchiveEntry],
    replacements: usize,
) -> Result<UpdateOutcome, OpenMohaaError> {
    fs::create_dir_all(destination).map_err(|source| OpenMohaaError::Filesystem {
        path: destination.to_path_buf(),
        source,
    })?;
    let mut prepared = Vec::with_capacity(entries.len());
    for entry in entries {
        let source = staging.path().join(&entry.path);
        let target = destination.join(&entry.path);
        let parent = target
            .parent()
            .ok_or_else(|| OpenMohaaError::NoDestinationParent(target.clone()))?;
        fs::create_dir_all(parent).map_err(|source| OpenMohaaError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
        if target.is_dir() {
            return Err(OpenMohaaError::TargetIsDirectory(target));
        }
        let backup = if target.exists() {
            let backup =
                NamedTempFile::new_in(parent).map_err(|source| OpenMohaaError::Filesystem {
                    path: parent.to_path_buf(),
                    source,
                })?;
            fs::copy(&target, backup.path()).map_err(|source| OpenMohaaError::Filesystem {
                path: target.clone(),
                source,
            })?;
            Some(backup)
        } else {
            None
        };
        let mut replacement =
            NamedTempFile::new_in(parent).map_err(|source| OpenMohaaError::Filesystem {
                path: parent.to_path_buf(),
                source,
            })?;
        let mut input = File::open(&source).map_err(|source_error| OpenMohaaError::Filesystem {
            path: source.clone(),
            source: source_error,
        })?;
        io::copy(&mut input, &mut replacement).map_err(|source_error| {
            OpenMohaaError::Filesystem {
                path: target.clone(),
                source: source_error,
            }
        })?;
        replacement
            .flush()
            .map_err(|source| OpenMohaaError::Filesystem {
                path: target.clone(),
                source,
            })?;
        prepared.push(PreparedFile {
            target,
            replacement: Some(replacement),
            backup,
        });
    }

    for index in 0..prepared.len() {
        let Some(replacement) = prepared[index].replacement.take() else {
            rollback(&mut prepared, index);
            return Err(OpenMohaaError::IncompleteTransaction);
        };
        if let Err(error) = replacement.persist(&prepared[index].target) {
            let target = prepared[index].target.clone();
            rollback(&mut prepared, index);
            return Err(OpenMohaaError::Filesystem {
                path: target,
                source: error.error,
            });
        }
    }

    if replacements == 0 {
        Ok(UpdateOutcome::Installed {
            files: entries.len(),
        })
    } else {
        Ok(UpdateOutcome::Updated {
            files: entries.len(),
            replaced: replacements,
        })
    }
}

fn rollback(prepared: &mut [PreparedFile], committed: usize) {
    for file in prepared[..committed].iter_mut().rev() {
        if let Some(backup) = file.backup.take() {
            let _ = backup.persist(&file.target);
        } else {
            let _ = fs::remove_file(&file.target);
        }
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Release discovery, verification, or installation error.
#[derive(Debug, Error)]
pub enum OpenMohaaError {
    #[error("could not configure GitHub client")]
    Client(#[source] reqwest::Error),
    #[error("GitHub request failed")]
    Network(#[source] reqwest::Error),
    #[error("GitHub returned HTTP {0}")]
    HttpStatus(u16),
    #[error("malformed GitHub release response")]
    MalformedRelease(#[source] serde_json::Error),
    #[error("release has no portable asset for {0:?}")]
    MissingAsset(ReleaseTarget),
    #[error("release asset {0:?} has no publisher digest")]
    MissingDigest(String),
    #[error("unsupported published digest {0:?}")]
    UnsupportedDigest(String),
    #[error("invalid published SHA-256 digest {0:?}")]
    InvalidDigest(String),
    #[error("release asset is {size} bytes; maximum is {maximum}")]
    AssetTooLarge { size: u64, maximum: u64 },
    #[error("release size differs: expected {expected}, downloaded {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("release digest differs: expected {expected}, downloaded {actual}")]
    DigestMismatch {
        expected: PublishedSha256,
        actual: PublishedSha256,
    },
    #[error("release is not a readable ZIP archive")]
    InvalidZip(#[source] zip::result::ZipError),
    #[error("unsafe release archive entry {0:?}")]
    UnsafeArchiveEntry(String),
    #[error("duplicate release archive entry {0:?}")]
    DuplicateArchiveEntry(String),
    #[error("release archive contains no files")]
    EmptyArchive,
    #[error("destination has no parent directory: {0}")]
    NoDestinationParent(PathBuf),
    #[error("release target is an existing directory: {0}")]
    TargetIsDirectory(PathBuf),
    #[error("release transaction was internally incomplete")]
    IncompleteTransaction,
    #[error("filesystem operation failed at {path}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Cursor, Write};

    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{
        ClientActivity, OpenMohaaError, PublishedSha256, ReleasePackage, ReleaseTarget,
        UpdateDeferredReason, UpdateOutcome, install_verified_archive, parse_latest_release,
    };

    #[test]
    fn selects_exact_portable_asset_and_requires_github_digest() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        let package =
            parse_latest_release(fixture, ReleaseTarget::WindowsX64).expect("x64 release package");
        assert_eq!(package.version, "v0.82.1");
        assert_eq!(package.asset_name, "openmohaa-v0.82.1-windows-x64.zip");
        assert_eq!(
            package.digest.to_hex(),
            "dba183f9c666928b3925e6fdd1f2a780517ef283170d14d55b7650607ca7bb6d"
        );

        let without_digest = fixture.replace(
            "\"digest\": \"sha256:dba183f9c666928b3925e6fdd1f2a780517ef283170d14d55b7650607ca7bb6d\"",
            "\"digest\": null",
        );
        assert!(matches!(
            parse_latest_release(&without_digest, ReleaseTarget::WindowsX64),
            Err(OpenMohaaError::MissingDigest(_))
        ));
    }

    #[test]
    fn installs_then_refuses_to_overwrite_while_running_or_unknown() {
        let temporary = TempDir::new().expect("temporary directory");
        let destination = temporary.path().join("profile");
        let first = archive(&[("openmohaa.exe", b"version one"), ("game.dll", b"one")]);
        let first_package = package(&first);
        assert_eq!(
            install_verified_archive(
                &first_package,
                &first,
                &destination,
                ClientActivity::Unknown
            )
            .expect("initial install"),
            UpdateOutcome::Installed { files: 2 }
        );

        let second = archive(&[("openmohaa.exe", b"version two"), ("game.dll", b"two")]);
        let second_package = package(&second);
        for (activity, reason) in [
            (ClientActivity::Running, UpdateDeferredReason::ClientRunning),
            (
                ClientActivity::Unknown,
                UpdateDeferredReason::ClientStateUnknown,
            ),
        ] {
            assert_eq!(
                install_verified_archive(&second_package, &second, &destination, activity)
                    .expect("deferred update"),
                UpdateOutcome::Deferred { reason }
            );
            assert_eq!(
                fs::read(destination.join("openmohaa.exe")).expect("existing executable"),
                b"version one"
            );
        }

        assert_eq!(
            install_verified_archive(
                &second_package,
                &second,
                &destination,
                ClientActivity::ConfirmedStopped
            )
            .expect("confirmed update"),
            UpdateOutcome::Updated {
                files: 2,
                replaced: 2
            }
        );
        assert_eq!(
            fs::read(destination.join("openmohaa.exe")).expect("updated executable"),
            b"version two"
        );
    }

    #[test]
    fn digest_mismatch_and_hostile_zip_make_no_installation_changes() {
        let temporary = TempDir::new().expect("temporary directory");
        let destination = temporary.path().join("profile");
        let bytes = archive(&[("openmohaa.exe", b"release")]);
        let mut wrong_package = package(&bytes);
        wrong_package.digest = PublishedSha256::from_bytes(b"another release");
        assert!(matches!(
            install_verified_archive(
                &wrong_package,
                &bytes,
                &destination,
                ClientActivity::ConfirmedStopped
            ),
            Err(OpenMohaaError::DigestMismatch { .. })
        ));
        assert!(!destination.exists());

        let hostile = archive(&[("../outside.exe", b"hostile")]);
        assert!(matches!(
            install_verified_archive(
                &package(&hostile),
                &hostile,
                &destination,
                ClientActivity::ConfirmedStopped
            ),
            Err(OpenMohaaError::UnsafeArchiveEntry(_))
        ));
        assert!(!destination.exists());
    }

    fn package(bytes: &[u8]) -> ReleasePackage {
        ReleasePackage {
            version: "fixture".to_owned(),
            asset_name: "openmohaa-fixture-windows-x64.zip".to_owned(),
            download_url: "https://example.invalid/openmohaa.zip".to_owned(),
            size: bytes.len() as u64,
            digest: PublishedSha256::from_bytes(bytes),
        }
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut archive = ZipWriter::new(&mut bytes);
            for (name, contents) in entries {
                archive
                    .start_file(*name, SimpleFileOptions::default())
                    .expect("archive entry");
                archive.write_all(contents).expect("entry contents");
            }
            archive.finish().expect("finish archive");
        }
        bytes.into_inner()
    }

    #[test]
    fn replacement_is_file_atomic_for_the_synthetic_fixture() {
        let temporary = TempDir::new().expect("temporary directory");
        let destination = temporary.path().join("profile");
        fs::create_dir(&destination).expect("profile directory");
        File::create(destination.join("unrelated-retail-asset.pk3")).expect("unrelated asset");
        let bytes = archive(&[("bin/helper.dll", b"helper")]);

        install_verified_archive(
            &package(&bytes),
            &bytes,
            &destination,
            ClientActivity::Unknown,
        )
        .expect("overlay install");
        assert!(destination.join("unrelated-retail-asset.pk3").exists());
        assert_eq!(
            fs::read(destination.join("bin/helper.dll")).expect("installed helper"),
            b"helper"
        );
    }
}
