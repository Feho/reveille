// SPDX-License-Identifier: GPL-2.0-only

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell. This layer owns presentation policy: it turns the pipeline's typed results into
//! payloads and progress events, and decides nothing the core has not already established.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{self, Read as _};
use std::net::SocketAddrV4;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use reveille_core::content::{
    self, CatalogueCandidate, CatalogueResolutionPass, DownloadProgress, ResolutionOutcome,
    WantedMap,
};
use reveille_core::discovery::{
    self, BrowseConfig, BrowseEvent, BrowseSummary, MasterEndpoint, NonResult, NonResultReason,
    ProbeStage, QueryPort, Server, TargetGame,
};
use reveille_core::engine::EngineChoice;
use reveille_core::install::{self, Installation};
use reveille_core::join::{
    CompatibilityAssessment, CompatibilityState, CurrentMapReadiness, FsGame, LaunchCommand,
    LaunchProfile,
};
use reveille_core::mapindex::{MapIndex, MapKey};
use reveille_core::platform::openmohaa::{
    ClientActivity, OpenMohaaError, OpenMohaaReleaseClient, ReleaseChannel,
    ReleaseDownloadProgress, ReleasePackage, ReleaseSelector, ReleaseTarget, UpdateOutcome,
};
use reveille_core::platform::reborn::{
    self, DownloadProgress as RebornDownloadProgress, RebornClient,
};
use reveille_platform as platform;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use thiserror::Error;
use tokio::sync::{Notify, mpsc, oneshot};

/// Events the frontend listens for, kept together so the contract reads in one place.
const BROWSE_EVENT: &str = "reveille://browse";
const PREVIEW_EVENT: &str = "reveille://preview";
const INSTALL_EVENT: &str = "reveille://install";
const OPENMOHAA_INSTALL_EVENT: &str = "reveille://openmohaa-install";
const REBORN_INSTALL_EVENT: &str = "reveille://reborn-install";

/// Deadline for one per-server UDP probe.
///
/// The sweep and the single-server check share it deliberately: a remembered server checked on a
/// gentler deadline than the sweep uses would be listed on terms the list itself never offered.
const PROBE_TIMEOUT: Duration = Duration::from_millis(2_500);

/// Bytes between download progress emissions. A 24 MB shopping list produces a few hundred events
/// rather than tens of thousands.
const DOWNLOAD_EVENT_STRIDE: u64 = 256 * 1024;
const OPENMOHAA_RECEIPT_FILENAME: &str = ".reveille-openmohaa.json";
const OPENMOHAA_RECEIPT_FORMAT: OpenMohaaReceiptFormat = OpenMohaaReceiptFormat(1);

#[derive(Default)]
struct AppState {
    /// The servers behind the current list, keyed on by address when a join is prepared.
    servers: Mutex<Vec<Server>>,
    /// Raised to stop an in-flight sweep.
    cancel_browse: Notify,
    /// The most recent join preview, reused so a launch does not repeat the catalogue pass.
    preview: Mutex<Option<CachedPreview>>,
    /// Only one engine archive may target an installation at a time.
    openmohaa_install: tokio::sync::Mutex<()>,
    /// Read between download chunks; cancellation never interrupts the atomic apply phase.
    openmohaa_cancel: AtomicBool,
    /// Release offers already shown to the player, retained so Install uses those exact bytes.
    openmohaa_offers: Mutex<VecDeque<CachedOpenMohaaOffer>>,
    /// Opaque identity generator for cached offers.
    openmohaa_next_offer: AtomicU64,
    /// Cancellation for the pinned Reborn archive transfer.
    reborn_cancel: AtomicBool,
}

const OPENMOHAA_OFFER_CACHE_CAPACITY: usize = 8;

#[derive(Clone)]
struct CachedOpenMohaaOffer {
    id: OpenMohaaOfferId,
    installation_root: PathBuf,
    target: ReleaseTarget,
    package: ReleasePackage,
}

struct CachedPreview {
    install_root: PathBuf,
    engine: EngineChoice,
    game: TargetGame,
    preview: JoinPreview,
}

#[derive(Serialize)]
struct BrowserPayload {
    servers: Vec<BrowserServer>,
    summary: BrowseSummary,
    non_results: Vec<NonResultGroup>,
    cancelled: bool,
}

#[derive(Clone, Serialize)]
struct BrowserServer {
    address: SocketAddrV4,
    server: Server,
    compatibility: CompatibilityAssessment,
}

/// What checking one remembered server found.
///
/// Never an error and never an empty success: either the server answered and is now joinable, or
/// the reason it did not is recorded (H9).
#[derive(Serialize)]
struct CheckResult {
    row: Option<BrowserServer>,
    non_result: Option<NonResultGroup>,
    /// The server answered, but for another of the three games (H14).
    other_game: Option<TargetGame>,
}

/// Recorded non-results grouped for display. Individual reasons stay distinguishable; only the
/// repetition is collapsed.
#[derive(Serialize)]
struct NonResultGroup {
    stage: ProbeStage,
    reason: &'static str,
    detail: Option<String>,
    count: usize,
}

#[derive(Clone, Serialize)]
struct BrowseProgress {
    registered: usize,
    inspected: usize,
    probed: usize,
    answered: usize,
    non_results: usize,
    row: Option<BrowserServer>,
}

#[derive(Clone, Serialize)]
struct PreviewProgress {
    address: SocketAddrV4,
    index: usize,
    of: usize,
    map: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum InstallPhase {
    Downloading { received: u64, total: Option<u64> },
    Confirming,
    Installed,
    Failed { reason: String },
}

#[derive(Clone, Serialize)]
struct InstallProgress {
    map: String,
    filename: String,
    index: usize,
    of: usize,
    #[serde(flatten)]
    phase: InstallPhase,
}

#[derive(Clone, Serialize)]
struct JoinPreview {
    address: SocketAddrV4,
    server: Server,
    assessment: CompatibilityAssessment,
    catalogue: Option<CatalogueResolutionPass>,
    game_directory: PathBuf,
    used_home_fallback: bool,
    engine: EngineChoice,
    game: TargetGame,
}

/// One map that could not be installed. Structured rather than pre-formatted prose, so the
/// interface decides how to say it.
#[derive(Serialize)]
struct InstallFailure {
    map: String,
    reason: String,
}

/// What happened at the launch gate. A refusal always carries its reason.
#[derive(Serialize)]
#[serde(tag = "launch", rename_all = "snake_case")]
enum LaunchOutcome {
    Launched { process_id: u32 },
    Refused { reason: String },
}

#[derive(Serialize)]
struct JoinResult {
    assessment: CompatibilityAssessment,
    installed: Vec<PathBuf>,
    failures: Vec<InstallFailure>,
    game_directory: PathBuf,
    used_home_fallback: bool,
    engine: EngineChoice,
    game: TargetGame,
    outcome: LaunchOutcome,
}

#[derive(Serialize)]
struct EngineOverview {
    inventory: platform::engine::EngineInventory,
    resolved: Option<EngineChoice>,
    selection_error: Option<String>,
    reborn: RebornSummary,
}

#[derive(Clone, Serialize)]
struct RebornSummary {
    version: &'static str,
    filename: String,
    size: u64,
    sha256: &'static str,
    supported: bool,
}

#[derive(Serialize)]
struct RebornInstallResult {
    engine: EngineChoice,
    inventory: platform::engine::EngineInventory,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
struct OpenMohaaOfferId(u64);

#[derive(Clone, Serialize)]
struct OpenMohaaReleaseSummary {
    offer_id: OpenMohaaOfferId,
    channel: ReleaseChannel,
    version: String,
    asset_name: String,
    size: u64,
    digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct OpenMohaaReceiptFormat(u8);

#[derive(Debug, Deserialize, Serialize)]
struct OpenMohaaInstallReceipt {
    format: OpenMohaaReceiptFormat,
    channel: ReleaseChannel,
    version: String,
    asset_name: String,
    release_digest: String,
    client_sha256: String,
}

#[derive(Debug, Error)]
enum OpenMohaaReceiptError {
    #[error("could not access OpenMoHAA receipt data at {path}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not encode or decode the OpenMoHAA receipt")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum OpenMohaaInstalledBuild {
    Absent,
    Current,
    KnownOther {
        channel: ReleaseChannel,
        version: String,
    },
    Unknown,
}

#[derive(Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum OpenMohaaStatus {
    Available {
        target: ReleaseTarget,
        installed_build: OpenMohaaInstalledBuild,
        activity: OpenMohaaActivitySummary,
        package: OpenMohaaReleaseSummary,
    },
    Unsupported {
        os: String,
        architecture: String,
    },
}

#[derive(Serialize)]
struct OpenMohaaInstallResult {
    package: OpenMohaaReleaseSummary,
    outcome: UpdateOutcome,
    activity: OpenMohaaActivitySummary,
    installed_build: OpenMohaaInstalledBuild,
}

#[derive(Clone, Serialize)]
struct OpenMohaaActivitySummary {
    state: ClientActivity,
    running: Vec<OpenMohaaRunningProgram>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum OpenMohaaRunningProgram {
    Game,
    DedicatedServer,
    Launcher,
}

#[derive(Clone, Serialize)]
struct OpenMohaaInstallProgress {
    received: u64,
    total: Option<u64>,
}

/// Why an engine step stopped, classified once here rather than by matching message text.
///
/// The interface has to say what actually happened — a release that published no digest is not
/// a corrupted download, and neither is a release that has no build for this machine. Reading
/// that distinction out of a formatted `OpenMohaaError` string in JavaScript is how the two got
/// merged, so the classification lives beside the errors it names. `detail` carries the original
/// message for diagnosis; the shell chooses its own wording from `kind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OpenMohaaFailureKind {
    /// GitHub could not be reached or refused the request.
    Unreachable,
    /// The release exists but publishes nothing for this machine.
    NoAssetForHost,
    /// The release metadata is unusable — no digest, an ambiguous asset, no dev identity.
    /// Nothing was downloaded, so retrying changes nothing.
    ReleaseMetadata,
    /// Bytes arrived but did not match the published size or digest.
    CorruptDownload,
    /// The archive itself was rejected before any file was written.
    ArchiveRejected,
    /// The player pressed Stop.
    Cancelled,
    /// Writing into the game folder failed; existing files were restored.
    Filesystem,
    /// A failure outside the release pipeline, carried through with its own message rather than
    /// dressed up as one of the causes above.
    Internal,
}

#[derive(Clone, Debug, Serialize)]
struct OpenMohaaFailure {
    kind: OpenMohaaFailureKind,
    detail: String,
}

impl From<OpenMohaaError> for OpenMohaaFailure {
    fn from(error: OpenMohaaError) -> Self {
        use OpenMohaaFailureKind as Kind;

        let kind = match error {
            OpenMohaaError::Client(_)
            | OpenMohaaError::Network(_)
            | OpenMohaaError::HttpStatus(_) => Kind::Unreachable,
            OpenMohaaError::MissingAsset(_) => Kind::NoAssetForHost,
            OpenMohaaError::MalformedRelease(_)
            | OpenMohaaError::AmbiguousAsset(_)
            | OpenMohaaError::MissingDevIdentity
            | OpenMohaaError::MissingDigest(_)
            | OpenMohaaError::UnsupportedDigest(_)
            | OpenMohaaError::InvalidDigest(_)
            | OpenMohaaError::AssetTooLarge { .. } => Kind::ReleaseMetadata,
            OpenMohaaError::SizeMismatch { .. } | OpenMohaaError::DigestMismatch { .. } => {
                Kind::CorruptDownload
            }
            OpenMohaaError::DownloadCancelled => Kind::Cancelled,
            OpenMohaaError::InvalidZip(_)
            | OpenMohaaError::UnsafeArchiveEntry(_)
            | OpenMohaaError::DuplicateArchiveEntry(_)
            | OpenMohaaError::EmptyArchive => Kind::ArchiveRejected,
            OpenMohaaError::NoDestinationParent(_)
            | OpenMohaaError::TargetIsDirectory(_)
            | OpenMohaaError::IncompleteTransaction
            | OpenMohaaError::Filesystem { .. } => Kind::Filesystem,
        };
        Self {
            kind,
            detail: error.to_string(),
        }
    }
}

impl OpenMohaaFailure {
    /// A failure outside the release pipeline: an unreadable folder, or a busy install lock.
    fn other(detail: &impl ToString) -> Self {
        Self {
            kind: OpenMohaaFailureKind::Internal,
            detail: detail.to_string(),
        }
    }

