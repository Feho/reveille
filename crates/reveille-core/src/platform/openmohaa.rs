// SPDX-License-Identifier: GPL-2.0-only

//! Digest-gated `OpenMoHAA` GitHub Release selection, download, installation, and update.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Cursor, Write as IoWrite};
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;
use zip::ZipArchive;

// GitHub REST Releases API: /releases/latest excludes prereleases, including OpenMoHAA's rolling
// `dev` release. https://docs.github.com/rest/releases/releases#get-the-latest-release
const STABLE_RELEASE_URL: &str = "https://api.github.com/repos/openmoh/openmohaa/releases/latest";
// OpenMoHAA publishes its opt-in rolling build as the `dev` prerelease tag.
const DEV_RELEASE_URL: &str = "https://api.github.com/repos/openmoh/openmohaa/releases/tags/dev";
const USER_AGENT: &str = "Reveille/0.1 (+https://github.com/openmoh/openmohaa)";
const MAX_RELEASE_BYTES: u64 = 128 * 1024 * 1024;

/// Executable stems every `OpenMoHAA` release archive installs, without a platform extension.
///
/// Read from the openmoh/openmohaa v0.82.1 archive central directories: the Windows archives
/// carry these five with an `.exe` suffix, the Linux and macOS archives carry them bare. Shared
/// libraries (`game`, `cgame`, SDL, curl, OpenAL) are deliberately absent — they are loaded, not
/// executed. Callers use this both to mark Unix binaries executable and to decide which process
/// names hold a lock on an installation.
pub const RELEASE_EXECUTABLE_STEMS: [&str; 5] = [
    "openmohaa",
    "omohaaded",
    "launch_openmohaa_base",
    "launch_openmohaa_spearhead",
    "launch_openmohaa_breakthrough",
];

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
    LinuxAmd64,
    LinuxArm64,
    LinuxArmhf,
    LinuxI686,
    MacosArm64,
    MacosX64,
    WindowsX64,
    WindowsX86,
    WindowsArm64,
}

impl ReleaseTarget {
    /// Resolve the Rust compilation host without substituting a nearby architecture.
    ///
    /// # Errors
    ///
    /// Returns the actual OS and architecture when `OpenMoHAA` publishes no supported archive for
    /// the host.
    pub fn for_host() -> Result<Self, UnsupportedHost> {
        Self::for_platform(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn for_platform(os: &str, architecture: &str) -> Result<Self, UnsupportedHost> {
        match (os, architecture) {
            ("linux", "x86_64") => Ok(Self::LinuxAmd64),
            ("linux", "aarch64") => Ok(Self::LinuxArm64),
            ("linux", "arm") => Ok(Self::LinuxArmhf),
            ("linux", "x86") => Ok(Self::LinuxI686),
            ("macos", "aarch64") => Ok(Self::MacosArm64),
            ("macos", "x86_64") => Ok(Self::MacosX64),
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            ("windows", "x86") => Ok(Self::WindowsX86),
            ("windows", "aarch64") => Ok(Self::WindowsArm64),
            _ => Err(UnsupportedHost {
                os: os.to_owned(),
                architecture: architecture.to_owned(),
            }),
        }
    }

    const fn asset_suffix(self, channel: ReleaseChannel) -> &'static str {
        // openmoh/openmohaa .github/workflows/tags-publish-release.yml, verified at v0.82.1.
        // Exact suffixes deliberately exclude stable Windows `-pdb.zip`/`.msi` assets and the
        // unsupported PowerPC archives. The dev channel only publishes complete Windows builds
        // inside its `-pdb.zip` assets. Both macOS hosts intentionally select one universal asset.
        match (channel, self) {
            (_, Self::LinuxAmd64) => "-linux-amd64.zip",
            (_, Self::LinuxArm64) => "-linux-arm64.zip",
            (_, Self::LinuxArmhf) => "-linux-armhf.zip",
            (_, Self::LinuxI686) => "-linux-i686.zip",
            (_, Self::MacosArm64 | Self::MacosX64) => "-macos-multiarch-arm64-x86_64.zip",
            (ReleaseChannel::Stable, Self::WindowsX64) => "-windows-x64.zip",
            (ReleaseChannel::Stable, Self::WindowsX86) => "-windows-x86.zip",
            (ReleaseChannel::Stable, Self::WindowsArm64) => "-windows-arm64.zip",
            (ReleaseChannel::Dev, Self::WindowsX64) => "-windows-x64-pdb.zip",
            (ReleaseChannel::Dev, Self::WindowsX86) => "-windows-x86-pdb.zip",
            (ReleaseChannel::Dev, Self::WindowsArm64) => "-windows-arm64-pdb.zip",
        }
    }
}

/// Release stream used for its endpoint, asset suffix table, and update identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    /// Versioned release returned by GitHub's latest stable endpoint.
    Stable,
    /// Opt-in rolling prerelease built from the latest `OpenMoHAA` commit.
    Dev,
}

