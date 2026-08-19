// SPDX-License-Identifier: GPL-2.0-only

use std::env;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use reveille_core::discovery::TargetGame;
use reveille_core::join::{LaunchCommand, LaunchDialect};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientKind {
    OpenMohaa,
    Retail,
}

impl ClientKind {
    pub(crate) const fn dialect(self) -> LaunchDialect {
        match self {
            Self::OpenMohaa => LaunchDialect::OpenMohaa,
            Self::Retail => LaunchDialect::Retail,
        }
    }
}

pub(crate) fn detect_client(install_root: &Path) -> ClientKind {
    if install_root.join("openmohaa.exe").is_file() {
        ClientKind::OpenMohaa
    } else {
        ClientKind::Retail
    }
}

pub(crate) fn default_client(
    install_root: &Path,
    target: TargetGame,
    client: ClientKind,
) -> PathBuf {
    let filename = match (client, target) {
        (ClientKind::OpenMohaa, _) => "openmohaa.exe",
        (ClientKind::Retail, TargetGame::AlliedAssault) => "MOHAA.exe",
        (ClientKind::Retail, TargetGame::Spearhead) => "moh_spearhead.exe",
        (ClientKind::Retail, TargetGame::Breakthrough) => "moh_breakthrough.exe",
    };
    install_root.join(filename)
}

pub(crate) struct InstallTarget {
    pub(crate) game_directory: PathBuf,
    pub(crate) used_home_fallback: bool,
}

pub(crate) fn resolve_install_target(
    install_root: &Path,
    data_directory: &str,
    client: ClientKind,
) -> Result<InstallTarget, PlatformError> {
    let preferred = install_root.join(data_directory);
    match probe_writable(&preferred) {
        Ok(()) => Ok(InstallTarget {
            game_directory: preferred,
            used_home_fallback: false,
        }),
        Err(source) if client == ClientKind::Retail => Err(PlatformError::RetailUnwritable {
            path: preferred,
            source,
        }),
        Err(_) => {
            let app_data = env::var_os("APPDATA").ok_or(PlatformError::MissingAppData)?;
            let fallback = PathBuf::from(app_data).join("moh").join(data_directory);
            fs::create_dir_all(&fallback).map_err(|source| PlatformError::HomeFallback {
                path: fallback.clone(),
                source,
            })?;
            probe_writable(&fallback).map_err(|source| PlatformError::HomeFallback {
                path: fallback.clone(),
                source,
            })?;
            Ok(InstallTarget {
                game_directory: fallback,
                used_home_fallback: true,
            })
        }
    }
}

fn probe_writable(directory: &Path) -> io::Result<()> {
    for suffix in 0..16 {
        let path = directory.join(format!(
            ".reveille-write-probe-{}-{suffix}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                drop(file);
                return fs::remove_file(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique write probe",
    ))
}

pub(crate) fn launch_client(
    command: &LaunchCommand,
    client: ClientKind,
) -> Result<Child, PlatformError> {
    let program = Path::new(&command.program);
    let mut process = Command::new(program);
    process.args(command.arguments_for(client.dialect()));
    if let Some(parent) = program
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        process.current_dir(parent);
    }
    process.spawn().map_err(|source| PlatformError::Launch {
        program: program.to_path_buf(),
        source,
    })
}

#[derive(Debug, Error)]
pub(crate) enum PlatformError {
    #[error("retail has no writable fallback for {path}")]
    RetailUnwritable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("APPDATA is unavailable for the OpenMoHAA fallback")]
    MissingAppData,
    #[error("could not prepare OpenMoHAA home target {path}")]
    HomeFallback {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not launch client {program}")]
    Launch {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn selects_product_specific_retail_executable() {
        let root = Path::new(r"C:\Games\MOHAA");

        assert_eq!(
            default_client(root, TargetGame::AlliedAssault, ClientKind::Retail),
            root.join("MOHAA.exe")
        );
        assert_eq!(
            default_client(root, TargetGame::Spearhead, ClientKind::Retail),
            root.join("moh_spearhead.exe")
        );
        assert_eq!(
            default_client(root, TargetGame::Breakthrough, ClientKind::Retail),
            root.join("moh_breakthrough.exe")
        );
    }

    #[test]
    fn writable_install_directory_wins_without_a_fallback() {
        let root = TempDir::new().expect("temporary install root");
        fs::create_dir(root.path().join("main")).expect("game directory");

        let target = resolve_install_target(root.path(), "main", ClientKind::OpenMohaa)
            .expect("writable target");

        assert_eq!(target.game_directory, root.path().join("main"));
        assert!(!target.used_home_fallback);
    }
}
