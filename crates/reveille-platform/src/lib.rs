// SPDX-License-Identifier: GPL-2.0-only

//! Shared launcher and content-path policy for Reveille's executable front ends.
//!
//! The policy encoded here targets Windows, which is v1's only supported platform, but the code
//! is portable so the composed pipeline stays exercisable — and testable in CI — on Linux.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;

use std::env;
use std::fs::{self, OpenOptions};
use std::process::Command;

use reveille_core::discovery::TargetGame;
use reveille_core::engine::EngineChoice;
use reveille_core::join::{LaunchCommand, LaunchDialect};
use reveille_core::platform::openmohaa::{self, ClientActivity};
use thiserror::Error;

/// One kind of `OpenMoHAA` program whose files a release replaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMohaaProgram {
    /// The playable game client.
    Game,
    /// The dedicated server.
    DedicatedServer,
    /// One of the expansion launch helpers.
    Launcher,
}

/// Process-list evidence about programs whose files an `OpenMoHAA` release replaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenMohaaActivity {
    checked: bool,
    running: Vec<OpenMohaaProgram>,
}

impl OpenMohaaActivity {
    /// Conservative core state used by the transactional replacement gate.
    #[must_use]
    pub fn client_activity(&self) -> ClientActivity {
        if !self.checked {
            ClientActivity::Unknown
        } else if self.running.is_empty() {
            ClientActivity::ConfirmedStopped
        } else {
            ClientActivity::Running
        }
    }

    /// Exact program kinds observed in the process list.
    #[must_use]
    pub fn running_programs(&self) -> &[OpenMohaaProgram] {
        &self.running
    }

    /// No trustworthy process-list result was available.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            checked: false,
            running: Vec::new(),
        }
    }

    const fn checked() -> Self {
        Self {
            checked: true,
            running: Vec::new(),
        }
    }

    fn observe(&mut self, program: OpenMohaaProgram) {
        if !self.running.contains(&program) {
            self.running.push(program);
        }
    }
}

/// Client implementation selected for launch and content-path policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientKind {
    /// Community `OpenMoHAA` client.
    OpenMohaa,
    /// Original retail executable for the selected product.
    Retail,
    /// Reborn retains the retail executable and argument dialect.
    Reborn,
}

