// SPDX-License-Identifier: GPL-2.0-only

use std::error::Error;
use std::fmt::Write as _;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use reveille_core::content::{self, ResolutionOutcome, WantedMap};
use reveille_core::discovery::{self, BrowseConfig, TargetGame};
use reveille_core::install;
use reveille_core::join::{
    CompatibilityAssessment, CompatibilityState, FsGame, LaunchCommand, LaunchProfile,
};
use reveille_core::mapindex::MapIndex;
use reveille_platform as platform;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "reveille", version, about = "Headless MOHAA launcher pipeline")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Discover MOHAA installations from the live Windows registry.
    Discover {
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Run detection, browsing, preflight, map installation, and launch as one journey.
    Journey {
        /// Server to join after it appears in the browse pass.
        server: SocketAddrV4,
        /// User-selected fallback when registry discovery finds no installation.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Game family to browse.
        #[arg(long, value_enum, default_value_t = Game::AlliedAssault)]
        game: Game,
        /// Override automatically detected `OpenMoHAA` or retail launch behavior.
        #[arg(long, value_enum)]
        client_kind: Option<ClientFlavor>,
        /// Override the client executable selected from the installation.
        #[arg(long)]
        client: Option<String>,
        /// Launch when the final preflight is Compatible.
        #[arg(long)]
        execute: bool,
    },
    /// Identify an install and index its maps.
    Scan {
        /// MOHAA installation root (the directory containing `main`).
        path: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Browse public servers through `GameSpy` and MOHAA status queries.
    Browse {
        /// Game family registered with the master.
        #[arg(long, value_enum, default_value_t = Game::AlliedAssault)]
        game: Game,
        /// Maximum registered servers to inspect; zero inspects all.
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Maximum simultaneous server probes.
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        /// Per-server UDP deadline in milliseconds.
        #[arg(long, default_value_t = 2_500)]
        timeout_ms: u64,
        /// Master-server I/O deadline in milliseconds.
        #[arg(long, default_value_t = 15_000)]
        master_timeout_ms: u64,
        /// Optional installation root used to classify every reachable server.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Resolve missing maps for one server into an explicit shopping list.
    Resolve {
        /// Authoritative server game address, such as 173.249.214.104:12203.
        server: SocketAddrV4,
        /// MOHAA installation root (the directory containing `main`).
        path: PathBuf,
        /// Per-server status-query deadline in milliseconds.
        #[arg(long, default_value_t = 2_500)]
        timeout_ms: u64,
        /// Deadline for each moh-db request in milliseconds.
        #[arg(long, default_value_t = 15_000)]
        catalogue_timeout_ms: u64,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Preflight one server, resolve needed content, and optionally launch the client.
    Join {
        /// Authoritative server game address, such as 173.249.214.104:12203.
        server: SocketAddrV4,
        /// MOHAA installation/profile root.
        path: PathBuf,
        /// Fallback profile when the server omits or mangles `com_gamename`.
        #[arg(long, value_enum, default_value_t = Game::AlliedAssault)]
        game: Game,
        /// Override the server-published mod directory. Empty selects the profile's base game.
        #[arg(long)]
        fs_game: Option<String>,
        /// Client executable to describe or launch.
        #[arg(long, default_value = "openmohaa")]
        client: String,
        /// Select `OpenMoHAA`'s single-executable dialect or retail's per-product dialect.
        #[arg(long, value_enum, default_value_t = ClientFlavor::OpenMohaa)]
        client_kind: ClientFlavor,
        /// Start the client after emitting the preflight result.
        #[arg(long)]
        execute: bool,
        /// Per-server status-query deadline in milliseconds.
        #[arg(long, default_value_t = 2_500)]
        timeout_ms: u64,
        /// Deadline for each content-source request in milliseconds.
        #[arg(long, default_value_t = 15_000)]
        catalogue_timeout_ms: u64,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Game {
    #[default]
    AlliedAssault,
    Spearhead,
    Breakthrough,
}

impl From<Game> for TargetGame {
    fn from(game: Game) -> Self {
        match game {
            Game::AlliedAssault => Self::AlliedAssault,
            Game::Spearhead => Self::Spearhead,
            Game::Breakthrough => Self::Breakthrough,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ClientFlavor {
    #[default]
    OpenMohaa,
    Retail,
}

impl From<ClientFlavor> for platform::ClientKind {
    fn from(value: ClientFlavor) -> Self {
        match value {
            ClientFlavor::OpenMohaa => Self::OpenMohaa,
            ClientFlavor::Retail => Self::Retail,
        }
    }
}

#[derive(Serialize)]
struct ScanOutput<'a> {
    installation: &'a install::Installation,
    game_directory: &'a Path,
    index: &'a MapIndex,
}

#[derive(Serialize)]
struct BrowseOutput<'a> {
    summary: discovery::BrowseSummary,
    report: &'a discovery::BrowseReport,
    compatibility: &'a Option<BrowseCompatibility>,
}

#[derive(Serialize)]
struct BrowseCompatibility {
    summary: CompatibilitySummary,
    servers: Vec<ClassifiedServer>,
}

#[derive(Default, Serialize)]
struct CompatibilitySummary {
    compatible: usize,
    needs_maps: usize,
    no_source: usize,
    cant_tell: usize,
}

#[derive(Serialize)]
struct ClassifiedServer {
    server: SocketAddrV4,
    assessment: CompatibilityAssessment,
}

#[derive(Serialize)]
struct ResolveOutput<'a> {
    server: SocketAddrV4,
    game_directory: &'a Path,
    preflight: &'a reveille_core::preflight::Report,
    pakradar: &'a Option<PakRadarOutput>,
    catalogue: &'a content::CatalogueResolutionPass,
}

#[derive(Serialize)]
struct PakRadarOutput {
    url: String,
    entries: Vec<content::PakRadarEntry>,
    non_result: Option<String>,
}

#[derive(Serialize)]
struct JoinOutput<'a> {
    server: SocketAddrV4,
    game_directory: &'a Path,
    compatibility: &'a CompatibilityState,
    preflight: &'a Option<reveille_core::preflight::Report>,
    pakradar: &'a Option<PakRadarOutput>,
    launch: &'a LaunchCommand,
    used_home_fallback: bool,
    launched_process_id: Option<u32>,
}

struct JoinRequest<'a> {
    server: SocketAddrV4,
    install_root: &'a Path,
    fallback_target: TargetGame,
    fs_game_override: Option<String>,
    client: String,
    client_kind: platform::ClientKind,
    execute: bool,
    server_timeout: Duration,
    catalogue_timeout: Duration,
    format: Format,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    match Arguments::parse().command {
        Command::Discover { format } => discover_windows(format),
        Command::Journey {
            server,
            path,
            game,
            client_kind,
            client,
            execute,
        } => {
            run_journey(
                server,
                path.as_deref(),
                game.into(),
                client_kind.map(Into::into),
                client,
                execute,
            )
            .await
        }
        Command::Scan { path, format } => scan(&path, format),
        Command::Browse {
            game,
            limit,
            concurrency,
            timeout_ms,
            master_timeout_ms,
            path,
            format,
        } => {
            browse_servers(
                BrowseConfig {
                    target: game.into(),
                    limit: (limit != 0).then_some(limit),
                    concurrency,
                    master_timeout: Duration::from_millis(master_timeout_ms),
                    probe_timeout: Duration::from_millis(timeout_ms),
                },
                path.as_deref(),
                format,
            )
            .await
        }
        Command::Resolve {
            server,
            path,
            timeout_ms,
            catalogue_timeout_ms,
            format,
        } => {
            resolve_server(
                server,
                &path,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(catalogue_timeout_ms),
                format,
            )
            .await
        }
        Command::Join {
            server,
            path,
            game,
            fs_game,
            client,
            client_kind,
            execute,
            timeout_ms,
            catalogue_timeout_ms,
            format,
        } => {
            join_server(JoinRequest {
                server,
                install_root: &path,
                fallback_target: game.into(),
                fs_game_override: fs_game,
                client,
                client_kind: client_kind.into(),
                execute,
                server_timeout: Duration::from_millis(timeout_ms),
                catalogue_timeout: Duration::from_millis(catalogue_timeout_ms),
                format,
            })
            .await
        }
    }
}

async fn run_journey(
    address: SocketAddrV4,
    selected_path: Option<&Path>,
    target: TargetGame,
    client_kind: Option<platform::ClientKind>,
    client_override: Option<String>,
    execute: bool,
) -> Result<(), Box<dyn Error>> {
    let installation = detect_install(selected_path)?;
    let client_kind = client_kind.unwrap_or_else(|| platform::detect_client(&installation.root));
    println!("Install: {}", installation.root.display());
    println!("Identification: {:?}", installation.identification);

    let server = browse_journey_target(target, address).await?;
    let profile = LaunchProfile::new(target);
    let install_target = platform::resolve_install_target(
        &installation.root,
        profile.data_directory(),
        client_kind,
    )?;
    if install_target.used_home_fallback {
        println!(
            "Downloaded maps will be kept in {}",
            install_target.game_directory.display()
        );
    }

    let index = MapIndex::scan(&install_target.game_directory)?;
    let initial = reveille_core::join::classify_server(&index, &server, None);
    println!("Preflight: {}", render_compatibility_state(&initial.state));
    let wanted = initial
        .preflight
        .as_ref()
        .map_or_else(Vec::new, wanted_maps);
    let catalogue = if wanted.is_empty() {
        None
    } else {
        Some(
            content::MohDbClient::new(Duration::from_secs(15))?
                .resolve_all(&wanted)
                .await,
        )
    };

    let install_non_results =
        install_journey_content(catalogue.as_ref(), &server, &install_target.game_directory)
            .await?;
    for non_result in &install_non_results {
        println!("Content non-result: {non_result}");
    }

    let updated_index = MapIndex::scan(&install_target.game_directory)?;
    let final_assessment =
        reveille_core::join::classify_server(&updated_index, &server, catalogue.as_ref());
    println!(
        "Final preflight: {}",
        render_compatibility_state(&final_assessment.state)
    );
    let program = client_override.unwrap_or_else(|| {
        platform::default_client(&installation.root, target, client_kind)
            .to_string_lossy()
            .into_owned()
    });
    let command = LaunchCommand::new(program, profile, FsGame::new("")?, address)?;
    if execute && matches!(final_assessment.state, CompatibilityState::Compatible) {
        let child = platform::launch_client(&command, client_kind)?;
        println!("Launched client process {}", child.id());
    } else if execute {
        println!("Client not launched because maps are still unresolved.");
    } else {
        let arguments = command.arguments_for(client_kind.dialect());
        println!(
            "Launch ready: {}",
            render_command_parts(&command.program, &arguments)
        );
    }
    Ok(())
}

fn detect_install(selected_path: Option<&Path>) -> Result<install::Installation, Box<dyn Error>> {
    if let Some(path) = selected_path {
        return Ok(install::identify(path)?);
    }
    #[cfg(windows)]
    {
        let keys = reveille_core::platform::registry::read_live_hives()?;
        let mut roots = reveille_core::platform::registry::discover_ea_install_roots(&keys)
            .into_iter()
            .map(|candidate| candidate.root)
            .collect::<Vec<_>>();
        if let Some(root) = reveille_core::platform::registry::discover_gog_install_root(&keys) {
            roots.push(root);
        }
        for root in roots {
            if let Ok(installation) = install::identify(root) {
                return Ok(installation);
            }
        }
    }
    Err("No MOHAA installation was detected; pass --path once to select it.".into())
}

#[cfg(windows)]
fn discover_windows(format: Format) -> Result<(), Box<dyn Error>> {
    use reveille_core::platform::registry;

    let keys = registry::read_live_hives()?;
    let ea_roots = registry::discover_ea_install_roots(&keys);
    let ea = registry::identify_ea_installations(&ea_roots);
    let gog_root = registry::discover_gog_install_root(&keys);
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ea": ea,
                "gog_root": gog_root,
            }))?
        ),
        Format::Text => {
            for installation in ea.installations {
                println!(
                    "EA install: {} ({:?})",
                    installation.installation.root.display(),
                    installation.installation.identification
                );
            }
            for skipped in ea.skipped {
                println!(
                    "EA non-result: {} — {}",
                    skipped.discovery.root.display(),
                    skipped.reason
                );
            }
            if let Some(root) = gog_root {
                println!("GOG install root: {}", root.display());
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn discover_windows(_format: Format) -> Result<(), Box<dyn Error>> {
    Err("live registry discovery is available only on Windows".into())
}

async fn join_server(request: JoinRequest<'_>) -> Result<(), Box<dyn Error>> {
    let JoinRequest {
        server,
        install_root,
        fallback_target,
        fs_game_override,
        client,
        client_kind,
        execute,
        server_timeout,
        catalogue_timeout,
        format,
    } = request;
    let game_port = discovery::GamePort::new(server.port());
    let status = discovery::query_getstatus(*server.ip(), game_port, server_timeout).await?;
    let target = status
        .get("com_gamename")
        .and_then(|value| TargetGame::from_game_name(value))
        .unwrap_or(fallback_target);
    let profile = LaunchProfile::new(target);
    let install_target =
        platform::resolve_install_target(install_root, profile.data_directory(), client_kind)?;
    let game_directory = install_target.game_directory;
    let index = MapIndex::scan(&game_directory)?;
    let rotation = status
        .get("sv_maplist")
        .map(|value| value.split_whitespace().collect::<Vec<_>>())
        .filter(|rotation| !rotation.is_empty());
    let published_checksum = published_checksum(&status);
    let preflight = rotation.as_ref().map(|rotation| {
        reveille_core::preflight::check(&index, rotation, published_checksum.as_ref())
    });
    let wanted = preflight.as_ref().map_or_else(Vec::new, wanted_maps);

    // No published rotation means Can't tell immediately and deliberately makes no moh-db call.
    let catalogue = if rotation.is_none() || wanted.is_empty() {
        None
    } else {
        Some(
            content::MohDbClient::new(catalogue_timeout)?
                .resolve_all(&wanted)
                .await,
        )
    };
    let pakradar = if wanted.is_empty() {
        None
    } else {
        fetch_pakradar(&status, catalogue_timeout).await
    };
    let compatibility = reveille_core::join::classify(preflight.as_ref(), catalogue.as_ref());

    let fs_game_value = fs_game_override
        .or_else(|| status.get("fs_game").cloned())
        .unwrap_or_default();
    let fs_game_value = if fs_game_value.eq_ignore_ascii_case(profile.data_directory()) {
        String::new()
    } else {
        fs_game_value
    };
    let launch = LaunchCommand::new(client, profile, FsGame::new(fs_game_value)?, server)?;
    let launched_process_id = execute
        .then(|| platform::launch_client(&launch, client_kind))
        .transpose()?
        .map(|child| child.id());

    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&JoinOutput {
                server,
                game_directory: &game_directory,
                compatibility: &compatibility,
                preflight: &preflight,
                pakradar: &pakradar,
                launch: &launch,
                used_home_fallback: install_target.used_home_fallback,
                launched_process_id,
            })?
        ),
        Format::Text => {
            if install_target.used_home_fallback {
                println!(
                    "Install directory is not writable; OpenMoHAA content target: {}",
                    game_directory.display()
                );
            }
            render_join(
                server,
                &compatibility,
                pakradar.as_ref(),
                catalogue.as_ref(),
                &launch,
            );
            if let Some(process_id) = launched_process_id {
                println!("Launched client process {process_id}");
            }
        }
    }
    Ok(())
}