impl ReleaseChannel {
    const fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => STABLE_RELEASE_URL,
            Self::Dev => DEV_RELEASE_URL,
        }
    }
}

/// Channel and host target needed to select one exact release asset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ReleaseSelector {
    pub channel: ReleaseChannel,
    pub target: ReleaseTarget,
}

impl ReleaseSelector {
    #[must_use]
    pub const fn stable(target: ReleaseTarget) -> Self {
        Self {
            channel: ReleaseChannel::Stable,
            target,
        }
    }

    #[must_use]
    pub const fn dev(target: ReleaseTarget) -> Self {
        Self {
            channel: ReleaseChannel::Dev,
            target,
        }
    }
}

/// Unsupported host returned without guessing another `OpenMoHAA` archive.
#[derive(Clone, Debug, Eq, Error, PartialEq, Serialize)]
#[error("OpenMoHAA publishes no supported archive for {os}/{architecture}")]
pub struct UnsupportedHost {
    pub os: String,
    pub architecture: String,
}

/// One digest-bearing portable release archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReleasePackage {
    /// Release stream that supplied this package.
    pub channel: ReleaseChannel,
    /// Stable tag or rolling dev release name; the latter contains the source commit identity.
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

/// Bytes received while downloading one release archive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseDownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

/// Parse a frozen or live GitHub release response and select one exact portable archive.
///
/// # Errors
///
/// Returns an error for malformed JSON, a missing/ambiguous asset, a missing dev identity, or a
/// missing/invalid published digest.
pub fn parse_release(
    json: &str,
    selector: ReleaseSelector,
) -> Result<ReleasePackage, OpenMohaaError> {
    let release: RawRelease =
        serde_json::from_str(json).map_err(OpenMohaaError::MalformedRelease)?;
    let suffix = selector.target.asset_suffix(selector.channel);
    let mut matching_assets = release
        .assets
        .into_iter()
        .filter(|asset| asset.name.ends_with(suffix));
    let asset = matching_assets
        .next()
        .ok_or(OpenMohaaError::MissingAsset(selector))?;
    if matching_assets.next().is_some() {
        return Err(OpenMohaaError::AmbiguousAsset(selector));
    }
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
    let version = match selector.channel {
        ReleaseChannel::Stable => release.tag_name,
        ReleaseChannel::Dev => release.name.ok_or(OpenMohaaError::MissingDevIdentity)?,
    };
    Ok(ReleasePackage {
        channel: selector.channel,
        version,
        asset_name: asset.name,
        download_url: asset.browser_download_url,
        size: asset.size,
        digest,
    })
}