    fn no_asset_for_host(detail: &impl ToString) -> Self {
        Self {
            kind: OpenMohaaFailureKind::NoAssetForHost,
            detail: detail.to_string(),
        }
    }
}

#[tauri::command]
fn detect_install(selected_path: Option<String>) -> Result<Option<Installation>, String> {
    if let Some(path) = selected_path.filter(|path| !path.trim().is_empty()) {
        return install::identify(path)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    #[cfg(windows)]
    {
        let keys = reveille_core::platform::registry::read_live_hives()
            .map_err(|error| error.to_string())?;
        let mut roots = reveille_core::platform::registry::discover_ea_install_roots(&keys)
            .into_iter()
            .map(|candidate| candidate.root)
            .collect::<Vec<_>>();
        if let Some(root) = reveille_core::platform::registry::discover_gog_install_root(&keys) {
            roots.push(root);
        }
        for root in roots {
            if let Ok(installation) = install::identify(root) {
                return Ok(Some(installation));
            }
        }
    }
    Ok(None)
}

#[tauri::command]
fn engine_overview(
    path: String,
    saved_engine: Option<EngineChoice>,
) -> Result<EngineOverview, String> {
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    let package = reborn::package(reborn::RebornProductSet::from_products(
        &installation.products,
    ));
    let resolved = platform::engine::resolve_choice(&installation.root, saved_engine);
    let (resolved, selection_error) = match resolved {
        Ok(choice) => (Some(choice), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(EngineOverview {
        inventory: platform::engine::inventory(&installation.root),
        resolved,
        selection_error,
        reborn: RebornSummary {
            version: package.version,
            filename: package.filename,
            size: package.size,
            sha256: package.sha256,
            supported: cfg!(windows),
        },
    })
}

#[tauri::command]
fn select_engine(path: String, engine: EngineChoice) -> Result<EngineOverview, String> {
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    if engine == EngineChoice::Openmohaa {
        platform::engine::resolve_choice(&installation.root, Some(engine))
            .map_err(|error| error.to_string())?;
    } else {
        platform::engine::activate(
            &installation.root,
            engine,
            platform::engine::retail_activity(),
        )
        .map_err(|error| error.to_string())?;
    }
    engine_overview(
        installation.root.to_string_lossy().into_owned(),
        Some(engine),
    )
}

#[tauri::command]
async fn install_reborn(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RebornInstallResult, String> {
    if !cfg!(windows) {
        return Err("The pinned Reborn legacy player package supports Windows only.".to_owned());
    }
    let _guard = state
        .openmohaa_install
        .try_lock()
        .map_err(|_| "another engine install is already running".to_owned())?;
    state.reborn_cancel.store(false, Ordering::Release);
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    let package = reborn::package(reborn::RebornProductSet::from_products(
        &installation.products,
    ));
    let client = RebornClient::new(Duration::from_secs(120)).map_err(|error| error.to_string())?;
    let bytes = client
        .download_reporting(&package, |RebornDownloadProgress { received, total }| {
            drop(app.emit(
                REBORN_INSTALL_EVENT,
                OpenMohaaInstallProgress { received, total },
            ));
            if state.reborn_cancel.load(Ordering::Acquire) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .await
        .map_err(|error| error.to_string())?;
    let executables =
        reborn::inspect_package(&package, &bytes).map_err(|error| error.to_string())?;
    platform::engine::install_reborn(
        &installation.root,
        &package,
        &executables,
        platform::engine::retail_activity(),
    )
    .map_err(|error| error.to_string())?;
    Ok(RebornInstallResult {
        engine: EngineChoice::Reborn,
        inventory: platform::engine::inventory(&installation.root),
    })
}

#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri managed state parameter"
)]
fn cancel_reborn_install(state: tauri::State<'_, AppState>) {
    state.reborn_cancel.store(true, Ordering::Release);
}

#[tauri::command]
async fn openmohaa_status(
    path: String,
    channel: ReleaseChannel,
    state: tauri::State<'_, AppState>,
) -> Result<OpenMohaaStatus, OpenMohaaFailure> {
    let installation = install::identify(&path).map_err(|error| OpenMohaaFailure::other(&error))?;
    let target = match ReleaseTarget::for_host() {
        Ok(target) => target,
        Err(unsupported) => {
            return Ok(OpenMohaaStatus::Unsupported {
                os: unsupported.os,
                architecture: unsupported.architecture,
            });
        }
    };
    let client = OpenMohaaReleaseClient::new(Duration::from_secs(120))?;
    let package = client.release(ReleaseSelector { channel, target }).await?;
    let offer_id = cache_openmohaa_offer(&state, &installation, target, package.clone())?;
    Ok(OpenMohaaStatus::Available {
        target,
        installed_build: installed_openmohaa_build(&installation.root, target, &package),
        activity: activity_summary(&platform::openmohaa_activity()),
        package: release_summary(&package, offer_id),
    })
}

#[tauri::command]
async fn install_openmohaa(
    path: String,
    offer_id: OpenMohaaOfferId,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<OpenMohaaInstallResult, OpenMohaaFailure> {
    let _install_guard = state
        .openmohaa_install
        .try_lock()
        .map_err(|_| OpenMohaaFailure::other(&"an OpenMoHAA install is already running"))?;
    state.openmohaa_cancel.store(false, Ordering::Release);

    // Re-identification prevents a stale or forged frontend path from becoming an arbitrary
    // archive extraction destination.
    let installation = install::identify(&path).map_err(|error| OpenMohaaFailure::other(&error))?;
    let target =
        ReleaseTarget::for_host().map_err(|error| OpenMohaaFailure::no_asset_for_host(&error))?;
    let offer = cached_openmohaa_offer(&state, offer_id)?;
    if offer.installation_root != installation.root || offer.target != target {
        return Err(OpenMohaaFailure::other(
            &"the displayed OpenMoHAA offer does not belong to this game folder",
        ));
    }
    let client = OpenMohaaReleaseClient::new(Duration::from_secs(120))?;
    let summary = release_summary(&offer.package, offer.id);
    let cancel = &state.openmohaa_cancel;
    let observed_activity = Mutex::new(platform::OpenMohaaActivity::unknown());
    let outcome = client
        .download_and_install_reporting(
            &offer.package,
            &installation.root,
            // Probed after the transfer, not before it: the archive takes long enough to
            // download that a player can start the client in between.
            || {
                let activity = platform::openmohaa_activity();
                let client_activity = activity.client_activity();
                if let Ok(mut observed) = observed_activity.lock() {
                    *observed = activity;
                }
                client_activity
            },
            |ReleaseDownloadProgress { received, total }| {
                drop(app.emit(
                    OPENMOHAA_INSTALL_EVENT,
                    OpenMohaaInstallProgress { received, total },
                ));
                if cancel.load(Ordering::Acquire) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .await?;
    let installed_build = if matches!(
        outcome,
        UpdateOutcome::Installed { .. } | UpdateOutcome::Updated { .. }
    ) {
        // The engine install succeeded even if this evidence record cannot be written. Without a
        // valid receipt the result and next status check honestly fall back to Version unknown.
        if record_openmohaa_install(&installation.root, target, &offer.package).is_ok() {
            OpenMohaaInstalledBuild::Current
        } else {
            OpenMohaaInstalledBuild::Unknown
        }
    } else {
        installed_openmohaa_build(&installation.root, target, &offer.package)
    };
    let activity = match observed_activity.into_inner() {
        Ok(activity) => activity,
        Err(error) => error.into_inner(),
    };
    Ok(OpenMohaaInstallResult {
        package: summary,
        outcome,
        activity: activity_summary(&activity),
        installed_build,
    })
}

#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri resolves managed state only for by-value command parameters"
)]
fn cancel_openmohaa_install(state: tauri::State<'_, AppState>) {
    state.openmohaa_cancel.store(true, Ordering::Release);
}

fn cache_openmohaa_offer(
    state: &AppState,
    installation: &Installation,
    target: ReleaseTarget,
    package: ReleasePackage,
) -> Result<OpenMohaaOfferId, OpenMohaaFailure> {
    let id = OpenMohaaOfferId(state.openmohaa_next_offer.fetch_add(1, Ordering::Relaxed));
    let mut offers = state
        .openmohaa_offers
        .lock()
        .map_err(|error| OpenMohaaFailure::other(&error))?;
    if offers.len() == OPENMOHAA_OFFER_CACHE_CAPACITY {
        offers.pop_front();
    }
    offers.push_back(CachedOpenMohaaOffer {
        id,
        installation_root: installation.root.clone(),
        target,
        package,
    });
    Ok(id)
}

fn cached_openmohaa_offer(
    state: &AppState,
    id: OpenMohaaOfferId,
) -> Result<CachedOpenMohaaOffer, OpenMohaaFailure> {
    state
        .openmohaa_offers
        .lock()
        .map_err(|error| OpenMohaaFailure::other(&error))?
        .iter()
        .find(|offer| offer.id == id)
        .cloned()
        .ok_or_else(|| {
            OpenMohaaFailure::other(
                &"the displayed OpenMoHAA offer expired; refresh before installing",
            )
        })
}

fn release_summary(
    package: &ReleasePackage,
    offer_id: OpenMohaaOfferId,
) -> OpenMohaaReleaseSummary {
    OpenMohaaReleaseSummary {
        offer_id,
        channel: package.channel,
        version: package.version.clone(),
        asset_name: package.asset_name.clone(),
        size: package.size,
        digest: package.digest.to_string(),
    }
}

fn activity_summary(activity: &platform::OpenMohaaActivity) -> OpenMohaaActivitySummary {
    let running = activity
        .running_programs()
        .iter()
        .map(|program| match program {
            platform::OpenMohaaProgram::Game => OpenMohaaRunningProgram::Game,
            platform::OpenMohaaProgram::DedicatedServer => OpenMohaaRunningProgram::DedicatedServer,
            platform::OpenMohaaProgram::Launcher => OpenMohaaRunningProgram::Launcher,
        })
        .collect();
    OpenMohaaActivitySummary {
        state: activity.client_activity(),
        running,
    }
}

fn installed_openmohaa_build(
    root: &Path,
    target: ReleaseTarget,
    selected: &ReleasePackage,
) -> OpenMohaaInstalledBuild {
    let client_path = openmohaa_client_path(root, target);
    if !client_path.is_file() {
        return OpenMohaaInstalledBuild::Absent;
    }
    let Some(receipt) = validated_openmohaa_receipt(root, &client_path) else {
        return OpenMohaaInstalledBuild::Unknown;
    };
    if receipt.channel == selected.channel
        && receipt.version == selected.version
        && receipt.asset_name == selected.asset_name
        && receipt.release_digest == selected.digest.to_string()
    {
        OpenMohaaInstalledBuild::Current
    } else {
        OpenMohaaInstalledBuild::KnownOther {
            channel: receipt.channel,
            version: receipt.version,
        }
    }
}

fn validated_openmohaa_receipt(root: &Path, client_path: &Path) -> Option<OpenMohaaInstallReceipt> {
    let receipt_path = root.join(OPENMOHAA_RECEIPT_FILENAME);
    let bytes = fs::read(&receipt_path).ok()?;
    let receipt = serde_json::from_slice::<OpenMohaaInstallReceipt>(&bytes).ok()?;
    if receipt.format != OPENMOHAA_RECEIPT_FORMAT {
        return None;
    }
    let client_sha256 = sha256_file(client_path).ok()?;
    (receipt.client_sha256 == client_sha256).then_some(receipt)
}

fn record_openmohaa_install(
    root: &Path,
    target: ReleaseTarget,
    package: &ReleasePackage,
) -> Result<(), OpenMohaaReceiptError> {
    let client_path = openmohaa_client_path(root, target);
    let receipt = OpenMohaaInstallReceipt {
        format: OPENMOHAA_RECEIPT_FORMAT,
        channel: package.channel,
        version: package.version.clone(),
        asset_name: package.asset_name.clone(),
        release_digest: package.digest.to_string(),
        client_sha256: sha256_file(&client_path)?,
    };
    let encoded = serde_json::to_vec_pretty(&receipt)?;
    let receipt_path = root.join(OPENMOHAA_RECEIPT_FILENAME);
    fs::write(&receipt_path, encoded).map_err(|source| OpenMohaaReceiptError::Filesystem {
        path: receipt_path,
        source,
    })
}

fn sha256_file(path: &Path) -> Result<String, OpenMohaaReceiptError> {
    let mut file = fs::File::open(path).map_err(|source| OpenMohaaReceiptError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| OpenMohaaReceiptError::Filesystem {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn openmohaa_client_path(root: &Path, target: ReleaseTarget) -> PathBuf {
    // openmoh/openmohaa v0.82.1 release archive layout: Windows uses `openmohaa.exe`; every Unix
    // archive uses the extensionless `openmohaa` binary at its root.
    let filename = match target {
        ReleaseTarget::WindowsX64 | ReleaseTarget::WindowsX86 | ReleaseTarget::WindowsArm64 => {
            "openmohaa.exe"
        }
        ReleaseTarget::LinuxAmd64
        | ReleaseTarget::LinuxArm64
        | ReleaseTarget::LinuxArmhf
        | ReleaseTarget::LinuxI686
        | ReleaseTarget::MacosArm64
        | ReleaseTarget::MacosX64 => "openmohaa",
    };
    root.join(filename)
}

/// Open the platform folder picker. `None` means the player dismissed it.
#[tauri::command]
async fn pick_install_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select your Allied Assault game folder")
        .pick_folder(move |folder| {
            // The receiver is only gone if the window closed while the dialog was open.
            drop(sender.send(folder));
        });
    let folder = receiver
        .await
        .map_err(|_| "the folder picker closed unexpectedly".to_owned())?;
    Ok(folder.map(|folder| folder.to_string()))
}

/// Stop the sweep currently running, if any. Servers already probed are kept.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri resolves managed state only for by-value command parameters"
)]
fn cancel_browse(state: tauri::State<'_, AppState>) {
    state.cancel_browse.notify_one();
}

#[tauri::command]
async fn browse_servers(
    session: Session,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<BrowserPayload, String> {
    let (_, index) = installed_maps(&session)?;

    // A stop pressed just as the previous sweep ended leaves a permit behind, which would cancel
    // this one before it probed anything. Consume it: polling `notified` once resolves immediately
    // when a permit is stored and times out otherwise.
    drop(tokio::time::timeout(Duration::ZERO, state.cancel_browse.notified()).await);
    // Rows are offered to the player as they stream, so a join prepared mid-sweep must be able to
    // find its server. The list is rebuilt from the authoritative report when the sweep ends.
    state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())?
        .clear();

    let (sink, mut events) = mpsc::channel(64);
    let sweep = tokio::spawn(discovery::browse_streaming(
        BrowseConfig {
            target: session.game,
            limit: None,
            concurrency: 16,
            master_timeout: Duration::from_secs(15),
            probe_timeout: PROBE_TIMEOUT,
        },
        sink,
    ));

    let cancelled = stream_sweep(&app, &state, &index, &mut events).await?;
    // Dropping the receiver is what stops the sweep. It returns what it already inspected.
    drop(events);

    let report = sweep
        .await
        .map_err(|error| format!("the server sweep did not finish: {error}"))?
        .map_err(|error| error.to_string())?;
    let servers = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.server.clone())
        .collect::<Vec<_>>();
    let mut rows = servers
        .iter()
        .map(|server| classified(server, &index))
        .collect::<Vec<_>>();
    // Ordered by the only quantity a server actually reports about its population.
    rows.sort_by_key(|row| {
        std::cmp::Reverse(
            row.server
                .occupancy
                .clients_reported
                .map_or(0, discovery::ClientsReported::get),
        )
    });
    *state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())? = servers;

    Ok(BrowserPayload {
        servers: rows,
        summary: report.summary(),
        non_results: group_non_results(
            report
                .outcomes
                .iter()
                .filter_map(|outcome| outcome.non_result.as_ref()),
        ),
        cancelled,
    })
}

/// Relay sweep events to the frontend until the sweep ends or the player stops it.
///
/// Answered servers land in the shared list as they arrive, because a player can select a row while
/// the sweep is still running and preparing that join has to be able to find the server.
///
/// Returns whether the sweep was stopped early.
async fn stream_sweep(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    index: &MapIndex,
    events: &mut mpsc::Receiver<BrowseEvent>,
) -> Result<bool, String> {
    let mut progress = BrowseProgress {
        registered: 0,
        inspected: 0,
        probed: 0,
        answered: 0,
        non_results: 0,
        row: None,
    };
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            () = state.cancel_browse.notified() => return Ok(true),
        };
        let Some(event) = event else {
            return Ok(false);
        };
        match event {
            BrowseEvent::Registered {
                registered,
                inspected,
            } => {
                progress.registered = registered;
                progress.inspected = inspected;
                progress.row = None;
            }
            BrowseEvent::Outcome(outcome) => {
                progress.probed += 1;
                progress.row = outcome
                    .server
                    .as_ref()
                    .map(|server| classified(server, index));
                if let Some(server) = outcome.server {
                    progress.answered += 1;
                    state
                        .servers
                        .lock()
                        .map_err(|_| "server list state is unavailable".to_owned())?
                        .push(server);
                } else {
                    progress.non_results += 1;
                }
            }
        }
        // A frontend that stopped listening is not an error; the sweep result is still worth having.
        drop(app.emit(BROWSE_EVENT, progress.clone()));
    }
}

/// Check one remembered server directly, with no master list involved.
///
/// A favourite is often not in the current sweep — the master never registered it, or it did not
/// answer in time. Without this the bookmark would be a dead row: `install_and_launch` resolves
/// its target out of the sweep's list, so a server missing from that list cannot be joined at all.
/// A server that answers here is merged into the same list and becomes joinable like any other.
#[tauri::command]
async fn check_server(
    session: Session,
    address: String,
    query_port: u16,
    state: tauri::State<'_, AppState>,
) -> Result<CheckResult, String> {
    let address = address
        .parse::<SocketAddrV4>()
        .map_err(|error| format!("invalid server address: {error}"))?;
    let (_, index) = installed_maps(&session)?;
    let endpoint = MasterEndpoint {
        address: *address.ip(),
        query_port: QueryPort::new(query_port),
    };
    let outcome = discovery::inspect_endpoint(endpoint, PROBE_TIMEOUT).await;

    let Some(server) = outcome.server else {
        // Not an error. Why it did not answer is what the player asked for.
        //
        // The entry goes with it. This list is what `find_server` prepares a join from, and a check
        // that ran and got no answer is evidence about now that outranks whatever the sweep saw —
        // the same reason the shell drops the row (docs/rules.md H12). Leaving it would keep a join
        // preparable from figures the interface has already withdrawn.
        forget_checked_server(&state, address)?;
        return Ok(CheckResult {
            row: None,
            non_result: outcome
                .non_result
                .as_ref()
                .and_then(|non_result| group_non_results(std::iter::once(non_result)).pop()),
            other_game: None,
        });
    };
    if let Some(published) = answered_for_another_game(&server, session.game) {
        // It answered, for a game this session's client cannot join. Not a joinable entry either.
        forget_checked_server(&state, address)?;
        return Ok(CheckResult {
            row: None,
            non_result: None,
            other_game: Some(published),
        });
    }
    // The server publishes its own `hostport`, so a server that moved answers at an address other
    // than the remembered one. The row carries the address it actually answered at; repointing the
    // bookmark at it would be a guess about whether it is the same server.
    let row = classified(&server, &index);
    let mut servers = state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())?;
    merge_checked_server(&mut servers, server);
    drop(servers);
    Ok(CheckResult {
        row: Some(row),
        non_result: None,
        other_game: None,
    })
}

/// The family a checked server belongs to, when it is not this session's.
///
/// A bookmark is an address, so it outlives the game it was starred under. A server that answers
/// for another family is real and reachable and still cannot be joined from this session: the
/// client this session launches speaks a different protocol and would be dropped at connect. A
/// server that publishes no family at all is not guessed about — it is listed, exactly as the
/// sweep would have listed it.
fn answered_for_another_game(server: &Server, game: TargetGame) -> Option<TargetGame> {
    server
        .game_name
        .as_deref()
        .and_then(TargetGame::from_game_name)
        .filter(|published| *published != game)
}

/// Merge a freshly checked server into the current list, replacing any entry for the same game
/// endpoint.
///
/// Appending would leave `find_server` resolving whichever copy it reached first, so a join could
/// be prepared from figures this check has already superseded.
fn merge_checked_server(servers: &mut Vec<Server>, server: Server) {
    let endpoint = (server.endpoint.address, server.game_port);
    servers.retain(|existing| (existing.endpoint.address, existing.game_port) != endpoint);
    servers.push(server);
}

/// Drop the entry for a game endpoint a check has just found nothing at.
///
/// Deliberately keyed on the game address the check was asked about, not on the query port: the
/// caller asked about one join target and learned that it is not there.
fn forget_checked_server(
    state: &tauri::State<'_, AppState>,
    address: SocketAddrV4,
) -> Result<(), String> {
    let mut servers = state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())?;
    servers.retain(|existing| {
        (existing.endpoint.address, existing.game_port.get()) != (*address.ip(), address.port())
    });
    Ok(())
}

/// What every server-facing command needs before it can say anything: which game folder, which
/// engine program, and which of the three games.
///
/// One struct rather than three repeated parameters, so a command cannot be given the folder and
/// the engine and quietly left with the wrong game.
#[derive(Clone, Deserialize)]
struct Session {
    path: String,
    engine: EngineChoice,
    game: TargetGame,
}

/// Resolve the installation, confirm the engine it will run, and index the maps on disk.
///
/// Every command that classifies a server needs these in this order: a game the install has no
/// assets for and an unresolvable engine choice must both fail before a directory is probed for
/// writability, and the index has to come from every directory that engine actually reads —
/// which for an expansion is `main` underneath `mainta` or `maintt`, not the expansion alone.
fn installed_maps(session: &Session) -> Result<(platform::InstallTarget, MapIndex), String> {
    let install = install_destination(session)?;
    let index = reindex(session, &install)?;
    Ok((install, index))
}

/// Resolve where downloaded content goes for this session, and nothing else.
///
/// Split out because a join must resolve it **once**: the probe can legitimately answer
/// differently a second time — a folder that was locked when the preview ran may be writable when
/// the install finishes — and reporting the second answer would name a directory the files were
/// never written to (rule H8).
fn install_destination(session: &Session) -> Result<platform::InstallTarget, String> {
    let installation = install::identify(&session.path).map_err(|error| error.to_string())?;
    if !installation.provides(session.game) {
        // The directory name is what the check actually looked at, and it is exactly the detail a
        // newcomer cannot act on. Say which game is missing from the folder they picked.
        return Err(format!(
            "{} cannot be run from this game folder: its game files are not there.",
            session.game.label()
        ));
    }
    platform::engine::resolve_choice(&installation.root, Some(session.engine))
        .map_err(|error| error.to_string())?;
    platform::resolve_install_target(
        &installation.root,
        LaunchProfile::new(session.game).data_directory(),
        platform::ClientKind::from(session.engine),
    )
    .map_err(|error| error.to_string())
}

/// Index every directory this session's engine reads, around an already-resolved destination.
///
/// # Errors
///
/// Returns an error when the installation cannot be identified or a directory cannot be read.
fn reindex(session: &Session, install: &platform::InstallTarget) -> Result<MapIndex, String> {
    let installation = install::identify(&session.path).map_err(|error| error.to_string())?;
    let search = search_path(
        &installation.root,
        session.game,
        platform::ClientKind::from(session.engine),
        install,
    );
    MapIndex::scan_chain(&search).map_err(|error| error.to_string())
}

/// The engine's search path, with the directory Reveille writes to guaranteed to be in it.
///
/// `content_search_path` lists only directories that exist, and the home fallback is created the
/// moment it is chosen — so on the first fallback install the destination would otherwise be
/// absent from the index that decides whether the download worked.
fn search_path(
    install_root: &Path,
    game: TargetGame,
    client: platform::ClientKind,
    install: &platform::InstallTarget,
) -> Vec<PathBuf> {
    let mut search = platform::content_search_path(install_root, game, client);
    if !search.contains(&install.game_directory) {
        search.push(install.game_directory.clone());
    }
    search
}

fn classified(server: &Server, index: &MapIndex) -> BrowserServer {
    BrowserServer {
        address: SocketAddrV4::new(server.endpoint.address, server.game_port.get()),
        compatibility: reveille_core::join::classify_server(index, server, None),
        server: server.clone(),
    }
}

fn group_non_results<'a>(non_results: impl Iterator<Item = &'a NonResult>) -> Vec<NonResultGroup> {
    let mut groups: Vec<NonResultGroup> = Vec::new();
    for non_result in non_results {
        let (reason, detail) = describe_non_result(&non_result.reason);
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.stage == non_result.stage && group.reason == reason)
        {
            group.count += 1;
            continue;
        }
        groups.push(NonResultGroup {
            stage: non_result.stage,
            reason,
            detail,
            count: 1,
        });
    }
    groups.sort_by_key(|group| std::cmp::Reverse(group.count));
    groups
}

