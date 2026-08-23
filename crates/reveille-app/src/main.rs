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
    self, BrowseConfig, BrowseEvent, BrowseSummary, NonResult, NonResultReason, ProbeStage, Server,
    TargetGame,
};
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
    outcome: LaunchOutcome,
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
        .set_title("Select your Medal of Honor: Allied Assault folder")
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
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<BrowserPayload, String> {
    let installation = install::identify(&path).map_err(|error| error.to_string())?;
    let client = platform::detect_client(&installation.root);
    let target = platform::resolve_install_target(&installation.root, "main", client)
        .map_err(|error| error.to_string())?;
    let index = MapIndex::scan(&target.game_directory).map_err(|error| error.to_string())?;

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
            target: TargetGame::AlliedAssault,
            limit: None,
            concurrency: 16,
            master_timeout: Duration::from_secs(15),
            probe_timeout: Duration::from_millis(2_500),
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
    path: String,
    address: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<JoinPreview, String> {
    let server = find_server(&state, &address)?;
    let preview = build_preview(&path, server, Some(&app)).await?;
    if let Ok(mut cache) = state.preview.lock() {
        *cache = Some(CachedPreview {
            install_root: PathBuf::from(&path),
            preview: preview.clone(),
        });
    }
    Ok(preview)
}

#[tauri::command]
async fn install_and_launch(
    path: String,
    address: String,
    selected_candidate_ids: Vec<u64>,
    accept_incomplete: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<JoinResult, String> {
    let server = find_server(&state, &address)?;
    let preview = match take_cached_preview(&state, &path, &address) {
        Some(preview) => preview,
        None => build_preview(&path, server.clone(), Some(&app)).await?,
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
    let index = MapIndex::scan(&preview.game_directory).map_err(|error| error.to_string())?;
    let assessment =
        reveille_core::join::classify_server(&index, &server, preview.catalogue.as_ref());
    let outcome = if let Some(reason) = launch_refusal(&assessment, accept_incomplete) {
        LaunchOutcome::Refused { reason }
    } else {
        launch(&path, preview.address)?
    };
    Ok(JoinResult {
        assessment,
        installed,
        failures,
        game_directory: preview.game_directory,
        used_home_fallback: preview.used_home_fallback,
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
fn launch(path: &str, address: SocketAddrV4) -> Result<LaunchOutcome, String> {
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    let kind = platform::detect_client(&installation.root);
    let profile = LaunchProfile::new(TargetGame::AlliedAssault);
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
    path: &str,
    address: &str,
) -> Option<JoinPreview> {
    let mut cache = state.preview.lock().ok()?;
    let usable = cache.as_ref().is_some_and(|cached| {
        cached.install_root == Path::new(path) && cached.preview.address.to_string() == address
    });
    if !usable {
        return None;
    }
    cache.take().map(|cached| cached.preview)
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
    path: &str,
    server: Server,
    app: Option<&tauri::AppHandle>,
) -> Result<JoinPreview, String> {
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    let client_kind = platform::detect_client(&installation.root);
    let target = platform::resolve_install_target(&installation.root, "main", client_kind)
        .map_err(|error| error.to_string())?;
    let index = MapIndex::scan(&target.game_directory).map_err(|error| error.to_string())?;
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
        game_directory: target.game_directory,
        used_home_fallback: target.used_home_fallback,
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
            openmohaa_status,
            install_openmohaa,
            cancel_openmohaa_install,
            pick_install_folder,
            cancel_browse,
            browse_servers,
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
        AppState, OpenMohaaFailure, OpenMohaaFailureKind, OpenMohaaInstalledBuild,
        cache_openmohaa_offer, cached_openmohaa_offer, installed_openmohaa_build, launch_refusal,
        openmohaa_client_path, record_openmohaa_install,
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

    #[test]
    fn compatible_launches_without_consent() {
        let ready = assessment(
            CompatibilityState::Compatible,
            CurrentMapReadiness::Playable,
        );

        assert_eq!(launch_refusal(&ready, false), None);
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
