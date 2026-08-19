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
use reveille_core::join::{CompatibilityState, FsGame, LaunchCommand, LaunchProfile};
use reveille_core::mapindex::MapIndex;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "reveille", version, about = "Headless MOHAA launcher pipeline")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
    /// Preflight one server, resolve needed content, and emit (but do not run) its launch command.
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
        /// Client program to place in the command description. It is not opened or executed.
        #[arg(long, default_value = "openmohaa")]
        client: String,
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
    state: CompatibilityState,
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
}

struct JoinRequest<'a> {
    server: SocketAddrV4,
    install_root: &'a Path,
    fallback_target: TargetGame,
    fs_game_override: Option<String>,
    client: String,
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
                server_timeout: Duration::from_millis(timeout_ms),
                catalogue_timeout: Duration::from_millis(catalogue_timeout_ms),
                format,
            })
            .await
        }
    }
}

async fn join_server(request: JoinRequest<'_>) -> Result<(), Box<dyn Error>> {
    let JoinRequest {
        server,
        install_root,
        fallback_target,
        fs_game_override,
        client,
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
    let game_directory = install_root.join(profile.data_directory());
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
            })?
        ),
        Format::Text => render_join(
            server,
            &compatibility,
            pakradar.as_ref(),
            catalogue.as_ref(),
            &launch,
        ),
    }
    Ok(())
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
    std::iter::once(command.program.as_str())
        .chain(command.arguments.iter().map(String::as_str))
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
                println!(
                    "  {}:{}  {occupancy}  protocol={}  maps={}  {}",
                    server.endpoint.address,
                    server.game_port,
                    server.protocol.as_deref().unwrap_or("?"),
                    server.rotation.len(),
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
            state: reveille_core::join::classify_server(index, server, None).state,
        })
        .collect::<Vec<_>>();
    let mut summary = CompatibilitySummary::default();
    for server in &servers {
        match server.state {
            CompatibilityState::Compatible => summary.compatible += 1,
            CompatibilityState::NeedsMaps { .. } => summary.needs_maps += 1,
            CompatibilityState::NoSource { .. } => summary.no_source += 1,
            CompatibilityState::CantTell => summary.cant_tell += 1,
        }
    }
    BrowseCompatibility { summary, servers }
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
    use reveille_core::join::{FsGame, LaunchCommand, LaunchProfile};

    use super::{render_launch_command, render_occupancy};

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
            "'/opt/Open MOHAA/openmohaa' +set com_target_game 0 +set fs_game '' +connect 203.0.113.9:12203"
        );
    }
}