async fn browse_journey_target(
    target: TargetGame,
    address: SocketAddrV4,
) -> Result<discovery::Server, Box<dyn Error>> {
    let report = discovery::browse(BrowseConfig {
        target,
        limit: None,
        concurrency: 16,
        master_timeout: Duration::from_secs(15),
        probe_timeout: Duration::from_millis(2_500),
    })
    .await?;
    let servers = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.server.as_ref())
        .collect::<Vec<_>>();
    println!(
        "Servers answering now: {} ({} recorded non-results)",
        servers.len(),
        report.summary().non_results
    );
    for server in &servers {
        println!(
            "  {}:{}  {}  {}",
            server.endpoint.address,
            server.game_port,
            render_occupancy(server.occupancy, server.client_capacity),
            server.hostname
        );
    }
    servers
        .into_iter()
        .find(|server| {
            SocketAddrV4::new(server.endpoint.address, server.game_port.get()) == address
        })
        .cloned()
        .ok_or_else(|| format!("selected server {address} did not answer the browse pass").into())
}

async fn install_journey_content(
    catalogue: Option<&content::CatalogueResolutionPass>,
    server: &discovery::Server,
    game_directory: &Path,
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut non_results = Vec::new();
    let Some(catalogue) = catalogue else {
        return Ok(non_results);
    };
    let client = content::MohDbClient::new(Duration::from_secs(30))?;
    let staging = tempfile::TempDir::new()?;
    for resolution in &catalogue.resolutions {
        let ResolutionOutcome::Exact { name_match, .. } = &resolution.outcome else {
            continue;
        };
        let result: Result<PathBuf, Box<dyn Error>> = async {
            let archive =
                content::download_mohdb_archive(&client, name_match, staging.path()).await?;
            let inspection = content::inspect_archive(&archive.path)?;
            let checksum = server
                .current_map
                .as_deref()
                .filter(|current| {
                    reveille_core::mapindex::MapKey::new(current)
                        == Some(resolution.wanted.key.clone())
                })
                .and(server.map_checksum);
            content::confirm_map(&inspection, &resolution.wanted.name, checksum)?;
            Ok(content::install_archive(&archive, game_directory)?)
        }
        .await;
        match result {
            Ok(path) => println!("Installed {}", path.display()),
            Err(error) => non_results.push(format!("{}: {error}", resolution.wanted.name)),
        }
    }
    non_results.extend(
        catalogue
            .non_results
            .iter()
            .map(|result| format!("{}: catalogue {:?}", result.wanted.name, result.reason)),
    );
    Ok(non_results)
}

