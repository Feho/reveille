// SPDX-License-Identifier: GPL-2.0-only

//! Windows caller-side policy for content placement and client launch.

use std::env;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

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

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let probe = directory.join(format!(
            ".reveille-write-probe-{}-{suffix}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                return fs::remove_file(probe);
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
    use std::fs;

    use tempfile::TempDir;

    use super::{ClientKind, resolve_install_target};

    #[test]
    fn writable_game_directory_is_preferred() {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir(&main).expect("main directory");

        let target = resolve_install_target(temporary.path(), "main", ClientKind::Retail)
            .expect("writable retail target");

        assert_eq!(target.game_directory, main);
        assert!(!target.used_home_fallback);
    }
}