fn describe_non_result(reason: &NonResultReason) -> (&'static str, Option<String>) {
    match reason {
        NonResultReason::Timeout => ("timeout", None),
        NonResultReason::Network { message } => ("network", Some(message.clone())),
        NonResultReason::Malformed { message } => ("malformed", Some(message.clone())),
        NonResultReason::MissingHostPort => ("missing_host_port", None),
        NonResultReason::DuplicateEndpoint { game_port } => {
            ("duplicate_endpoint", Some(game_port.get().to_string()))
        }
    }
}

#[tauri::command]
async fn preview_join(
    session: Session,
    address: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<JoinPreview, String> {
    let server = find_server(&state, &address)?;
    let preview = build_preview(&session, server, Some(&app)).await?;
    if let Ok(mut cache) = state.preview.lock() {
        *cache = Some(CachedPreview {
            install_root: PathBuf::from(&session.path),
            engine: session.engine,
            game: session.game,
            preview: preview.clone(),
        });
    }
    Ok(preview)
}

#[tauri::command]
async fn install_and_launch(
    session: Session,
    address: String,
    selected_candidate_ids: Vec<u64>,
    accept_incomplete: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<JoinResult, String> {
    let server = find_server(&state, &address)?;
    let preview = match take_cached_preview(&state, &session, &address) {
        Some(preview) => preview,
        None => build_preview(&session, server.clone(), Some(&app)).await?,
    };
    let (installed, failures) = match &preview.catalogue {
        None => (Vec::new(), Vec::new()),
        Some(catalogue) => {
            install_shopping_list(
                catalogue,
                &selected_candidate_ids.into_iter().collect::<HashSet<_>>(),
                &server,
                &preview.game_directory,
                &app,
            )
            .await?
        }
    };
    // Re-index the whole search path, not just the directory written to: the gate below asks
    // whether the engine can now find the map, and the engine reads all of it. The destination is
    // the preview's, not a fresh probe — the files went where the download put them, and that is
    // what gets reported (H8).
    let install_target = platform::InstallTarget {
        game_directory: preview.game_directory.clone(),
        used_home_fallback: preview.used_home_fallback,
    };
    let index = reindex(&session, &install_target)?;
    let assessment =
        reveille_core::join::classify_server(&index, &server, preview.catalogue.as_ref());
    let outcome = if let Some(reason) = launch_refusal(&assessment, accept_incomplete) {
        LaunchOutcome::Refused { reason }
    } else {
        launch(&session, preview.address)?
    };
    Ok(JoinResult {
        assessment,
        installed,
        failures,
        game_directory: install_target.game_directory,
        used_home_fallback: install_target.used_home_fallback,
        engine: session.engine,
        game: session.game,
        outcome,
    })
}

/// Download, confirm and install every candidate the player's selection resolves to.
///
/// A failure is recorded against its map and the pass continues; one unobtainable archive never
/// abandons the rest of the shopping list.
async fn install_shopping_list(
    catalogue: &CatalogueResolutionPass,
    selected: &HashSet<u64>,
    server: &Server,
    game_directory: &Path,
    app: &tauri::AppHandle,
) -> Result<(Vec<PathBuf>, Vec<InstallFailure>), String> {
    let client =
        content::MohDbClient::new(Duration::from_secs(30)).map_err(|error| error.to_string())?;
    let staging = tempfile::TempDir::new().map_err(|error| error.to_string())?;
    let mut installed = Vec::new();
    let mut failures = Vec::new();
    let mut filenames = HashSet::new();
    let planned = catalogue
        .resolutions
        .iter()
        .filter(|resolution| candidate_for_resolution(&resolution.outcome, selected).is_some())
        .count();
    let mut position = 0;
    for resolution in &catalogue.resolutions {
        let Some(candidate) = candidate_for_resolution(&resolution.outcome, selected) else {
            continue;
        };
        if !filenames.insert(candidate.filename.clone()) {
            continue;
        }
        let progress = InstallProgress {
            map: resolution.wanted.name.clone(),
            filename: candidate.filename.clone(),
            index: position,
            of: planned,
            phase: InstallPhase::Downloading {
                received: 0,
                total: Some(candidate.file_size.get()),
            },
        };
        position += 1;
        match install_candidate(
            &client,
            candidate,
            &resolution.wanted,
            server,
            staging.path(),
            game_directory,
            app.clone(),
            &progress,
        )
        .await
        {
            Ok(path) => {
                emit_install(app, &progress, InstallPhase::Installed);
                installed.push(path);
            }
            Err(reason) => {
                emit_install(
                    app,
                    &progress,
                    InstallPhase::Failed {
                        reason: reason.clone(),
                    },
                );
                failures.push(InstallFailure {
                    map: resolution.wanted.name.clone(),
                    reason,
                });
            }
        }
    }
    failures.extend(catalogue.non_results.iter().map(|result| InstallFailure {
        map: result.wanted.name.clone(),
        reason: format!("the catalogue lookup did not complete: {:?}", result.reason),
    }));
    Ok((installed, failures))
}

/// Start the client the detected install actually provides, connected to `address`.
fn launch(session: &Session, address: SocketAddrV4) -> Result<LaunchOutcome, String> {
    let installation = install::identify(&session.path).map_err(|error| error.to_string())?;
    platform::engine::resolve_choice(&installation.root, Some(session.engine))
        .map_err(|error| error.to_string())?;
    let kind = platform::ClientKind::from(session.engine);
    let profile = LaunchProfile::new(session.game);
    let program = platform::default_client(&installation.root, profile.target, kind)
        .to_string_lossy()
        .into_owned();
    let command = LaunchCommand::new(
        program,
        profile,
        FsGame::new("").map_err(|error| error.to_string())?,
        address,
    )
    .map_err(|error| error.to_string())?;
    Ok(LaunchOutcome::Launched {
        process_id: platform::launch_client(&command, kind)
            .map_err(|error| error.to_string())?
            .id(),
    })
}

/// Decide whether the launch may proceed, and say why when it may not.
///
/// `Compatible` needs no consent. Anything else needs the player to have accepted an incomplete
/// check — but consent cannot override the one fact that makes a join pointless: the map running
/// right now being absent, which drops the connection immediately. An unobtainable map later in
/// the rotation is not that. It costs one disconnect at one map change, and refusing the join over
/// it would invent a problem the engine does not have.
fn launch_refusal(assessment: &CompatibilityAssessment, accept_incomplete: bool) -> Option<String> {
    if matches!(assessment.state, CompatibilityState::Compatible) {
        return None;
    }
    if matches!(assessment.current_map, CurrentMapReadiness::Missing) {
        return Some(
            "The map this server is running right now is not on disk, so the join would be dropped immediately."
                .to_owned(),
        );
    }
    if accept_incomplete {
        return None;
    }
    Some("This join has not been fully checked and was not confirmed.".to_owned())
}

fn take_cached_preview(
    state: &tauri::State<'_, AppState>,
    session: &Session,
    address: &str,
) -> Option<JoinPreview> {
    let mut cache = state.preview.lock().ok()?;
    let usable = cache.as_ref().is_some_and(|cached| {
        cached.game == session.game
            && preview_cache_matches(
                &cached.install_root,
                cached.preview.address,
                cached.engine,
                Path::new(&session.path),
                address,
                session.engine,
            )
    });
    if !usable {
        return None;
    }
    cache.take().map(|cached| cached.preview)
}

fn preview_cache_matches(
    cached_root: &Path,
    cached_address: SocketAddrV4,
    cached_engine: EngineChoice,
    requested_root: &Path,
    requested_address: &str,
    requested_engine: EngineChoice,
) -> bool {
    cached_root == requested_root
        && cached_address.to_string() == requested_address
        && cached_engine == requested_engine
}

fn find_server(state: &tauri::State<'_, AppState>, address: &str) -> Result<Server, String> {
    let address = address
        .parse::<SocketAddrV4>()
        .map_err(|error| format!("invalid server address: {error}"))?;
    state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())?
        .iter()
        .find(|server| {
            SocketAddrV4::new(server.endpoint.address, server.game_port.get()) == address
        })
        .cloned()
        .ok_or_else(|| {
            "This server is no longer in the current list. Refresh and try again.".to_owned()
        })
}

async fn build_preview(
    session: &Session,
    server: Server,
    app: Option<&tauri::AppHandle>,
) -> Result<JoinPreview, String> {
    let (install_target, index) = installed_maps(session)?;
    let first = reveille_core::join::classify_server(&index, &server, None);
    let wanted = first.preflight.as_ref().map_or_else(Vec::new, wanted_maps);
    let address = SocketAddrV4::new(server.endpoint.address, server.game_port.get());
    let catalogue = if wanted.is_empty() {
        None
    } else {
        let client = content::MohDbClient::new(Duration::from_secs(15))
            .map_err(|error| error.to_string())?;
        Some(
            client
                .resolve_all_reporting(&wanted, |progress| {
                    let Some(app) = app else {
                        return;
                    };
                    let map = match progress.resolved {
                        Ok(resolution) => resolution.wanted.name.clone(),
                        Err(non_result) => non_result.wanted.name.clone(),
                    };
                    drop(app.emit(
                        PREVIEW_EVENT,
                        PreviewProgress {
                            address,
                            index: progress.index,
                            of: progress.of,
                            map,
                        },
                    ));
                })
                .await,
        )
    };
    let assessment = reveille_core::join::classify_server(&index, &server, catalogue.as_ref());
    Ok(JoinPreview {
        address,
        server,
        assessment,
        catalogue,
        game_directory: install_target.game_directory,
        used_home_fallback: install_target.used_home_fallback,
        engine: session.engine,
        game: session.game,
    })
}

fn wanted_maps(preflight: &reveille_core::preflight::Report) -> Vec<WantedMap> {
    preflight
        .maps
        .iter()
        .filter(|map| {
            matches!(
                map.status,
                reveille_core::preflight::MapStatus::Absent
                    | reveille_core::preflight::MapStatus::ChecksumDiffers { .. }
            )
        })
        .filter_map(|map| WantedMap::new(map.map.clone()))
        .collect()
}

fn candidate_for_resolution<'a>(
    outcome: &'a ResolutionOutcome,
    selected: &HashSet<u64>,
) -> Option<&'a CatalogueCandidate> {
    match outcome {
        ResolutionOutcome::Exact { name_match, .. } => Some(name_match),
        ResolutionOutcome::ChoiceRequired { choices } => choices
            .iter()
            .find(|candidate| selected.contains(&candidate.id)),
        ResolutionOutcome::NoSource => None,
    }
}

