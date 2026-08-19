// SPDX-License-Identifier: GPL-2.0-only

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
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
