// SPDX-License-Identifier: GPL-2.0-only

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell. This layer owns presentation policy: it turns the pipeline's typed results into
//! payloads and progress events, and decides nothing the core has not already established.

use std::collections::HashSet;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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
use reveille_platform as platform;
use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::{Notify, mpsc, oneshot};

/// Events the frontend listens for, kept together so the contract reads in one place.
const BROWSE_EVENT: &str = "reveille://browse";
const PREVIEW_EVENT: &str = "reveille://preview";
const INSTALL_EVENT: &str = "reveille://install";

/// Bytes between download progress emissions. A 24 MB shopping list produces a few hundred events
/// rather than tens of thousands.
const DOWNLOAD_EVENT_STRIDE: u64 = 256 * 1024;

#[derive(Default)]
struct AppState {
    /// The servers behind the current list, keyed on by address when a join is prepared.
    servers: Mutex<Vec<Server>>,
    /// Raised to stop an in-flight sweep.
    cancel_browse: Notify,
    /// The most recent join preview, reused so a launch does not repeat the catalogue pass.
    preview: Mutex<Option<CachedPreview>>,
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
    use reveille_core::bsp::Checksum;
    use reveille_core::join::{
        CompatibilityAssessment, CompatibilityState, CurrentMapReadiness, MapsNeeded,
    };
    use reveille_core::preflight::{MapResult, MapStatus, Report, Verdict};

    use super::launch_refusal;

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
