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

async fn browse_servers(config: BrowseConfig, format: Format) -> Result<(), Box<dyn Error>> {
    let report = discovery::browse(config).await?;
    let summary = report.summary();
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&BrowseOutput {
                summary,
                report: &report,
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
    use reveille_core::discovery::{
        BotsReported, ClientCapacity, ClientsReported, ReportedOccupancy,
    };

    use super::render_occupancy;

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
}