impl ClientKind {
    /// Launch argument dialect understood by this client.
    #[must_use]
    pub const fn dialect(self) -> LaunchDialect {
        match self {
            Self::OpenMohaa => LaunchDialect::OpenMohaa,
            Self::Retail | Self::Reborn => LaunchDialect::Retail,
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

/// Conservatively report whether a Windows `OpenMoHAA` installation can be replaced.
///
/// A failed or unavailable process query is `Unknown`, never evidence that the client stopped.
#[must_use]
pub fn openmohaa_activity() -> OpenMohaaActivity {
    #[cfg(windows)]
    {
        // Microsoft tasklist documentation: learn.microsoft.com/windows-server/administration/
        // windows-commands/tasklist. One unfiltered CSV listing is used rather than an
        // `IMAGENAME eq` filter per name, because `/FI` filters combine with AND and cannot
        // express "any of these images". Matching on image name alone is deliberately
        // conservative: tasklist cannot prove which installation launched a process.
        let Ok(output) = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
        else {
            return OpenMohaaActivity::unknown();
        };
        if !output.status.success() {
            return OpenMohaaActivity::unknown();
        }
        tasklist_release_activity(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(windows))]
    {
        OpenMohaaActivity::unknown()
    }
}

/// Does any listed process hold one of the executables a release archive overwrites?
///
/// The client is not the only lock: an archive also replaces `omohaaded.exe` and the three
/// `launch_openmohaa_*.exe` shims, and a running dedicated server locks `game.dll` just as a
/// running client does. Naming only `openmohaa.exe` would report `ConfirmedStopped` and then
/// fail part-way through the apply on a sharing violation.
#[cfg(any(windows, test))]
fn tasklist_release_activity(output: &str) -> OpenMohaaActivity {
    let mut activity = OpenMohaaActivity::checked();
    let mut valid_rows = 0_usize;
    // `tasklist_image_name` lowercases, so this also matches a row spelled `OMohAADed.EXE`.
    for image in output.lines().filter_map(tasklist_image_name) {
        valid_rows += 1;
        let Some(stem) = image.strip_suffix(".exe").map(str::to_owned) else {
            continue;
        };
        if !openmohaa::RELEASE_EXECUTABLE_STEMS.contains(&stem.as_str()) {
            continue;
        }
        match stem.as_str() {
            "openmohaa" => activity.observe(OpenMohaaProgram::Game),
            "omohaaded" => activity.observe(OpenMohaaProgram::DedicatedServer),
            "launch_openmohaa_base"
            | "launch_openmohaa_spearhead"
            | "launch_openmohaa_breakthrough" => activity.observe(OpenMohaaProgram::Launcher),
            _ => {}
        }
    }
    if !output.trim().is_empty() && valid_rows == 0 {
        return OpenMohaaActivity::unknown();
    }
    activity
}

/// Read the quoted image name from one `tasklist /FO CSV /NH` row, lowercased.
///
/// A row that is not a quoted CSV record — a localised "no tasks are running" notice, or a
/// blank line — yields nothing rather than a spurious match.
#[cfg(any(windows, test))]
fn tasklist_image_name(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('"')?;
    let (image, after) = rest.split_once('"')?;
    after.starts_with(',').then(|| image.to_ascii_lowercase())
}

/// Derive the product-specific executable within one identified installation.
#[must_use]
pub fn default_client(install_root: &Path, target: TargetGame, client: ClientKind) -> PathBuf {
    let filename = match (client, target) {
        (ClientKind::OpenMohaa, _) => "openmohaa.exe",
        (ClientKind::Retail | ClientKind::Reborn, TargetGame::AlliedAssault) => "MOHAA.exe",
        (ClientKind::Retail | ClientKind::Reborn, TargetGame::Spearhead) => "moh_spearhead.exe",
        (ClientKind::Retail | ClientKind::Reborn, TargetGame::Breakthrough) => {
            "moh_breakthrough.exe"
        }
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
/// Returns an error for an unwritable retail directory, or when the `OpenMoHAA` fallback cannot
/// be located, created, and verified writable.
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
        Err(source) if client != ClientKind::OpenMohaa => Err(PlatformError::RetailUnwritable {
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

impl From<EngineChoice> for ClientKind {
    fn from(value: EngineChoice) -> Self {
        match value {
            EngineChoice::Original => Self::Retail,
            EngineChoice::Openmohaa => Self::OpenMohaa,
            EngineChoice::Reborn => Self::Reborn,
        }
    }
}

pub mod engine;

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
/// Returns an error when the process cannot be spawned.
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

/// Windows launcher policy failure.
#[derive(Debug, Error)]
pub enum PlatformError {
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
    fn tasklist_parser_is_exact_and_ignores_a_localised_empty_result() {
        for stem in openmohaa::RELEASE_EXECUTABLE_STEMS {
            let row = format!("\"{stem}.exe\",\"8120\",\"Console\",\"1\",\"42,000 K\"");
            assert!(
                tasklist_release_activity(&row).client_activity() == ClientActivity::Running,
                "{stem}.exe holds a lock on the installation"
            );
        }
        let mixed = tasklist_release_activity(
            "\"explorer.exe\",\"1\",\"Console\",\"1\",\"1 K\"
\"OMohAADed.EXE\",\"2\",\"Console\",\"1\",\"1 K\"",
        );
        assert_eq!(
            mixed.running_programs(),
            &[OpenMohaaProgram::DedicatedServer]
        );
        for output in [
            "\"not-openmohaa.exe\",\"8120\",\"Console\",\"1\",\"42,000 K\"",
            "\"openmohaa.exe.bak\",\"8120\",\"Console\",\"1\",\"42,000 K\"",
        ] {
            assert_eq!(
                tasklist_release_activity(output).client_activity(),
                ClientActivity::ConfirmedStopped
            );
        }
        for output in [
            "Information : aucune tâche en cours ne correspond aux critères spécifiés.",
            "\"openmohaa.exe\"",
        ] {
            assert_eq!(
                tasklist_release_activity(output).client_activity(),
                ClientActivity::Unknown
            );
        }
    }

    #[test]
    fn tasklist_activity_preserves_which_programs_were_observed() {
        let activity = tasklist_release_activity(
            "\"openmohaa.exe\",\"1\",\"Console\",\"1\",\"1 K\"\n\
             \"omohaaded.exe\",\"2\",\"Console\",\"1\",\"1 K\"\n\
             \"launch_openmohaa_base.exe\",\"3\",\"Console\",\"1\",\"1 K\"",
        );
        assert_eq!(
            activity.running_programs(),
            &[
                OpenMohaaProgram::Game,
                OpenMohaaProgram::DedicatedServer,
                OpenMohaaProgram::Launcher
            ]
        );
        assert_eq!(activity.client_activity(), ClientActivity::Running);
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
        assert_eq!(
            default_client(root, TargetGame::AlliedAssault, ClientKind::Reborn),
            root.join("MOHAA.exe")
        );
        assert_eq!(ClientKind::Reborn.dialect(), LaunchDialect::Retail);
    }

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