fn published_checksum(
    status: &discovery::FieldMap,
) -> Option<reveille_core::preflight::PublishedChecksum> {
    status
        .get("mapname")
        .zip(status.get("sv_mapChecksum"))
        .and_then(|(map, checksum)| {
            checksum.parse::<i32>().ok().map(|checksum| {
                reveille_core::preflight::PublishedChecksum {
                    map: map.clone(),
                    checksum: reveille_core::bsp::Checksum::new(checksum),
                }
            })
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

async fn fetch_pakradar(status: &discovery::FieldMap, timeout: Duration) -> Option<PakRadarOutput> {
    let url = status.get("pr_downloads")?;
    Some(match content::fetch_filelist(url, timeout).await {
        Ok(entries) => PakRadarOutput {
            url: url.clone(),
            entries,
            non_result: None,
        },
        Err(error) => PakRadarOutput {
            url: url.clone(),
            entries: Vec::new(),
            non_result: Some(error.to_string()),
        },
    })
}

fn render_join(
    server: SocketAddrV4,
    compatibility: &CompatibilityState,
    pakradar: Option<&PakRadarOutput>,
    catalogue: Option<&content::CatalogueResolutionPass>,
    launch: &LaunchCommand,
) {
    println!("Server: {server}");
    match compatibility {
        CompatibilityState::Compatible => {
            println!("Compatibility: Compatible — nothing checkable is wrong");
        }
        CompatibilityState::NeedsMaps { count, .. } => {
            println!("Compatibility: Needs {count} maps");
        }
        CompatibilityState::NoSource { count } => {
            println!("Compatibility: No source for {count} needed maps");
        }
        CompatibilityState::CantTell => {
            println!("Compatibility: Can't tell — server did not publish a rotation");
        }
    }
    if let Some(catalogue) = catalogue {
        render_content_sources(pakradar, catalogue);
    }
    println!(
        "Profile: {} ({})",
        launch.profile.target,
        launch.profile.data_directory()
    );
    println!(
        "fs_game: {}",
        if launch.fs_game.as_str().is_empty() {
            "<base>"
        } else {
            launch.fs_game.as_str()
        }
    );
    println!("Launch (not executed): {}", render_launch_command(launch));
}

fn render_launch_command(command: &LaunchCommand) -> String {
    render_command_parts(&command.program, &command.arguments)
}

fn render_command_parts(program: &str, arguments: &[String]) -> String {
    std::iter::once(program)
        .chain(arguments.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(part: &str) -> String {
    if !part.is_empty()
        && part
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        part.to_owned()
    } else {
        format!("'{}'", part.replace('\'', "'\\''"))
    }
}

async fn resolve_server(
    server: SocketAddrV4,
    install_root: &Path,
    server_timeout: Duration,
    catalogue_timeout: Duration,
    format: Format,
) -> Result<(), Box<dyn Error>> {
    let game_directory = install_root.join("main");
    let index = MapIndex::scan(&game_directory)?;
    let game_port = discovery::GamePort::new(server.port());
    let status = discovery::query_getstatus(*server.ip(), game_port, server_timeout).await?;
    let rotation = status
        .get("sv_maplist")
        .map(|value| value.split_whitespace().collect::<Vec<_>>())
        .unwrap_or_default();
    let published_checksum = status
        .get("mapname")
        .zip(status.get("sv_mapChecksum"))
        .and_then(|(map, checksum)| {
            checksum.parse::<i32>().ok().map(|checksum| {
                reveille_core::preflight::PublishedChecksum {
                    map: map.clone(),
                    checksum: reveille_core::bsp::Checksum::new(checksum),
                }
            })
        });
    let preflight = reveille_core::preflight::check(&index, &rotation, published_checksum.as_ref());
    let wanted = preflight
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
        .collect::<Vec<_>>();
    let catalogue = content::MohDbClient::new(catalogue_timeout)?
        .resolve_all(&wanted)
        .await;
    let pakradar = if let Some(url) = status.get("pr_downloads") {
        Some(
            match content::fetch_filelist(url, catalogue_timeout).await {
                Ok(entries) => PakRadarOutput {
                    url: url.clone(),
                    entries,
                    non_result: None,
                },
                Err(error) => PakRadarOutput {
                    url: url.clone(),
                    entries: Vec::new(),
                    non_result: Some(error.to_string()),
                },
            },
        )
    } else {
        None
    };

    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&ResolveOutput {
                server,
                game_directory: &game_directory,
                preflight: &preflight,
                pakradar: &pakradar,
                catalogue: &catalogue,
            })?
        ),
        Format::Text => render_resolution(server, &preflight, pakradar.as_ref(), &catalogue),
    }
    Ok(())
}

fn render_resolution(
    server: SocketAddrV4,
    preflight: &reveille_core::preflight::Report,
    pakradar: Option<&PakRadarOutput>,
    catalogue: &content::CatalogueResolutionPass,
) {
    println!("Server: {server}");
    println!("Preflight: {:?}", preflight.verdict);
    render_content_sources(pakradar, catalogue);
}

fn render_content_sources(
    pakradar: Option<&PakRadarOutput>,
    catalogue: &content::CatalogueResolutionPass,
) {
    println!(
        "Maps requiring content: {}",
        catalogue.resolutions.len() + catalogue.non_results.len()
    );
    if let Some(pakradar) = pakradar {
        if let Some(reason) = &pakradar.non_result {
            println!("PakRadar manifest non-result: {reason}");
        } else {
            println!(
                "PakRadar packages with server-published MD5: {}",
                pakradar.entries.len()
            );
            for entry in &pakradar.entries {
                println!("  {} — {} — {}", entry.alias, entry.md5, entry.url);
            }
        }
    }
    for resolution in &catalogue.resolutions {
        match &resolution.outcome {
            ResolutionOutcome::Exact { name_match, .. } => println!(
                "  {}: exact name match — {} ({} bytes; confirm archive after download)",
                resolution.wanted.name,
                name_match.filename,
                name_match.file_size.get()
            ),
            ResolutionOutcome::ChoiceRequired { choices } => {
                println!(
                    "  {}: choice required (nothing auto-applied)",
                    resolution.wanted.name
                );
                for choice in choices {
                    println!(
                        "    {} → {} ({} bytes)",
                        choice.map_name.trim(),
                        choice.filename,
                        choice.file_size.get()
                    );
                }
            }
            ResolutionOutcome::NoSource => {
                println!("  {}: no source", resolution.wanted.name);
            }
        }
    }
    for non_result in &catalogue.non_results {
        println!(
            "  {}: catalogue non-result — {:?}",
            non_result.wanted.name, non_result.reason
        );
    }
}

async fn browse_servers(
    config: BrowseConfig,
    install_root: Option<&Path>,
    format: Format,
) -> Result<(), Box<dyn Error>> {
    let target = config.target;
    let report = discovery::browse(config).await?;
    let summary = report.summary();
    let compatibility = install_root
        .map(|root| MapIndex::scan(root.join(LaunchProfile::new(target).data_directory())))
        .transpose()?
        .map(|index| classify_browse_report(&index, &report));
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&BrowseOutput {
                summary,
                report: &report,
                compatibility: &compatibility,
            })?
        ),
        Format::Text => {
            println!(
                "Game: {} ({})",
                report.target.label(),
                report.target.game_name()
            );
            println!("Registered: {}", summary.registered);
            println!("Inspected: {}", summary.inspected);
            println!("GameSpy reachable: {}", summary.gamespy_reachable);
            println!("getstatus reachable: {}", summary.getstatus_reachable);
            println!("Clients reported: {}", summary.clients_reported);
            println!("Bots reported: {}", summary.bots_reported);
            println!("Rotations published: {}", summary.rotations_published);
            println!(
                "Map checksums published: {}",
                summary.map_checksums_published
            );
            println!("PakRadar manifests: {}", summary.pakradar_published);
            println!("pure published: {}", summary.pure_published);
            println!("Protocols: {:?}", summary.protocols);
            if let Some(compatibility) = &compatibility {
                println!("Compatible: {}", compatibility.summary.compatible);
                println!("Needs maps: {}", compatibility.summary.needs_maps);
                println!("No source: {}", compatibility.summary.no_source);
                println!("Can't tell: {}", compatibility.summary.cant_tell);
            }

            let mut servers = report
                .outcomes
                .iter()
                .filter_map(|outcome| outcome.server.as_ref())
                .collect::<Vec<_>>();
            servers.sort_by_key(|server| {
                std::cmp::Reverse(
                    server
                        .occupancy
                        .clients_reported
                        .map_or(0, discovery::ClientsReported::get),
                )
            });
            println!();
            println!("Servers (client slots and bots are disjoint reported quantities):");
            for server in servers {
                let occupancy = render_occupancy(server.occupancy, server.client_capacity);
                let compatibility_state = compatibility.as_ref().and_then(|compatibility| {
                    let address =
                        SocketAddrV4::new(server.endpoint.address, server.game_port.get());
                    compatibility
                        .servers
                        .iter()
                        .find(|classified| classified.server == address)
                        .map(|classified| render_compatibility_state(&classified.assessment.state))
                });
                println!(
                    "  {}:{}  {occupancy}  protocol={}  maps={}{}  {}",
                    server.endpoint.address,
                    server.game_port,
                    server.protocol.as_deref().unwrap_or("?"),
                    server.rotation.len(),
                    compatibility_state
                        .map(|state| format!("  {state}"))
                        .unwrap_or_default(),
                    server.hostname
                );
            }

            println!();
            println!("Recorded non-results: {}", summary.non_results);
        }
    }
    Ok(())
}

