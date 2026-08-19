// SPDX-License-Identifier: GPL-2.0-only

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use reveille_core::content::{
    self, CatalogueCandidate, CatalogueResolutionPass, ResolutionOutcome, WantedMap,
};
use reveille_core::discovery::{self, BrowseConfig, Server, TargetGame};
use reveille_core::install::{self, Installation};
use reveille_core::join::{
    CompatibilityAssessment, CompatibilityState, FsGame, LaunchCommand, LaunchProfile,
};
use reveille_core::mapindex::{MapIndex, MapKey};
use serde::Serialize;

mod platform;

#[derive(Default)]
struct AppState {
    servers: Mutex<Vec<Server>>,
}

#[derive(Serialize)]
struct BrowserPayload {
    servers: Vec<BrowserServer>,
    recorded_non_results: usize,
}

#[derive(Serialize)]
struct BrowserServer {
    address: SocketAddrV4,
    server: Server,
    compatibility: CompatibilityAssessment,
}

#[derive(Serialize)]
struct JoinPreview {
    address: SocketAddrV4,
    server: Server,
    assessment: CompatibilityAssessment,
    catalogue: Option<CatalogueResolutionPass>,
    game_directory: PathBuf,
    used_home_fallback: bool,
}

#[derive(Serialize)]
struct JoinResult {
    assessment: CompatibilityAssessment,
    installed: Vec<PathBuf>,
    non_results: Vec<String>,
    game_directory: PathBuf,
    used_home_fallback: bool,
    process_id: Option<u32>,
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
async fn browse_servers(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<BrowserPayload, String> {
    let installation = install::identify(&path).map_err(|error| error.to_string())?;
    let client = platform::detect_client(&installation.root);
    let target = platform::resolve_install_target(&installation.root, "main", client)
        .map_err(|error| error.to_string())?;
    let index = MapIndex::scan(&target.game_directory).map_err(|error| error.to_string())?;
    let report = discovery::browse(BrowseConfig {
        target: TargetGame::AlliedAssault,
        limit: None,
        concurrency: 16,
        master_timeout: Duration::from_secs(15),
        probe_timeout: Duration::from_millis(2_500),
    })
    .await
    .map_err(|error| error.to_string())?;
    let mut addresses = HashSet::new();
    let mut servers = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.server.clone())
        .filter(|server| {
            addresses.insert(SocketAddrV4::new(
                server.endpoint.address,
                server.game_port.get(),
            ))
        })
        .collect::<Vec<_>>();
    servers.sort_by_key(|server| {
        std::cmp::Reverse(
            server
                .occupancy
                .clients_reported
                .map_or(0, discovery::ClientsReported::get),
        )
    });
    let payload = BrowserPayload {
        servers: servers
            .iter()
            .map(|server| BrowserServer {
                address: SocketAddrV4::new(server.endpoint.address, server.game_port.get()),
                compatibility: reveille_core::join::classify_server(&index, server, None),
                server: server.clone(),
            })
            .collect(),
        recorded_non_results: report.summary().non_results,
    };
    *state
        .servers
        .lock()
        .map_err(|_| "server list state is unavailable".to_owned())? = servers;
    Ok(payload)
}

#[tauri::command]
async fn preview_join(
    path: String,
    address: String,
    state: tauri::State<'_, AppState>,
) -> Result<JoinPreview, String> {
    let server = find_server(&state, &address)?;
    build_preview(&path, server).await
}

#[tauri::command]
async fn install_and_launch(
    path: String,
    address: String,
    selected_candidate_ids: Vec<u64>,
    allow_unchecked: bool,
    state: tauri::State<'_, AppState>,
) -> Result<JoinResult, String> {
    let server = find_server(&state, &address)?;
    let preview = build_preview(&path, server.clone()).await?;
    let mut installed = Vec::new();
    let mut non_results = Vec::new();
    if let Some(catalogue) = &preview.catalogue {
        let client = content::MohDbClient::new(Duration::from_secs(30))
            .map_err(|error| error.to_string())?;
        let staging = tempfile::TempDir::new().map_err(|error| error.to_string())?;
        let selected = selected_candidate_ids.into_iter().collect::<HashSet<_>>();
        let mut filenames = HashSet::new();
        for resolution in &catalogue.resolutions {
            let candidate = candidate_for_resolution(&resolution.outcome, &selected);
            let Some(candidate) = candidate else {
                continue;
            };
            if !filenames.insert(candidate.filename.clone()) {
                continue;
            }
            match install_candidate(
                &client,
                candidate,
                &resolution.wanted,
                &server,
                staging.path(),
                &preview.game_directory,
            )
            .await
            {
                Ok(path) => installed.push(path),
                Err(error) => non_results.push(format!("{}: {error}", resolution.wanted.name)),
            }
        }
        non_results.extend(
            catalogue
                .non_results
                .iter()
                .map(|result| format!("{}: catalogue {:?}", result.wanted.name, result.reason)),
        );
    }
    let index = MapIndex::scan(&preview.game_directory).map_err(|error| error.to_string())?;
    let assessment =
        reveille_core::join::classify_server(&index, &server, preview.catalogue.as_ref());
    let may_launch = matches!(assessment.state, CompatibilityState::Compatible)
        || (allow_unchecked && matches!(assessment.state, CompatibilityState::CantTell));
    let process_id = if may_launch {
        let installation = install::identify(&path).map_err(|error| error.to_string())?;
        let kind = platform::detect_client(&installation.root);
        let profile = LaunchProfile::new(TargetGame::AlliedAssault);
        let program = platform::default_client(&installation.root, profile.target, kind)
            .to_string_lossy()
            .into_owned();
        let command = LaunchCommand::new(
            program,
            profile,
            FsGame::new("").map_err(|error| error.to_string())?,
            preview.address,
        )
        .map_err(|error| error.to_string())?;
        Some(
            platform::launch_client(&command, kind)
                .map_err(|error| error.to_string())?
                .id(),
        )
    } else {
        None
    };
    Ok(JoinResult {
        assessment,
        installed,
        non_results,
        game_directory: preview.game_directory,
        used_home_fallback: preview.used_home_fallback,
        process_id,
    })
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

async fn build_preview(path: &str, server: Server) -> Result<JoinPreview, String> {
    let installation = install::identify(path).map_err(|error| error.to_string())?;
    let client_kind = platform::detect_client(&installation.root);
    let target = platform::resolve_install_target(&installation.root, "main", client_kind)
        .map_err(|error| error.to_string())?;
    let index = MapIndex::scan(&target.game_directory).map_err(|error| error.to_string())?;
    let first = reveille_core::join::classify_server(&index, &server, None);
    let wanted = first.preflight.as_ref().map_or_else(Vec::new, wanted_maps);
    let catalogue = if wanted.is_empty() {
        None
    } else {
        Some(
            content::MohDbClient::new(Duration::from_secs(15))
                .map_err(|error| error.to_string())?
                .resolve_all(&wanted)
                .await,
        )
    };
    let assessment = reveille_core::join::classify_server(&index, &server, catalogue.as_ref());
    Ok(JoinPreview {
        address: SocketAddrV4::new(server.endpoint.address, server.game_port.get()),
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

async fn install_candidate(
    client: &content::MohDbClient,
    candidate: &CatalogueCandidate,
    wanted: &WantedMap,
    server: &Server,
    staging: &Path,
    game_directory: &Path,
) -> Result<PathBuf, String> {
    let archive = content::download_mohdb_archive(client, candidate, staging)
        .await
        .map_err(|error| error.to_string())?;
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
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            detect_install,
            browse_servers,
            preview_join,
            install_and_launch
        ])
        .run(tauri::generate_context!())
        .expect("error while running Reveille");
}
