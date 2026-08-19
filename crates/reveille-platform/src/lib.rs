// SPDX-License-Identifier: GPL-2.0-only

//! Shared Windows launcher policy for Reveille's executable front ends.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;

#[cfg(windows)]
use std::env;
#[cfg(windows)]
use std::fs::{self, OpenOptions};
#[cfg(windows)]
use std::process::Command;

use reveille_core::discovery::TargetGame;
use reveille_core::join::{LaunchCommand, LaunchDialect};
use thiserror::Error;

/// Client implementation selected for launch and content-path policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientKind {
    /// Community `OpenMoHAA` client.
    OpenMohaa,
    /// Original retail executable for the selected product.
    Retail,
}

impl ClientKind {
    /// Launch argument dialect understood by this client.
    #[must_use]
    pub const fn dialect(self) -> LaunchDialect {
        match self {
            Self::OpenMohaa => LaunchDialect::OpenMohaa,
            Self::Retail => LaunchDialect::Retail,
        }
    }
}

/// Select `OpenMoHAA` when its executable exists, otherwise retain retail behavior.
#[must_use]
pub fn detect_client(install_root: &Path) -> ClientKind {
    if install_root.join("openmohaa.exe").is_file() {
        ClientKind::OpenMohaa
    } else {
        ClientKind::Retail
    }
}

/// Derive the product-specific executable within one identified installation.
#[must_use]
pub fn default_client(install_root: &Path, target: TargetGame, client: ClientKind) -> PathBuf {
    let filename = match (client, target) {
        (ClientKind::OpenMohaa, _) => "openmohaa.exe",
        (ClientKind::Retail, TargetGame::AlliedAssault) => "MOHAA.exe",
        (ClientKind::Retail, TargetGame::Spearhead) => "moh_spearhead.exe",
        (ClientKind::Retail, TargetGame::Breakthrough) => "moh_breakthrough.exe",
    };
    install_root.join(filename)
}

/// Writable game directory selected for downloaded content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallTarget {
    /// Directory into which archives should be installed.
    pub game_directory: PathBuf,
    /// Whether an unwritable `OpenMoHAA` installation required the home path.
    pub used_home_fallback: bool,
}

/// Probe the conventional install directory and apply the OpenMoHAA-only home fallback.
///
/// # Errors
///
/// Returns an error outside Windows, for an unwritable retail directory, or when the `OpenMoHAA`
/// fallback cannot be created and verified writable.
#[cfg(windows)]
pub fn resolve_install_target(
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

/// Refuse Windows content-path policy on another operating system while keeping portable callers
/// buildable.
#[cfg(not(windows))]
pub fn resolve_install_target(
    _install_root: &Path,
    _data_directory: &str,
    _client: ClientKind,
) -> Result<InstallTarget, PlatformError> {
    Err(PlatformError::UnsupportedOperatingSystem)
}

#[cfg(windows)]
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

/// Spawn the selected client with its exact launch dialect and executable directory.
///
/// # Errors
///
/// Returns an error outside Windows or when the process cannot be spawned.
#[cfg(windows)]
pub fn launch_client(command: &LaunchCommand, client: ClientKind) -> Result<Child, PlatformError> {
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

/// Refuse process spawning on operating systems outside the Windows v1 scope.
#[cfg(not(windows))]
pub fn launch_client(
    _command: &LaunchCommand,
    _client: ClientKind,
) -> Result<Child, PlatformError> {
    Err(PlatformError::UnsupportedOperatingSystem)
}

/// Windows launcher policy failure.
#[derive(Debug, Error)]
pub enum PlatformError {
    /// This operation is deliberately Windows-only in v1.
    #[error("Reveille launch and install-target policy is available only on Windows in v1")]
    UnsupportedOperatingSystem,
    /// Retail has no engine home directory to use instead.
    #[error("retail has no writable fallback for {path}")]
    RetailUnwritable {
        /// Preferred retail data directory.
        path: PathBuf,
        /// Writability probe failure.
        #[source]
        source: io::Error,
    },
    /// `OpenMoHAA` fallback root is unavailable.
    #[error("APPDATA is unavailable for the OpenMoHAA fallback")]
    MissingAppData,
    /// `OpenMoHAA` home directory could not be prepared.
    #[error("could not prepare OpenMoHAA home target {path}")]
    HomeFallback {
        /// Attempted home data directory.
        path: PathBuf,
        /// Creation or writability failure.
        #[source]
        source: io::Error,
    },
    /// Selected executable could not be started.
    #[error("could not launch client {program}")]
    Launch {
        /// Attempted executable.
        program: PathBuf,
        /// Process creation failure.
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn detects_openmohaa_only_when_its_client_is_present() {
        let temporary = TempDir::new().expect("temporary directory");
        assert_eq!(detect_client(temporary.path()), ClientKind::Retail);

        fs::write(temporary.path().join("openmohaa.exe"), []).expect("client marker");
        assert_eq!(detect_client(temporary.path()), ClientKind::OpenMohaa);
    }

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
        assert_eq!(
            default_client(root, TargetGame::AlliedAssault, ClientKind::OpenMohaa),
            root.join("openmohaa.exe")
        );
    }

    #[cfg(windows)]
    #[test]
    fn writable_game_directory_is_preferred_without_a_fallback() {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir(&main).expect("main directory");

        for client in [ClientKind::Retail, ClientKind::OpenMohaa] {
            let target =
                resolve_install_target(temporary.path(), "main", client).expect("writable target");
            assert_eq!(target.game_directory, main);
            assert!(!target.used_home_fallback);
        }
    }
}