fn emit_install(app: &tauri::AppHandle, progress: &InstallProgress, phase: InstallPhase) {
    drop(app.emit(
        INSTALL_EVENT,
        InstallProgress {
            map: progress.map.clone(),
            filename: progress.filename.clone(),
            index: progress.index,
            of: progress.of,
            phase,
        },
    ));
}

#[expect(
    clippy::too_many_arguments,
    reason = "one download: what to fetch, what it is for, where it lands, and where to report it"
)]
async fn install_candidate(
    client: &content::MohDbClient,
    candidate: &CatalogueCandidate,
    wanted: &WantedMap,
    server: &Server,
    staging: &Path,
    game_directory: &Path,
    app: tauri::AppHandle,
    progress: &InstallProgress,
) -> Result<PathBuf, String> {
    let mut announced = 0_u64;
    let archive = content::download_mohdb_archive_reporting(
        client,
        candidate,
        staging,
        |DownloadProgress { received, declared }| {
            if received != 0 && received < announced.saturating_add(DOWNLOAD_EVENT_STRIDE) {
                return;
            }
            announced = received;
            emit_install(
                &app,
                progress,
                InstallPhase::Downloading {
                    received,
                    total: declared.or_else(|| Some(candidate.file_size.get())),
                },
            );
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    emit_install(&app, progress, InstallPhase::Confirming);
    let inspection = content::inspect_archive(&archive.path).map_err(|error| error.to_string())?;
    let checksum = server
        .current_map
        .as_deref()
        .filter(|current| MapKey::new(current) == Some(wanted.key.clone()))
        .and(server.map_checksum);
    content::confirm_map(&inspection, &wanted.name, checksum).map_err(|error| error.to_string())?;
    content::install_archive(&archive, game_directory).map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_install,
            engine_overview,
            select_engine,
            install_reborn,
            cancel_reborn_install,
            openmohaa_status,
            install_openmohaa,
            cancel_openmohaa_install,
            pick_install_folder,
            cancel_browse,
            browse_servers,
            check_server,
            preview_join,
            install_and_launch
        ])
        .run(tauri::generate_context!())
        .expect("error while running Reveille");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use reveille_core::bsp::Checksum;
    use reveille_core::install::{IdentificationMethod, Product};
    use reveille_core::join::{
        CompatibilityAssessment, CompatibilityState, CurrentMapReadiness, MapsNeeded,
    };
    use reveille_core::platform::openmohaa::{
        OpenMohaaError, PublishedSha256, ReleaseChannel, ReleasePackage, ReleaseSelector,
        ReleaseTarget,
    };
    use reveille_core::preflight::{MapResult, MapStatus, Report, Verdict};
    use tempfile::TempDir;

    use super::{
        AppState, EngineChoice, MasterEndpoint, OpenMohaaFailure, OpenMohaaFailureKind,
        OpenMohaaInstalledBuild, QueryPort, Server, Session, TargetGame, answered_for_another_game,
        cache_openmohaa_offer, cached_openmohaa_offer, installed_maps, installed_openmohaa_build,
        launch_refusal, merge_checked_server, openmohaa_client_path, preview_cache_matches,
        record_openmohaa_install,
    };

    fn assessment(
        state: CompatibilityState,
        current_map: CurrentMapReadiness,
    ) -> CompatibilityAssessment {
        CompatibilityAssessment {
            state,
            preflight: Some(Report {
                verdict: Verdict::Compatible,
                maps: vec![MapResult {
                    map: "dm/mohdm6".to_owned(),
                    status: MapStatus::Present {
                        checksum: Checksum::new(1),
                        checksum_checked: false,
                    },
                }],
            }),
            current_map,
        }
    }

    #[test]
    fn the_session_payload_matches_what_the_shell_sends() {
        // `lib/api.js` sends `{ session: { path, engine, game } }`, and both enums travel as the
        // snake_case names the rest of the payloads already use. A rename on either side is a
        // silent "invalid args" on every command, so it is pinned here rather than found by hand.
        let session: Session = serde_json::from_str(
            r#"{"path":"D:\\Games\\MOHAA","engine":"openmohaa","game":"breakthrough"}"#,
        )
        .expect("the shell's session payload");

        assert_eq!(session.path, r"D:\Games\MOHAA");
        assert_eq!(session.engine, EngineChoice::Openmohaa);
        assert_eq!(session.game, TargetGame::Breakthrough);
    }

    #[test]
    fn a_game_the_folder_has_no_files_for_is_refused_before_anything_is_probed() {
        let temporary = TempDir::new().expect("temporary directory");
        fs::create_dir(temporary.path().join("main")).expect("main directory");
        fs::write(temporary.path().join("openmohaa.exe"), []).expect("client marker");
        let path = temporary.path().to_string_lossy().into_owned();

        // Allied Assault is what this folder has, and it indexes.
        installed_maps(&Session {
            path: path.clone(),
            engine: EngineChoice::Openmohaa,
            game: TargetGame::AlliedAssault,
        })
        .expect("the base game indexes");

        // Spearhead is not, and saying so beats an empty map index that would report every map
        // on the server as missing. The message names the game, never the engine's directory:
        // `mainta` is what the check looked at and is not something a player can act on.
        let refusal = installed_maps(&Session {
            path: path.clone(),
            engine: EngineChoice::Openmohaa,
            game: TargetGame::Spearhead,
        })
        .expect_err("an absent expansion is refused");
        assert!(refusal.contains("Spearhead"), "{refusal}");
        assert!(!refusal.contains("mainta"), "{refusal}");

        // "Before anything is probed" is the load-bearing half: this folder has no retail
        // executable either, so asking for Original as well proves which check runs first. A
        // writability probe or a home fallback must never happen for a game that was never
        // runnable from this folder.
        let refusal = installed_maps(&Session {
            path,
            engine: EngineChoice::Original,
            game: TargetGame::Spearhead,
        })
        .expect_err("an absent expansion is refused whatever the engine");
        assert!(refusal.contains("Spearhead"), "{refusal}");
    }

    #[test]
    fn a_release_without_a_published_file_check_is_not_reported_as_a_bad_download() {
        let digest = PublishedSha256::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("fixture digest");
        let cases = [
            (
                OpenMohaaError::MissingDigest("openmohaa.zip".to_owned()),
                OpenMohaaFailureKind::ReleaseMetadata,
            ),
            (
                OpenMohaaError::UnsupportedDigest("md5:00".to_owned()),
                OpenMohaaFailureKind::ReleaseMetadata,
            ),
            (
                OpenMohaaError::InvalidDigest("sha256:zz".to_owned()),
                OpenMohaaFailureKind::ReleaseMetadata,
            ),
            (
                OpenMohaaError::AmbiguousAsset(ReleaseSelector::stable(ReleaseTarget::WindowsX64)),
                OpenMohaaFailureKind::ReleaseMetadata,
            ),
            (
                OpenMohaaError::MissingAsset(ReleaseSelector::stable(ReleaseTarget::WindowsX64)),
                OpenMohaaFailureKind::NoAssetForHost,
            ),
            (
                OpenMohaaError::DigestMismatch {
                    expected: digest,
                    actual: digest,
                },
                OpenMohaaFailureKind::CorruptDownload,
            ),
            (
                OpenMohaaError::SizeMismatch {
                    expected: 1,
                    actual: 2,
                },
                OpenMohaaFailureKind::CorruptDownload,
            ),
            (
                OpenMohaaError::DownloadCancelled,
                OpenMohaaFailureKind::Cancelled,
            ),
            (
                OpenMohaaError::EmptyArchive,
                OpenMohaaFailureKind::ArchiveRejected,
            ),
        ];

        for (error, expected) in cases {
            let rendered = error.to_string();
            let failure = OpenMohaaFailure::from(error);
            assert_eq!(failure.kind, expected, "misclassified {rendered:?}");
            assert_eq!(failure.detail, rendered);
        }
    }

    #[test]
    fn openmohaa_client_path_matches_the_published_archive_layout() {
        let root = Path::new(r"C:\Games\MOHAA");
        for target in [
            ReleaseTarget::WindowsX64,
            ReleaseTarget::WindowsX86,
            ReleaseTarget::WindowsArm64,
        ] {
            assert_eq!(
                openmohaa_client_path(root, target),
                root.join("openmohaa.exe")
            );
        }
        for target in [
            ReleaseTarget::LinuxAmd64,
            ReleaseTarget::LinuxArm64,
            ReleaseTarget::LinuxArmhf,
            ReleaseTarget::LinuxI686,
            ReleaseTarget::MacosArm64,
            ReleaseTarget::MacosX64,
        ] {
            assert_eq!(openmohaa_client_path(root, target), root.join("openmohaa"));
        }
    }

    #[test]
    fn installation_reuses_the_exact_release_offer_that_status_returned() {
        let state = AppState::default();
        let installation = reveille_core::install::Installation {
            root: Path::new(r"C:\Games\MOHAA").to_path_buf(),
            products: vec![Product::AlliedAssault],
            playable: vec![Product::AlliedAssault],
            binaries: Vec::new(),
            identification: IdentificationMethod::DataDirectoriesOnly,
        };
        let stable = ReleasePackage {
            channel: ReleaseChannel::Stable,
            version: "v0.82.1".to_owned(),
            asset_name: "stable.zip".to_owned(),
            download_url: "https://example.invalid/stable.zip".to_owned(),
            size: 6,
            digest: PublishedSha256::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("stable digest"),
        };
        let preview = ReleasePackage {
            channel: ReleaseChannel::Dev,
            version: "preview commit".to_owned(),
            asset_name: "preview.zip".to_owned(),
            download_url: "https://example.invalid/preview.zip".to_owned(),
            size: 7,
            digest: PublishedSha256::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("preview digest"),
        };

        let stable_id = cache_openmohaa_offer(
            &state,
            &installation,
            ReleaseTarget::WindowsX64,
            stable.clone(),
        )
        .expect("cache stable offer");
        let preview_id =
            cache_openmohaa_offer(&state, &installation, ReleaseTarget::WindowsX64, preview)
                .expect("cache preview offer");

        assert_ne!(stable_id, preview_id);
        assert_eq!(
            cached_openmohaa_offer(&state, stable_id)
                .expect("displayed stable offer")
                .package,
            stable
        );
    }

    #[test]
    fn a_receipt_only_identifies_the_unchanged_client_and_exact_release() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::write(root.join("openmohaa.exe"), b"installed preview").expect("client fixture");
        let preview = ReleasePackage {
            channel: ReleaseChannel::Dev,
            version: "main-0123456789abcdef".to_owned(),
            asset_name: "openmohaa-dev-windows-x64-pdb.zip".to_owned(),
            download_url: "https://example.invalid/preview.zip".to_owned(),
            size: 7,
            digest: PublishedSha256::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("preview digest"),
        };

        assert_eq!(
            installed_openmohaa_build(root, ReleaseTarget::WindowsX64, &preview),
            OpenMohaaInstalledBuild::Unknown
        );
        record_openmohaa_install(root, ReleaseTarget::WindowsX64, &preview).expect("write receipt");
        assert_eq!(
            installed_openmohaa_build(root, ReleaseTarget::WindowsX64, &preview),
            OpenMohaaInstalledBuild::Current
        );

        let stable = ReleasePackage {
            channel: ReleaseChannel::Stable,
            version: "v0.82.1".to_owned(),
            asset_name: "openmohaa-v0.82.1-windows-x64.zip".to_owned(),
            download_url: "https://example.invalid/stable.zip".to_owned(),
            size: 6,
            digest: PublishedSha256::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("stable digest"),
        };
        assert_eq!(
            installed_openmohaa_build(root, ReleaseTarget::WindowsX64, &stable),
            OpenMohaaInstalledBuild::KnownOther {
                channel: ReleaseChannel::Dev,
                version: preview.version.clone(),
            }
        );

        fs::write(root.join("openmohaa.exe"), b"externally replaced")
            .expect("replace client fixture");
        assert_eq!(
            installed_openmohaa_build(root, ReleaseTarget::WindowsX64, &preview),
            OpenMohaaInstalledBuild::Unknown
        );
    }

    /// The minimum of a `Server` this test needs: the two fields that identify a game endpoint,
    /// plus a hostname to tell two answers apart.
    fn probed(address: &str, query_port: u16, game_port: u16, hostname: &str) -> Server {
        Server {
            endpoint: MasterEndpoint {
                address: address.parse().expect("address"),
                query_port: QueryPort::new(query_port),
            },
            game_port: reveille_core::discovery::GamePort::new(game_port),
            hostname: hostname.to_owned(),
            game_name: None,
            game_version: None,
            version: None,
            protocol: None,
            current_map: None,
            game_type: None,
            rotation: Vec::new(),
            allow_download: None,
            map_checksum: None,
            pr_downloads: None,
            minimum_ping: None,
            maximum_ping: None,
            join_window: None,
            reserved_slots: None,
            occupancy: reveille_core::discovery::ReportedOccupancy::default(),
            client_capacity: None,
            pure: None,
            status_round_trip: reveille_core::discovery::RoundTripMillis::new(12),
        }
    }

    #[test]
    fn a_checked_server_from_another_game_is_named_rather_than_listed() {
        let mut server = probed("10.0.0.1", 12300, 12203, "a Spearhead server");
        server.game_name = Some("mohaas".to_owned());

        // Browsing Spearhead, this is an ordinary row.
        assert_eq!(
            answered_for_another_game(&server, TargetGame::Spearhead),
            None
        );
        // Browsing Allied Assault, it answered — for something this session cannot join.
        assert_eq!(
            answered_for_another_game(&server, TargetGame::AlliedAssault),
            Some(TargetGame::Spearhead)
        );

        // A server that publishes no family is not guessed about. The sweep would have listed it,
        // and so does a check.
        server.game_name = None;
        assert_eq!(
            answered_for_another_game(&server, TargetGame::AlliedAssault),
            None
        );
        // Neither is one whose family is not a MOHAA family at all.
        server.game_name = Some("quake3".to_owned());
        assert_eq!(
            answered_for_another_game(&server, TargetGame::AlliedAssault),
            None
        );
    }

    #[test]
    fn the_shell_sends_the_session_to_every_command_that_needs_one() {
        // The Rust signatures and the JavaScript that calls them are one contract with two halves
        // in different languages, and no compiler spans both. A rename on either side is an
        // "invalid args" failure on every server-facing command, found by hand, at runtime.
        let api = include_str!("../ui/lib/api.js");

        for command in [
            "browse_servers",
            "check_server",
            "preview_join",
            "install_and_launch",
        ] {
            let call = format!(r#"invoke("{command}", {{ session"#);
            assert!(
                api.contains(&call),
                "ui/lib/api.js must pass `session` to {command}"
            );
        }
    }

    #[test]
    fn the_shell_sweeps_again_when_the_session_the_list_was_swept_for_changed() {
        // A text check, and it is what is available: the shell has no test runner, and the failure
        // it guards is invisible — the wrong game's servers under the right heading, with no error
        // and nothing on screen to contradict them (H12). The regression it catches is a real one
        // that shipped: `enterServers` swept only when the table was empty, so returning from setup
        // with a different game kept the list from the game just left.
        let app = include_str!("../ui/app.js");

        assert!(
            app.contains("next.listSession = swept;"),
            "ui/app.js: refresh must record the session its rows were swept for"
        );
        assert!(
            app.contains("if (!state.servers.length || !listIsForCurrentSession()) refresh();"),
            "ui/app.js: enterServers must sweep again when the list is for another session"
        );

        // The comparison has to cover all three: the game decides which servers exist, the folder
        // and the engine decide the search path their compatibility was judged against.
        let store = include_str!("../ui/lib/store.js");
        assert!(
            store.contains(
                "swept.path === now.path && swept.engine === now.engine && swept.game === now.game"
            ),
            "ui/lib/store.js: listIsForCurrentSession must compare path, engine and game"
        );
    }

    #[test]
    fn a_check_that_got_no_answer_drops_the_row_it_was_checking() {
        // The same kind of text check, for the same reason: the shell has no test runner, and the
        // failure is silent. A player presses "Check again" precisely to find out whether the
        // figures still hold, and a server that has stopped answering while keeping its client
        // count, map and round trip on screen answers that question with a lie (H12).
        let app = include_str!("../ui/app.js");

        assert!(
            app.contains(
                "next.servers = next.servers.filter((row) => row.address !== entry.address);"
            ),
            "ui/app.js: a check that returned no row must drop any live row for the address it asked"
        );
        assert!(
            app.contains("next.checkedAt.set(result.row.address, clockTime());"),
            "ui/app.js: a check that answered must record when it measured the row"
        );
    }

    #[test]
    fn folded_remembered_entries_always_state_their_count() {
        // The same text check as the two above, guarding H15. Favourites and History fold the
        // entries this check did not return behind a disclosure, and the one thing that keeps a
        // fold from being an invisible filter is that the count is drawn whether it is open or
        // shut. A regression here is silent: rows simply stop being there.
        let store = include_str!("../ui/lib/store.js");

        assert!(
            store.contains(r#"{ kind: "disclosure", address: `${absent.length}:${state.showAbsent}`, count: absent.length },"#),
            "ui/lib/store.js: the disclosure must be emitted with its count, whatever the open state"
        );
        assert!(
            store.contains("...(state.showAbsent"),
            "ui/lib/store.js: only the absent rows are conditional on the block being open"
        );

        let servers = include_str!("../ui/views/servers.js");
        assert!(
            servers.contains("`${count} ${savedNoun(count)} not in ${check}`"),
            "ui/views/servers.js: the disclosure must say how many entries it is folding away"
        );
        assert!(
            servers.contains(r#""aria-expanded": open ? "true" : "false","#),
            "ui/views/servers.js: the disclosure must publish its open state"
        );
    }

    #[test]
    fn a_check_carries_forward_what_it_knows_about_a_row_already_dropped() {
        // Found by review, and it made the one control on screen destroy the pane holding it: the
        // second "Check again" on a dropped server recomputed the remembered name from a list the
        // first check had already emptied, so the pane fell back to "No server selected".
        let app = include_str!("../ui/app.js");

        assert!(
            app.contains("(state.checks.get(entry.address)?.dropped ?? null)"),
            "ui/app.js: a check on a row already dropped must keep what the earlier check recorded"
        );
    }

    #[test]
    fn one_check_does_not_cancel_another() {
        // Also found by review. A single token bumped per call meant re-checking one server
        // abandoned the favourites batch mid-way and left the row it was probing reading
        // "Checking…" for a request nobody was waiting on. The token counts list generations —
        // a sweep and a game switch — not calls.
        let app = include_str!("../ui/app.js");

        assert!(
            app.contains("const generation = checkGeneration;"),
            "ui/app.js: check must capture the generation, not allocate a new token per call"
        );
        assert!(
            app.contains("  checkGeneration += 1;\n  const swept = session();"),
            "ui/app.js: a sweep must retire the checks still in flight against the old list"
        );
    }

    #[test]
    fn checking_a_server_replaces_its_entry_rather_than_adding_a_second() {
        let mut servers = vec![
            probed("10.0.0.1", 12300, 12203, "stale"),
            probed("10.0.0.2", 12300, 12203, "another server"),
        ];

        merge_checked_server(&mut servers, probed("10.0.0.1", 12300, 12203, "fresh"));

        assert_eq!(servers.len(), 2);
        // `find_server` takes the first match, so a stale copy left behind would be the one a join
        // is prepared from.
        assert!(servers.iter().all(|server| server.hostname != "stale"));
        assert!(servers.iter().any(|server| server.hostname == "fresh"));
        assert!(
            servers
                .iter()
                .any(|server| server.hostname == "another server")
        );
    }

    #[test]
    fn a_server_reregistered_under_a_new_query_port_leaves_no_duplicate_game_endpoint() {
        // The master can hand out a different query port for the same server. Identity is the
        // game endpoint, because that is what a join connects to.
        let mut servers = vec![probed("10.0.0.1", 12300, 12203, "stale")];

        merge_checked_server(&mut servers, probed("10.0.0.1", 12400, 12203, "fresh"));

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].hostname, "fresh");
    }

    #[test]
    fn a_server_that_moved_to_another_game_port_is_kept_alongside_the_old_entry() {
        // Different game endpoint, so it is a different join target. Collapsing the two would be
        // a guess that the server merely moved rather than that a second one exists.
        let mut servers = vec![probed("10.0.0.1", 12300, 12203, "old port")];

        merge_checked_server(&mut servers, probed("10.0.0.1", 12300, 12204, "new port"));

        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn compatible_launches_without_consent() {
        let ready = assessment(
            CompatibilityState::Compatible,
            CurrentMapReadiness::Playable,
        );

        assert_eq!(launch_refusal(&ready, false), None);
    }

    #[test]
    fn preview_cache_identity_includes_the_engine_choice() {
        let root = Path::new(r"C:\Games\MOHAA");
        let address = "127.0.0.1:12203".parse().expect("address");
        assert!(preview_cache_matches(
            root,
            address,
            reveille_core::engine::EngineChoice::Original,
            root,
            "127.0.0.1:12203",
            reveille_core::engine::EngineChoice::Original,
        ));
        assert!(!preview_cache_matches(
            root,
            address,
            reveille_core::engine::EngineChoice::Original,
            root,
            "127.0.0.1:12203",
            reveille_core::engine::EngineChoice::Reborn,
        ));
    }

    #[test]
    fn an_unobtainable_map_later_in_the_rotation_does_not_block_a_join() {
        // The server is running a map that is on disk. One map with no source further
        // along the rotation costs a disconnect at one map change, which is the player's
        // call to make — not a reason for the launcher to refuse.
        let no_source = assessment(
            CompatibilityState::NoSource {
                count: MapsNeeded::new(1),
            },
            CurrentMapReadiness::Playable,
        );

        assert_eq!(launch_refusal(&no_source, true), None);
        assert!(launch_refusal(&no_source, false).is_some());
    }

    #[test]
    fn consent_cannot_override_the_map_running_right_now_being_absent() {
        for state in [
            CompatibilityState::NoSource {
                count: MapsNeeded::new(1),
            },
            CompatibilityState::NeedsMaps {
                count: MapsNeeded::new(3),
                shopping_list: None,
            },
        ] {
            let dropped_on_arrival = assessment(state, CurrentMapReadiness::Missing);

            let refusal = launch_refusal(&dropped_on_arrival, true);

            assert!(refusal.is_some_and(|reason| reason.contains("right now")));
        }
    }

    #[test]
    fn an_unpublished_rotation_needs_explicit_consent() {
        let cant_tell = assessment(CompatibilityState::CantTell, CurrentMapReadiness::Unknown);

        assert!(launch_refusal(&cant_tell, false).is_some());
        assert_eq!(launch_refusal(&cant_tell, true), None);
    }
}
