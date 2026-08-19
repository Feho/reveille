// SPDX-License-Identifier: GPL-2.0-only

use std::error::Error;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
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

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        let mut source = error.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Arguments::parse().command {
        Command::Scan { path, format } => scan(&path, format),
    }
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