/// Parse GitHub's latest stable-release response for a target.
///
/// # Errors
///
/// Returns the same errors as [`parse_release`].
pub fn parse_latest_release(
    json: &str,
    target: ReleaseTarget,
) -> Result<ReleasePackage, OpenMohaaError> {
    parse_release(json, ReleaseSelector::stable(target))
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

    /// Fetch and select one portable archive from an explicit release channel.
    ///
    /// # Errors
    ///
    /// Returns an error for HTTP, JSON, asset-selection, or digest failures.
    pub async fn release(
        &self,
        selector: ReleaseSelector,
    ) -> Result<ReleasePackage, OpenMohaaError> {
        let response = self
            .client
            .get(selector.channel.endpoint())
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(OpenMohaaError::Network)?;
        let status = response.status();
        if !status.is_success() {
            return Err(OpenMohaaError::HttpStatus(status.as_u16()));
        }
        let text = response.text().await.map_err(OpenMohaaError::Network)?;
        parse_release(&text, selector)
    }

    /// Fetch and select the latest stable portable archive for a target.
    ///
    /// # Errors
    ///
    /// Returns an error for HTTP, JSON, asset-selection, or digest failures.
    pub async fn latest_release(
        &self,
        target: ReleaseTarget,
    ) -> Result<ReleasePackage, OpenMohaaError> {
        self.release(ReleaseSelector::stable(target)).await
    }

    /// Download, verify, and transactionally overlay one release archive.
    ///
    /// `probe_activity` runs after the transfer completes rather than before it, so a client
    /// started while the archive was downloading is still seen. The caller supplies the probe;
    /// this crate performs no process inspection of its own.
    ///
    /// # Errors
    ///
    /// Returns an error before installation for network, size, or digest failures, and retains
    /// existing target files if an individual atomic replacement fails.
    pub async fn download_and_install<A>(
        &self,
        package: &ReleasePackage,
        destination: impl AsRef<Path>,
        probe_activity: A,
    ) -> Result<UpdateOutcome, OpenMohaaError>
    where
        A: FnOnce() -> ClientActivity,
    {
        self.download_and_install_reporting(package, destination, probe_activity, |_| {
            ControlFlow::Continue(())
        })
        .await
    }

    /// Download, verify, and transactionally overlay one release archive while reporting bytes.
    ///
    /// Returning [`ControlFlow::Break`] from `report` cancels before archive verification or any
    /// installation write.
    ///
    /// `probe_activity` is deliberately a closure rather than a [`ClientActivity`] value, and it
    /// runs only once the whole archive has arrived. Sampling before a multi-megabyte transfer
    /// leaves a window in which the player starts the client mid-download, which would turn an
    /// honest [`UpdateOutcome::Deferred`] into a locked-file failure part-way through the apply.
    ///
    /// # Errors
    ///
    /// Returns an error before installation for network, size, digest, or cancellation failures,
    /// and retains existing target files if an individual atomic replacement fails.
    pub async fn download_and_install_reporting<A, F>(
        &self,
        package: &ReleasePackage,
        destination: impl AsRef<Path>,
        probe_activity: A,
        mut report: F,
    ) -> Result<UpdateOutcome, OpenMohaaError>
    where
        A: FnOnce() -> ClientActivity,
        F: FnMut(ReleaseDownloadProgress) -> ControlFlow<()>,
    {
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
        let declared = response.content_length();
        if let Some(size) = declared.filter(|length| *length > MAX_RELEASE_BYTES) {
            return Err(OpenMohaaError::AssetTooLarge {
                size,
                maximum: MAX_RELEASE_BYTES,
            });
        }
        if report(ReleaseDownloadProgress {
            received: 0,
            total: declared.or(Some(package.size)),
        })
        .is_break()
        {
            return Err(OpenMohaaError::DownloadCancelled);
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
            if report(ReleaseDownloadProgress {
                received: bytes.len() as u64,
                total: declared.or(Some(package.size)),
            })
            .is_break()
            {
                return Err(OpenMohaaError::DownloadCancelled);
            }
        }
        install_verified_archive(package, &bytes, destination, probe_activity())
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
    #[cfg(unix)]
    executable: bool,
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
        #[cfg(unix)]
        let executable = is_known_unix_executable(&path);
        entries.push(ArchiveEntry {
            path,
            #[cfg(unix)]
            executable,
        });
    }
    if entries.is_empty() {
        return Err(OpenMohaaError::EmptyArchive);
    }
    Ok(entries)
}

