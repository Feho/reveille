// SPDX-License-Identifier: GPL-2.0-only

//! Digest-gated `OpenMoHAA` GitHub Release selection, download, installation, and update.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use thiserror::Error;
use zip::ZipArchive;

use super::package::{self as package_io, OverlayFile};

// GitHub filters by its release flags; selection also checks the tag for prerelease identifiers.
// https://docs.github.com/rest/releases/releases#get-the-latest-release
const STABLE_RELEASE_URL: &str =
    "https://api.github.com/repos/mohcentral/openmohaa/releases/latest";
// The preview channel has no fixed tag: every build publishes an immutable semver prerelease tag
// (`v0.83.0-rc.1`). The list endpoint is sorted by creation date, which lies when a hotfix is cut
// from an older branch, so the newest entry is chosen by parsed semver precedence instead.
// https://docs.github.com/rest/releases/releases#list-releases
const RELEASE_LIST_URL: &str = "https://api.github.com/repos/mohcentral/openmohaa/releases";
// GitHub REST list-releases pagination: request a fixed page size and stop at a short page.
// https://docs.github.com/rest/releases/releases#list-releases
const RELEASES_PER_PAGE: usize = 30;
const USER_AGENT: &str = "Reveille/0.1 (+https://github.com/mohcentral/openmohaa)";
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

    const fn asset_suffix(self) -> &'static str {
        // mohcentral/openmohaa .github/workflows/publish-release.yml — one tag-triggered workflow
        // publishes both channels, so the asset names no longer differ per channel. Exact
        // suffixes deliberately exclude the `-pdb.zip` symbol archives, the `.msi` installer and
        // the unsupported PowerPC builds. Both macOS hosts intentionally select one universal
        // asset. `-windows-x64.zip` does not match `-windows-x64-pdb.zip`, so the symbol archive
        // can ship alongside without making the selection ambiguous.
        match self {
            Self::LinuxAmd64 => "-linux-amd64.zip",
            Self::LinuxArm64 => "-linux-arm64.zip",
            Self::LinuxArmhf => "-linux-armhf.zip",
            Self::LinuxI686 => "-linux-i686.zip",
            Self::MacosArm64 | Self::MacosX64 => "-macos-multiarch-arm64-x86_64.zip",
            Self::WindowsX64 => "-windows-x64.zip",
            Self::WindowsX86 => "-windows-x86.zip",
            Self::WindowsArm64 => "-windows-arm64.zip",
        }
    }
}

/// Release stream used to select an endpoint and eligible versions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    /// Versioned release returned by GitHub's latest stable endpoint.
    Stable,
    /// Opt-in stream that takes the highest semver release, prereleases included.
    #[serde(alias = "dev")]
    Preview,
}

impl ReleaseChannel {
    const fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => STABLE_RELEASE_URL,
            Self::Preview => RELEASE_LIST_URL,
        }
    }

    /// Whether a parsed release belongs to this channel.
    ///
    /// Preview deliberately admits stable releases too: a player on the preview channel who has
    /// `v0.83.0-rc.2` installed must be offered `v0.83.0` when it ships, not stranded on the
    /// release candidate.
    const fn admits_prereleases(self) -> bool {
        match self {
            Self::Stable => false,
            Self::Preview => true,
        }
    }
}

/// Semver version parsed from a release tag, ordered by semver precedence.
///
/// Ordering, and therefore equality, is `semver`'s: build metadata (`+abc`) carries no
/// precedence, so `v0.83.0` and `v0.83.0+a2f3401` are the same version. The verbatim tag is kept
/// beside it only so that what is displayed is what was published, never a reconstruction.
#[derive(Clone, Debug)]
pub struct ReleaseVersion {
    version: semver::Version,
    tag: String,
}

impl ReleaseVersion {
    /// Parse a `v`-prefixed or bare semver tag such as `v0.83.0-rc.1`.
    ///
    /// Returns `None` for a tag that is not semver; callers skip such releases rather than
    /// failing, so an unrelated tag in the repository cannot break selection.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let version = semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
        Some(Self {
            version,
            tag: tag.to_owned(),
        })
    }

    /// Whether this version carries a prerelease identifier such as `-rc.1`.
    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        !self.version.pre.is_empty()
    }

    /// The tag exactly as published.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }
}