fn classify_browse_report(
    index: &MapIndex,
    report: &discovery::BrowseReport,
) -> BrowseCompatibility {
    let servers = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.server.as_ref())
        .map(|server| ClassifiedServer {
            server: SocketAddrV4::new(server.endpoint.address, server.game_port.get()),
            assessment: reveille_core::join::classify_server(index, server, None),
        })
        .collect::<Vec<_>>();
    let mut summary = CompatibilitySummary::default();
    for server in &servers {
        match server.assessment.state {
            CompatibilityState::Compatible => summary.compatible += 1,
            CompatibilityState::NeedsMaps { .. } => summary.needs_maps += 1,
            CompatibilityState::NoSource { .. } => summary.no_source += 1,
            CompatibilityState::CantTell => summary.cant_tell += 1,
        }
    }
    BrowseCompatibility { summary, servers }
}

fn render_compatibility_state(state: &CompatibilityState) -> String {
    match state {
        CompatibilityState::Compatible => "compatible".to_owned(),
        CompatibilityState::NeedsMaps { count, .. } => format!("needs {count} maps"),
        CompatibilityState::NoSource { count } => format!("no source for {count} maps"),
        CompatibilityState::CantTell => "can't tell".to_owned(),
    }
}

fn render_occupancy(
    occupancy: discovery::ReportedOccupancy,
    capacity: Option<discovery::ClientCapacity>,
) -> String {
    let mut rendered = occupancy.clients_reported.map_or_else(
        || "? clients".to_owned(),
        |clients| format!("{clients} clients"),
    );
    if let Some(bots) = occupancy.bots_reported.filter(|bots| bots.get() > 0) {
        let _ = write!(rendered, " (+{bots} bots)");
    }
    rendered.push_str(" · cap ");
    rendered.push_str(&capacity.map_or_else(|| "?".to_owned(), |capacity| capacity.to_string()));
    rendered
}