#[cfg(any(unix, test))]
fn is_known_unix_executable(path: &Path) -> bool {
    // Unix archives ship these flat binaries as 0644 (see RELEASE_EXECUTABLE_STEMS). Shared
    // libraries are omitted intentionally; loading them does not require an executable bit.
    path.parent()
        .is_some_and(|parent| parent.as_os_str().is_empty())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| RELEASE_EXECUTABLE_STEMS.contains(&name))
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
        #[cfg(unix)]
        if entry.executable {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(replacement.path(), fs::Permissions::from_mode(0o755)).map_err(
                |source| OpenMohaaError::Filesystem {
                    path: target.clone(),
                    source,
                },
            )?;
        }
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
    name: Option<String>,
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
    MissingAsset(ReleaseSelector),
    #[error("release has more than one portable asset for {0:?}")]
    AmbiguousAsset(ReleaseSelector),
    #[error("rolling dev release has no commit-bearing release name")]
    MissingDevIdentity,
    #[error("release asset {0:?} has no publisher digest")]
    MissingDigest(String),
    #[error("unsupported published digest {0:?}")]
    UnsupportedDigest(String),
    #[error("invalid published SHA-256 digest {0:?}")]
    InvalidDigest(String),
    #[error("release asset is {size} bytes; maximum is {maximum}")]
    AssetTooLarge { size: u64, maximum: u64 },
    #[error("OpenMoHAA download was cancelled")]
    DownloadCancelled,
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
    use std::path::Path;

    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{
        ClientActivity, OpenMohaaError, PublishedSha256, ReleaseChannel, ReleasePackage,
        ReleaseSelector, ReleaseTarget, UpdateDeferredReason, UpdateOutcome,
        install_verified_archive, parse_latest_release, parse_release,
    };

    #[test]
    fn every_stable_target_selects_its_exact_portable_asset() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        let expected = [
            (
                ReleaseTarget::LinuxAmd64,
                "openmohaa-v0.82.1-linux-amd64.zip",
            ),
            (
                ReleaseTarget::LinuxArm64,
                "openmohaa-v0.82.1-linux-arm64.zip",
            ),
            (
                ReleaseTarget::LinuxArmhf,
                "openmohaa-v0.82.1-linux-armhf.zip",
            ),
            (ReleaseTarget::LinuxI686, "openmohaa-v0.82.1-linux-i686.zip"),
            (
                ReleaseTarget::MacosArm64,
                "openmohaa-v0.82.1-macos-multiarch-arm64-x86_64.zip",
            ),
            (
                ReleaseTarget::MacosX64,
                "openmohaa-v0.82.1-macos-multiarch-arm64-x86_64.zip",
            ),
            (
                ReleaseTarget::WindowsX64,
                "openmohaa-v0.82.1-windows-x64.zip",
            ),
            (
                ReleaseTarget::WindowsX86,
                "openmohaa-v0.82.1-windows-x86.zip",
            ),
            (
                ReleaseTarget::WindowsArm64,
                "openmohaa-v0.82.1-windows-arm64.zip",
            ),
        ];

        for (target, asset_name) in expected {
            let package = parse_latest_release(fixture, target).expect("stable release package");
            assert_eq!(package.channel, ReleaseChannel::Stable);
            assert_eq!(package.version, "v0.82.1");
            assert_eq!(package.asset_name, asset_name);
            assert!(!package.asset_name.ends_with("-pdb.zip"));
            assert!(
                !Path::new(&package.asset_name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"))
            );
        }
    }

    #[test]
    fn exact_selector_requires_one_digest_bearing_asset() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        let package =
            parse_latest_release(fixture, ReleaseTarget::WindowsX64).expect("x64 release package");
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

        let ambiguous = fixture.replace(
            "\"assets\": [",
            "\"assets\": [{\"name\":\"duplicate-windows-x64.zip\",\"browser_download_url\":\"https://example.invalid/duplicate.zip\",\"size\":1,\"digest\":\"sha256:0000000000000000000000000000000000000000000000000000000000000000\"},",
        );
        assert!(matches!(
            parse_latest_release(&ambiguous, ReleaseTarget::WindowsX64),
            Err(OpenMohaaError::AmbiguousAsset(_))
        ));
    }

    #[test]
    fn host_resolution_records_unsupported_pairs_without_guessing() {
        let supported = [
            ("linux", "x86_64", ReleaseTarget::LinuxAmd64),
            ("linux", "aarch64", ReleaseTarget::LinuxArm64),
            ("linux", "arm", ReleaseTarget::LinuxArmhf),
            ("linux", "x86", ReleaseTarget::LinuxI686),
            ("macos", "aarch64", ReleaseTarget::MacosArm64),
            ("macos", "x86_64", ReleaseTarget::MacosX64),
            ("windows", "x86_64", ReleaseTarget::WindowsX64),
            ("windows", "x86", ReleaseTarget::WindowsX86),
            ("windows", "aarch64", ReleaseTarget::WindowsArm64),
        ];
        for (os, architecture, target) in supported {
            assert_eq!(ReleaseTarget::for_platform(os, architecture), Ok(target));
        }

        let unsupported = ReleaseTarget::for_platform("linux", "powerpc")
            .expect_err("PowerPC is deliberately unsupported");
        assert_eq!(unsupported.os, "linux");
        assert_eq!(unsupported.architecture, "powerpc");
    }

    #[test]
    fn dev_selector_uses_its_own_endpoint_shape_and_release_identity() {
        let fixture = r#"{
            "tag_name":"dev",
            "name":"main-a2f34019",
            "assets":[{
                "name":"openmohaa-dev-windows-x64-pdb.zip",
                "browser_download_url":"https://example.invalid/dev.zip",
                "size":42,
                "digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"
            }]
        }"#;
        let package = parse_release(fixture, ReleaseSelector::dev(ReleaseTarget::WindowsX64))
            .expect("dev release package");
        assert_eq!(package.channel, ReleaseChannel::Dev);
        assert_eq!(package.version, "main-a2f34019");
        assert_eq!(package.asset_name, "openmohaa-dev-windows-x64-pdb.zip");
    }

    #[test]
    fn unix_executable_allowlist_is_narrow_and_archive_root_only() {
        for name in [
            "openmohaa",
            "omohaaded",
            "launch_openmohaa_base",
            "launch_openmohaa_spearhead",
            "launch_openmohaa_breakthrough",
        ] {
            assert!(super::is_known_unix_executable(Path::new(name)));
        }
        for name in [
            "game.so",
            "cgame.dylib",
            "nested/openmohaa",
            "openmohaa.exe",
        ] {
            assert!(!super::is_known_unix_executable(Path::new(name)));
        }
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
            channel: ReleaseChannel::Stable,
            version: "fixture".to_owned(),
            asset_name: "openmohaa-fixture-windows-x64.zip".to_owned(),
            download_url: "https://example.invalid/openmohaa.zip".to_owned(),
            size: bytes.len() as u64,
            digest: PublishedSha256::from_bytes(bytes),
        }
    }

    #[cfg(unix)]
    #[test]
    fn explicitly_marks_only_known_flat_unix_binaries_executable() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = TempDir::new().expect("temporary directory");
        let destination = temporary.path().join("profile");
        let bytes = archive(&[
            ("openmohaa", b"client"),
            ("omohaaded", b"server"),
            ("launch_openmohaa_base", b"launcher"),
            ("launch_openmohaa_spearhead", b"launcher"),
            ("launch_openmohaa_breakthrough", b"launcher"),
            ("game.so", b"library"),
            ("nested/openmohaa", b"unexpected nested client"),
        ]);

        install_verified_archive(
            &package(&bytes),
            &bytes,
            &destination,
            ClientActivity::Unknown,
        )
        .expect("Unix overlay install");

        for name in [
            "openmohaa",
            "omohaaded",
            "launch_openmohaa_base",
            "launch_openmohaa_spearhead",
            "launch_openmohaa_breakthrough",
        ] {
            let mode = fs::metadata(destination.join(name))
                .expect("known executable metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{name} was not executable");
        }
        for name in ["game.so", "nested/openmohaa"] {
            let mode = fs::metadata(destination.join(name))
                .expect("non-executable metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0, "{name} was unexpectedly executable");
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