// Every comparison delegates to the parsed version and deliberately ignores `tag`: deriving
// these would make the tag a tiebreaker, so `v0.83.0` and `v0.83.0+a2f3401` — the same version by
// semver precedence — would compare unequal and order arbitrarily.
// Equality goes through `cmp`, so it means the same thing as ordering.
impl PartialEq for ReleaseVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ReleaseVersion {}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        // `cmp_precedence`, not `Ord::cmp`: `semver::Version` *derives* `Ord`, which makes build
        // metadata a tiebreaker. The spec gives it no precedence (semver §10), so `v0.83.0` and
        // `v0.83.0+a2f3401` must compare equal.
        self.version.cmp_precedence(&other.version)
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.tag)
    }
}

impl Serialize for ReleaseVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.tag)
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
    pub const fn preview(target: ReleaseTarget) -> Self {
        Self {
            channel: ReleaseChannel::Preview,
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
    /// The release tag exactly as published; immutable on both channels.
    pub version: String,
    /// Parsed semver of `version`, present for every tag Reveille will install.
    pub semver: ReleaseVersion,
    /// Whether the selected release is a prerelease. The preview channel can legitimately serve
    /// a stable release, so this is a property of the release, not of the channel.
    pub prerelease: bool,
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

/// Parse a frozen or live single-release GitHub response and select one exact portable archive.
///
/// # Errors
///
/// Returns an error for malformed JSON, a non-semver tag, a draft or excluded prerelease,
/// a missing/ambiguous asset, or a missing/invalid published digest.
pub fn parse_release(
    json: &str,
    selector: ReleaseSelector,
) -> Result<ReleasePackage, OpenMohaaError> {
    let release: RawRelease =
        serde_json::from_str(json).map_err(OpenMohaaError::MalformedRelease)?;
    select_package(release, selector)
}

/// Parse a frozen or live `/releases` list and select the highest semver release for a channel.
///
/// A tag that is not semver is skipped rather than treated as a failure: an unrelated tag pushed
/// to the repository must not be able to break selection for every player. GitHub sorts the list
/// by creation date, which misorders a hotfix cut from an older branch, so precedence is decided
/// by the parsed version.
///
/// # Errors
///
/// Returns [`OpenMohaaError::NoSelectableRelease`] when no published release carries a semver
/// tag the channel admits, and otherwise the same errors as [`parse_release`].
pub fn parse_release_list(
    json: &str,
    selector: ReleaseSelector,
) -> Result<ReleasePackage, OpenMohaaError> {
    let releases: Vec<RawRelease> =
        serde_json::from_str(json).map_err(OpenMohaaError::MalformedRelease)?;
    select_release_list(releases, selector)
}

fn select_release_list(
    releases: Vec<RawRelease>,
    selector: ReleaseSelector,
) -> Result<ReleasePackage, OpenMohaaError> {
    let newest = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = ReleaseVersion::parse(&release.tag_name)?;
            // A release counts as a prerelease if either its tag or GitHub's flag says so; the
            // two disagreeing is a publishing mistake that must not leak an untested build into
            // the stable channel.
            let is_prerelease = version.is_prerelease() || release.prerelease;
            (selector.channel.admits_prereleases() || !is_prerelease).then_some((version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release)
        .ok_or(OpenMohaaError::NoSelectableRelease(selector.channel))?;
    select_package(newest, selector)
}

fn select_package(
    release: RawRelease,
    selector: ReleaseSelector,
) -> Result<ReleasePackage, OpenMohaaError> {
    let semver = ReleaseVersion::parse(&release.tag_name)
        .ok_or_else(|| OpenMohaaError::UnversionedRelease(release.tag_name.clone()))?;
    let prerelease = semver.is_prerelease() || release.prerelease;
    if release.draft || (prerelease && !selector.channel.admits_prereleases()) {
        return Err(OpenMohaaError::NoSelectableRelease(selector.channel));
    }
    let suffix = selector.target.asset_suffix();
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
    Ok(ReleasePackage {
        channel: selector.channel,
        version: release.tag_name,
        prerelease,
        semver,
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
        self.release_from(selector, selector.channel.endpoint())
            .await
    }

    async fn release_from(
        &self,
        selector: ReleaseSelector,
        endpoint: &str,
    ) -> Result<ReleasePackage, OpenMohaaError> {
        let mut releases = Vec::new();
        let mut page = 1;
        loop {
            let mut request = self.client.get(endpoint);
            if selector.channel == ReleaseChannel::Preview {
                request = request.query(&[("per_page", RELEASES_PER_PAGE), ("page", page)]);
            }
            let response = request
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(OpenMohaaError::Network)?;
            let status = response.status();
            if !status.is_success() {
                return Err(OpenMohaaError::HttpStatus(status.as_u16()));
            }
            let text = response.text().await.map_err(OpenMohaaError::Network)?;
            if selector.channel == ReleaseChannel::Stable {
                return parse_release(&text, selector);
            }
            let batch: Vec<RawRelease> =
                serde_json::from_str(&text).map_err(OpenMohaaError::MalformedRelease)?;
            let last_page = batch.len() < RELEASES_PER_PAGE;
            releases.extend(batch);
            if last_page {
                return select_release_list(releases, selector);
            }
            page += 1;
        }
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
        let bytes = package_io::download_reporting(
            &self.client,
            &package.download_url,
            package.size,
            MAX_RELEASE_BYTES,
            |progress| {
                report(ReleaseDownloadProgress {
                    received: progress.received,
                    total: progress.total,
                })
            },
        )
        .await
        .map_err(|error| match error {
            package_io::DownloadError::Network(source) => OpenMohaaError::Network(source),
            package_io::DownloadError::HttpStatus(status) => OpenMohaaError::HttpStatus(status),
            package_io::DownloadError::TooLarge { size, maximum } => {
                OpenMohaaError::AssetTooLarge { size, maximum }
            }
            package_io::DownloadError::Cancelled => OpenMohaaError::DownloadCancelled,
        })?;
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
    package_io::verify_sha256(bytes, package.size, &package.digest.to_hex()).map_err(|error| {
        match error {
            package_io::VerifyError::Size { expected, actual } => {
                OpenMohaaError::SizeMismatch { expected, actual }
            }
            package_io::VerifyError::Digest { .. } => OpenMohaaError::DigestMismatch {
                expected: package.digest,
                actual: PublishedSha256::from_bytes(bytes),
            },
        }
    })?;
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
    package_io::safe_archive_path(value)
        .ok_or_else(|| OpenMohaaError::UnsafeArchiveEntry(value.to_owned()))
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
    let sources = entries
        .iter()
        .map(|entry| staging.path().join(&entry.path))
        .collect::<Vec<_>>();
    let overlays = entries
        .iter()
        .zip(&sources)
        .map(|(entry, source)| OverlayFile {
            source,
            target: destination.join(&entry.path),
            #[cfg(unix)]
            executable: entry.executable,
            #[cfg(not(unix))]
            executable: false,
        })
        .collect::<Vec<_>>();
    package_io::transactional_overlay(&overlays).map_err(|error| match error {
        package_io::ApplyError::NoParent(path) => OpenMohaaError::NoDestinationParent(path),
        package_io::ApplyError::TargetDirectory(path) => OpenMohaaError::TargetIsDirectory(path),
        package_io::ApplyError::Incomplete => OpenMohaaError::IncompleteTransaction,
        package_io::ApplyError::Filesystem { path, source } => {
            OpenMohaaError::Filesystem { path, source }
        }
    })?;

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
    // `/releases/latest` omits neither, but a hand-frozen fixture may; defaulting keeps a missing
    // flag from being read as "published stable".
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
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
    #[error("no published release carries a semver tag for the {0:?} channel")]
    NoSelectableRelease(ReleaseChannel),
    #[error("release tag {0:?} is not a semver version")]
    UnversionedRelease(String),
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
        ReleaseSelector, ReleaseTarget, ReleaseVersion, UpdateDeferredReason, UpdateOutcome,
        install_verified_archive, parse_latest_release, parse_release_list,
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
    fn preview_channel_takes_the_highest_semver_release_not_the_newest_entry() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_release_list.json");
        let package =
            parse_release_list(fixture, ReleaseSelector::preview(ReleaseTarget::WindowsX64))
                .expect("preview release package");
        // The list is ordered by creation date and leads with the v0.82.2 hotfix cut from an
        // older branch; semver precedence must still choose the v0.83.0-rc.2 candidate.
        assert_eq!(package.channel, ReleaseChannel::Preview);
        assert_eq!(package.version, "v0.83.0-rc.2");
        assert!(package.prerelease);
        assert_eq!(package.asset_name, "openmohaa-v0.83.0-rc.2-windows-x64.zip");
    }

    #[test]
    fn stable_selection_from_a_list_skips_every_prerelease_and_draft() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_release_list.json");
        let package =
            parse_release_list(fixture, ReleaseSelector::stable(ReleaseTarget::WindowsX64))
                .expect("stable release package");
        assert_eq!(package.version, "v0.82.2");
        assert!(!package.prerelease);
    }

    #[test]
    fn preview_channel_offers_a_stable_release_once_it_outranks_the_candidate() {
        // The candidate is promoted exactly as publishing would: the tag loses its prerelease
        // identifier and GitHub's flag is cleared with it.
        let fixture = include_str!("../../tests/fixtures/openmohaa_release_list.json")
            .replace("v0.83.0-rc.2", "v0.83.0")
            .replace(
                "\"tag_name\": \"v0.83.0\",
    \"draft\": false,
    \"prerelease\": true",
                "\"tag_name\": \"v0.83.0\",
    \"draft\": false,
    \"prerelease\": false",
            );
        let package = parse_release_list(
            &fixture,
            ReleaseSelector::preview(ReleaseTarget::WindowsX64),
        )
        .expect("preview release package");
        assert_eq!(package.version, "v0.83.0");
        assert!(!package.prerelease);
    }

    #[test]
    fn a_non_semver_tag_is_skipped_rather_than_failing_the_whole_channel() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_release_list.json").replace(
            "\"tag_name\": \"v0.83.0-rc.2\"",
            "\"tag_name\": \"nightly-2026-09-05\"",
        );
        let package = parse_release_list(
            &fixture,
            ReleaseSelector::preview(ReleaseTarget::WindowsX64),
        )
        .expect("preview release package");
        assert_eq!(package.version, "v0.83.0-rc.1");

        let empty = parse_release_list("[]", ReleaseSelector::preview(ReleaseTarget::WindowsX64));
        assert!(matches!(
            empty,
            Err(OpenMohaaError::NoSelectableRelease(ReleaseChannel::Preview))
        ));
    }

    #[test]
    fn a_stable_tag_flagged_prerelease_stays_out_of_the_stable_channel() {
        let fixture = r#"[{
            "tag_name": "v0.84.0",
            "draft": false,
            "prerelease": true,
            "assets": [{
                "name": "openmohaa-v0.84.0-windows-x64.zip",
                "browser_download_url": "https://example.invalid/openmohaa-v0.84.0-windows-x64.zip",
                "size": 42,
                "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            }]
        }]"#;
        assert!(matches!(
            parse_release_list(fixture, ReleaseSelector::stable(ReleaseTarget::WindowsX64)),
            Err(OpenMohaaError::NoSelectableRelease(ReleaseChannel::Stable))
        ));
        let package =
            parse_release_list(fixture, ReleaseSelector::preview(ReleaseTarget::WindowsX64))
                .expect("preview release package");
        assert!(package.prerelease);
    }

    #[test]
    fn single_release_selection_enforces_channel_and_draft_guards() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        for (tag, prerelease, draft) in [
            ("v0.83.0-rc.1", false, false),
            ("v0.83.0", true, false),
            ("v0.83.0", false, true),
        ] {
            let mut release: serde_json::Value = serde_json::from_str(fixture).expect("fixture");
            release["tag_name"] = tag.into();
            release["prerelease"] = prerelease.into();
            release["draft"] = draft.into();
            let json = release.to_string();
            assert!(matches!(
                super::parse_release(&json, ReleaseSelector::stable(ReleaseTarget::WindowsX64)),
                Err(OpenMohaaError::NoSelectableRelease(ReleaseChannel::Stable))
            ));
            let preview =
                super::parse_release(&json, ReleaseSelector::preview(ReleaseTarget::WindowsX64));
            if draft {
                assert!(matches!(
                    preview,
                    Err(OpenMohaaError::NoSelectableRelease(_))
                ));
            } else {
                assert!(preview.expect("preview candidate").prerelease);
            }
        }
    }

    #[test]
    fn invalid_semver_identifiers_cannot_win_release_selection() {
        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        let stable: serde_json::Value = serde_json::from_str(fixture).expect("fixture");
        for tag in [
            "v9.0.0-rc_1",
            "v9.0.0-01",
            "v9.0.0-rc..1",
            "v9.0.0-rc.01",
            "v9.0.0-rc.é",
            "v9.0.0+",
            "v9.0.0+build_1",
            "v9.0.0+build..1",
            "v9.0.0+build+other",
            "v9.0.0-rc.1+bad_metadata",
        ] {
            assert!(ReleaseVersion::parse(tag).is_none(), "{tag}");
            let mut invalid = stable.clone();
            invalid["tag_name"] = tag.into();
            invalid["assets"] = serde_json::json!([]);
            let json = serde_json::json!([invalid, stable]).to_string();
            for channel in [ReleaseChannel::Stable, ReleaseChannel::Preview] {
                let package = parse_release_list(
                    &json,
                    ReleaseSelector {
                        channel,
                        target: ReleaseTarget::WindowsX64,
                    },
                )
                .expect("invalid tag skipped before asset selection");
                assert_eq!(package.version, "v0.82.1", "{tag}");
            }
        }
        for tag in ["v9.0.0-0", "v9.0.0-01a", "v9.0.0-rc-1+001.build-1"] {
            assert!(ReleaseVersion::parse(tag).is_some(), "{tag}");
        }
    }

    #[tokio::test]
    async fn preview_fetches_every_page_before_selecting_a_release() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let fixture = include_str!("../../tests/fixtures/openmohaa_latest_release.json");
        let stable: serde_json::Value = serde_json::from_str(fixture).expect("fixture");
        let mut older = stable.clone();
        older["tag_name"] = "v0.81.0-rc.1".into();
        older["prerelease"] = true.into();
        let first_page =
            serde_json::to_string(&vec![older; super::RELEASES_PER_PAGE]).expect("first page");
        for (status, body) in [
            ("200 OK", serde_json::json!([stable]).to_string()),
            ("503 Service Unavailable", "{}".to_owned()),
            ("200 OK", "invalid JSON".to_owned()),
            ("200 OK", "[]".to_owned()),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("loopback server");
            let endpoint = format!(
                "http://{}/releases",
                listener.local_addr().expect("address")
            );
            let responses = [("200 OK", first_page.clone()), (status, body.clone())];
            let server = tokio::spawn(async move {
                for (index, (status, body)) in responses.into_iter().enumerate() {
                    let (mut stream, _) = listener.accept().await.expect("request");
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).await.expect("request header");
                        request.push(byte[0]);
                    }
                    let request = String::from_utf8(request).expect("HTTP request");
                    assert!(
                        request.starts_with(&format!(
                            "GET /releases?per_page=30&page={} HTTP/1.1\r\n",
                            index + 1,
                        )),
                        "{request}"
                    );
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len(),
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("response");
                }
            });
            let client = super::OpenMohaaReleaseClient {
                client: reqwest::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .expect("client"),
            };
            let result = client
                .release_from(
                    ReleaseSelector::preview(ReleaseTarget::WindowsX64),
                    &endpoint,
                )
                .await;
            server.await.expect("server completed");
            if status.starts_with("503") {
                assert!(matches!(result, Err(OpenMohaaError::HttpStatus(503))));
            } else if body == "invalid JSON" {
                assert!(matches!(result, Err(OpenMohaaError::MalformedRelease(_))));
            } else {
                let expected = if body == "[]" {
                    "v0.81.0-rc.1"
                } else {
                    "v0.82.1"
                };
                assert_eq!(result.expect("highest release").version, expected);
            }
        }
    }

    #[test]
    fn semver_precedence_orders_candidates_below_their_release() {
        let ordered = [
            "v0.82.9",
            "v0.83.0-alpha.1",
            "v0.83.0-rc.1",
            "v0.83.0-rc.2",
            "v0.83.0-rc.10",
            "v0.83.0-rc.18446744073709551616",
            "v0.83.0-rc.100000000000000000000",
            "v0.83.0-rc.a",
            "v0.83.0",
            "v0.83.1",
        ]
        .map(|tag| ReleaseVersion::parse(tag).expect("semver tag"));
        for pair in ordered.windows(2) {
            assert!(pair[0] < pair[1], "{} !< {}", pair[0], pair[1]);
        }
        // Build metadata carries no precedence, and the displayed tag stays verbatim.
        let build = ReleaseVersion::parse("v0.83.0+a2f3401").expect("semver tag");
        assert_eq!(build, ReleaseVersion::parse("0.83.0").expect("semver tag"));
        assert_eq!(build.tag(), "v0.83.0+a2f3401");

        for rejected in ["dev", "v0.83", "v0.83.0.1", "v01.0.0", "v0.83.0-"] {
            assert!(ReleaseVersion::parse(rejected).is_none(), "{rejected}");
        }
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
            version: "v0.82.1".to_owned(),
            semver: ReleaseVersion::parse("v0.82.1").expect("fixture semver"),
            prerelease: false,
            asset_name: "openmohaa-v0.82.1-windows-x64.zip".to_owned(),
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