fn scan(path: &Path, format: Format) -> Result<(), Box<dyn Error>> {
    let installation = install::identify(path)?;
    let game_directory = path.join("main");
    let index = MapIndex::scan(&game_directory)?;

    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&ScanOutput {
                installation: &installation,
                game_directory: &game_directory,
                index: &index,
            })?
        ),
        Format::Text => {
            let stats = index.stats();
            println!("Install: {}", installation.root.display());
            println!("Identification: {:?}", installation.identification);
            println!("Products: {:?}", installation.products);
            println!("Game directory: {}", game_directory.display());
            println!("Archives: {}", stats.archives);
            println!("Package directories: {}", stats.package_directories);
            println!("Loose BSP files: {}", stats.loose_bsp_files);
            println!("Maps: {}", stats.maps);
            println!(
                "Maps with multiple providers: {}",
                stats.multi_provider_maps
            );
            println!("Skipped BSP entries: {}", stats.skipped_entries);
            println!();
            println!("Effective maps:");
            for map in index.maps() {
                println!(
                    "  {}: {} ({} provider{})",
                    map.display_name,
                    map.effective_provider()
                        .map(reveille_core::mapindex::Provider::checksum)
                        .map_or_else(|| "unknown".to_owned(), |checksum| checksum.to_string()),
                    map.providers.len(),
                    if map.providers.len() == 1 { "" } else { "s" }
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use reveille_core::discovery::{
        BotsReported, ClientCapacity, ClientsReported, ReportedOccupancy, TargetGame,
    };
    use reveille_core::join::{
        CompatibilityAssessment, CompatibilityState, CurrentMapReadiness, FsGame, LaunchCommand,
        LaunchDialect, LaunchProfile,
    };

    use super::{
        ClassifiedServer, render_command_parts, render_compatibility_state, render_launch_command,
        render_occupancy,
    };

    #[test]
    fn renders_equal_mixed_client_and_bot_counts_additively() {
        let occupancy =
            ReportedOccupancy::new(Some(ClientsReported::new(3)), Some(BotsReported::new(3)));

        assert_eq!(
            render_occupancy(occupancy, Some(ClientCapacity::new(32))),
            "3 clients (+3 bots) · cap 32"
        );
    }

    #[test]
    fn renders_more_bots_than_clients_additively() {
        let occupancy =
            ReportedOccupancy::new(Some(ClientsReported::new(3)), Some(BotsReported::new(8)));

        assert_eq!(
            render_occupancy(occupancy, Some(ClientCapacity::new(32))),
            "3 clients (+8 bots) · cap 32"
        );
    }

    #[test]
    fn renders_the_emitted_launch_command_without_running_it() {
        let command = LaunchCommand::new(
            "/opt/Open MOHAA/openmohaa",
            LaunchProfile::new(TargetGame::AlliedAssault),
            FsGame::new("").expect("base game"),
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 9), 12_203),
        )
        .expect("launch command");

        assert_eq!(
            render_launch_command(&command),
            "'/opt/Open MOHAA/openmohaa' +set com_target_game 0 +set fs_game '' +set ui_console 1 +set cl_playintro 0 +connect 203.0.113.9:12203"
        );
    }

    #[test]
    fn renders_the_selected_retail_dialect_without_openmohaa_arguments() {
        let command = LaunchCommand::new(
            r"C:\Games\MOHAA\MOHAA.exe",
            LaunchProfile::new(TargetGame::AlliedAssault),
            FsGame::new("").expect("base game"),
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 9), 12_203),
        )
        .expect("launch command");
        let arguments = command.arguments_for(LaunchDialect::Retail);

        assert_eq!(
            render_command_parts(&command.program, &arguments),
            r"'C:\Games\MOHAA\MOHAA.exe' +set fs_game '' +set ui_console 1 +set cl_playintro 0 +connect 203.0.113.9:12203"
        );
    }

    #[test]
    fn browse_json_keeps_each_servers_full_compatibility_assessment() {
        let classified = ClassifiedServer {
            server: SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 12_203),
            assessment: CompatibilityAssessment {
                state: CompatibilityState::CantTell,
                preflight: None,
                current_map: CurrentMapReadiness::Unknown,
            },
        };

        let value = serde_json::to_value(classified).expect("classified server serializes");
        assert_eq!(value["server"], "203.0.113.10:12203");
        assert_eq!(value["assessment"]["state"]["state"], "cant_tell");
        assert!(value["assessment"]["preflight"].is_null());
        assert_eq!(
            render_compatibility_state(&CompatibilityState::CantTell),
            "can't tell"
        );
    }
}
