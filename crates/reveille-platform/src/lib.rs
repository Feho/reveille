// SPDX-License-Identifier: GPL-2.0-only

//! Shared launcher and content-path policy for Reveille's executable front ends.
//!
//! Windows remains the packaged v1 target (NSIS). macOS uses the same overlay install of the
//! official `OpenMoHAA` archive into a player-picked folder that already has `main/`; the Reveille
//! app itself is built as `.app`/`.dmg` there, not as NSIS. Linux keeps the crate portable for CI.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;

use std::env;
use std::fs::{self, OpenOptions};
use std::process::Command;

use reveille_core::discovery::TargetGame;
use reveille_core::engine::EngineChoice;
use reveille_core::join::{LaunchCommand, LaunchDialect, LaunchProfile};
#[cfg(any(unix, test))]
use reveille_core::platform::openmohaa;
use reveille_core::platform::openmohaa::ClientActivity;
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

/// Select `OpenMoHAA` when its client or a product launcher exists, otherwise retain retail.
#[must_use]
pub fn detect_client(install_root: &Path) -> ClientKind {
    if openmohaa_is_installed(install_root) {
        ClientKind::OpenMohaa
    } else {
        ClientKind::Retail
    }
}

/// Whether this folder has a playable `OpenMoHAA` client, not only `openmohaa.exe`.
///
/// Official archives install the Windows client as `openmohaa.exe` and every Unix archive as
/// the extensionless `openmohaa` binary plus `launch_openmohaa_*` helpers
/// (`reveille_core::platform::openmohaa::RELEASE_EXECUTABLE_STEMS`). A dedicated server alone
/// is not enough: that is not a client the player can join with.
#[must_use]
pub fn openmohaa_is_installed(install_root: &Path) -> bool {
    openmohaa_client_markers().any(|name| install_root.join(name).is_file())
}

fn openmohaa_client_markers() -> impl Iterator<Item = &'static str> {
    [
        "openmohaa.exe",
        "openmohaa",
        "launch_openmohaa_base",
        "launch_openmohaa_base.exe",
        "launch_openmohaa_spearhead",
        "launch_openmohaa_spearhead.exe",
        "launch_openmohaa_breakthrough",
        "launch_openmohaa_breakthrough.exe",
    ]
    .into_iter()
}

fn openmohaa_unix_launcher(target: TargetGame) -> &'static str {
    match target {
        TargetGame::AlliedAssault => "launch_openmohaa_base",
        TargetGame::Spearhead => "launch_openmohaa_spearhead",
        TargetGame::Breakthrough => "launch_openmohaa_breakthrough",
    }
}

/// Build the process-listing command, suppressing the console window it would otherwise flash.
///
/// `tasklist` is a console program, so a GUI front end spawning it inherits no console and
/// Windows creates one — a black window that blinks on screen for the length of the query.
/// `CREATE_NO_WINDOW` = `0x0800_0000`; learn.microsoft.com/windows/win32/procthread/process-creation-flags
#[cfg(windows)]
fn tasklist_command() -> Command {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = Command::new("tasklist");
    command
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW);
    command
}

/// Conservatively report whether an `OpenMoHAA` installation can be replaced.
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
        let Ok(output) = tasklist_command().output() else {
            return OpenMohaaActivity::unknown();
        };
        if !output.status.success() {
            return OpenMohaaActivity::unknown();
        }
        tasklist_release_activity(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(unix)]
    {
        // `ps -ax -o comm=` is available on both macOS and Linux. The `=` suppresses the header,
        // so every non-empty line is a command name. A failed spawn or non-zero status is
        // Unknown, never ConfirmedStopped.
        let Ok(output) = Command::new("ps").args(["-ax", "-o", "comm="]).output() else {
            return OpenMohaaActivity::unknown();
        };
        if !output.status.success() {
            return OpenMohaaActivity::unknown();
        }
        unix_comm_release_activity(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(any(windows, unix)))]
    {
        OpenMohaaActivity::unknown()
    }
}

fn classify_release_stem(stem: &str) -> Option<OpenMohaaProgram> {
    match stem {
        "openmohaa" => Some(OpenMohaaProgram::Game),
        "omohaaded" => Some(OpenMohaaProgram::DedicatedServer),
        "launch_openmohaa_base"
        | "launch_openmohaa_spearhead"
        | "launch_openmohaa_breakthrough" => Some(OpenMohaaProgram::Launcher),
        _ => None,
    }
}

/// Unix `ps -o comm` truncates to 15 characters on Linux. Every `OpenMoHAA` launcher stem is
/// longer than that and shares the prefix `launch_openmohaa`; `openmohaa` and `omohaaded` fit.
#[cfg(any(unix, test))]
fn classify_process_comm(comm: &str) -> Option<OpenMohaaProgram> {
    let name = Path::new(comm)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(comm);
    let stem = name
        .strip_suffix(".exe")
        .unwrap_or(name)
        .to_ascii_lowercase();
    if let Some(program) = classify_release_stem(&stem) {
        return Some(program);
    }
    openmohaa::RELEASE_EXECUTABLE_STEMS
        .iter()
        .find_map(|known| {
            (stem.len() >= 15 && known.len() > stem.len() && known.starts_with(&stem))
                .then(|| classify_release_stem(known))
                .flatten()
        })
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
        let Some(stem) = image.strip_suffix(".exe") else {
            continue;
        };
        if let Some(program) = classify_release_stem(stem) {
            activity.observe(program);
        }
    }
    if !output.trim().is_empty() && valid_rows == 0 {
        return OpenMohaaActivity::unknown();
    }
    activity
}

/// Same lock set as [`tasklist_release_activity`], from `ps -ax -o comm=` lines.
#[cfg(any(unix, test))]
fn unix_comm_release_activity(output: &str) -> OpenMohaaActivity {
    let mut activity = OpenMohaaActivity::checked();
    for comm in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(program) = classify_process_comm(comm) {
            activity.observe(program);
        }
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
///
/// Windows `OpenMoHAA` still launches `openmohaa.exe` and selects the product with
/// `+set com_target_game`. Unix archives ship `launch_openmohaa_base` /
/// `launch_openmohaa_spearhead` / `launch_openmohaa_breakthrough` for that job; those are
/// preferred when present, with a fallback to the `openmohaa` binary so a folder that only has
/// the client remains launchable.
#[must_use]
pub fn default_client(install_root: &Path, target: TargetGame, client: ClientKind) -> PathBuf {
    match client {
        ClientKind::OpenMohaa => openmohaa_launch_program(install_root, target),
        ClientKind::Retail | ClientKind::Reborn => {
            let filename = match target {
                TargetGame::AlliedAssault => "MOHAA.exe",
                TargetGame::Spearhead => "moh_spearhead.exe",
                TargetGame::Breakthrough => "moh_breakthrough.exe",
            };
            install_root.join(filename)
        }
    }
}

fn openmohaa_launch_program(install_root: &Path, target: TargetGame) -> PathBuf {
    if cfg!(windows) {
        return install_root.join("openmohaa.exe");
    }
    let launcher = install_root.join(openmohaa_unix_launcher(target));
    let client = install_root.join("openmohaa");
    if launcher.is_file() || !client.is_file() {
        launcher
    } else {
        client
    }
}

/// Directory name `OpenMoHAA` appends under the platform home root.
///
/// `Sys_DefaultHomePath` appends `com_homepath`, empty for a non-demo build (`common.c:1771`),
/// and otherwise `HOMEPATH_NAME` — `"openmohaa"` (`q_shared.h:81`). Windows uses `%APPDATA%`
/// (`sys_win32.c:97-120`); macOS uses `$HOME/Library/Application Support/` (`sys_unix.c:130-140`
/// under `__APPLE__`); Linux uses `$XDG_DATA_HOME` or `$HOME/.local/share` (`sys_unix.c:194-205`).
/// The per-product `HOMEPATH_NAME_WIN_MOH*` defines name `moh`, `mohta` and `mohtt` but are
/// referenced nowhere in the engine; one home path serves all three target games, with `main`,
/// `mainta` and `maintt` inside it.
const OPENMOHAA_HOME_DIRECTORY: &str = "openmohaa";

/// `OpenMoHAA`'s home path on this machine, when the environment names one.
///
/// The engine searches this path in addition to the installation, whether or not Reveille writes
/// there, and it wins over the installation for any file present in both. Reveille does not pass
/// `fs_homepath`; it follows this default so maps written as a fallback land where the engine
/// already looks.
#[must_use]
pub fn openmohaa_home_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("APPDATA")
            .map(|app_data| PathBuf::from(app_data).join(OPENMOHAA_HOME_DIRECTORY))
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(OPENMOHAA_HOME_DIRECTORY)
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg).join(OPENMOHAA_HOME_DIRECTORY));
            }
        }
        env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join(OPENMOHAA_HOME_DIRECTORY)
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// The directories one target game reads **from the selected installation**, lowest precedence
/// first.
///
/// Two things are composed here. `LaunchProfile::search_directories` gives the game directories
/// — `main` alone, or `main` then the expansion's — and `FS_AddGameDirectories`
/// (`files.cpp:3246-3257`) adds each of them under every base path, the home path last, so the
/// home copy of a directory outranks the installed one. Directories that do not exist are left
/// out rather than reported: the engine simply finds nothing in them.
///
/// **This is the selected installation and the home path, not every path the engine can read.**
/// `FS_InitPathVars` (`files.cpp:3562-3573`) also registers `fs_steampath`, `fs_gogpath` and
/// `fs_microsoftstorepath`, and `FS_Startup` adds a non-empty `fs_game` on top
/// (`files.cpp:3647-3650`). A map that exists only in a *second*, unselected installation, or
/// only inside a server-published mod directory, is therefore loadable by the engine and absent
/// from this list — Reveille would report it missing. Recorded as a known limit rather than
/// guessed at: modelling the other roots means deciding which of several installations the
/// player meant, which is the question setup already asked them.
#[must_use]
pub fn content_search_path(
    install_root: &Path,
    target: TargetGame,
    client: ClientKind,
) -> Vec<PathBuf> {
    // Retail and Reborn read no home path: `fs_homepath` is an OpenMoHAA-era addition.
    let home = (client == ClientKind::OpenMohaa)
        .then(openmohaa_home_root)
        .flatten();
    search_path_with_home(install_root, target, home.as_deref())
}

/// The chain itself, with the home root supplied rather than read from the environment, so the
/// ordering can be tested without depending on what exists on the machine running the test.
fn search_path_with_home(
    install_root: &Path,
    target: TargetGame,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    LaunchProfile::new(target)
        .search_directories()
        .iter()
        .flat_map(|directory| {
            [
                Some(install_root.join(directory)),
                home.map(|home| home.join(directory)),
            ]
        })
        .flatten()
        .filter(|directory| directory.is_dir())
        .collect()
}

/// Writable game directory selected for downloaded content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallTarget {
    /// Directory into which archives should be installed.
    pub game_directory: PathBuf,
    /// Whether an unwritable `OpenMoHAA` installation required the home path.
    pub used_home_fallback: bool,
}

/// Probe the conventional install directory and apply the `OpenMoHAA`-only home fallback.
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
            let fallback = openmohaa_home_root()
                .ok_or(PlatformError::MissingAppData)?
                .join(data_directory);
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
    // Retail and Reborn select the product by executable, so an install without the expansion's
    // client has no file to spawn at all — the ordinary case for a folder that has Allied
    // Assault and not Spearhead, and worth naming rather than reporting as a generic failure.
    // The classification happens *after* the spawn attempt, never before it: `Command` resolves a
    // bare program name against `PATH`, which `Path::is_file` does not, and the CLI's default
    // client is the bare name `openmohaa`.
    process.spawn().map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PlatformError::ClientMissing {
                program: program.to_path_buf(),
                target: command.profile.target,
            }
        } else {
            PlatformError::Launch {
                program: program.to_path_buf(),
                source,
            }
        }
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
    #[error("the OpenMoHAA home path is unavailable")]
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
    /// The product's client executable is not in the installation.
    #[error("{target} cannot start because its game program was not found at {program}")]
    ClientMissing {
        /// Executable the selected product and engine resolve to.
        program: PathBuf,
        /// Product whose client is missing.
        target: TargetGame,
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
    fn detects_openmohaa_from_the_unix_client_or_a_launcher() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        assert_eq!(detect_client(root), ClientKind::Retail);

        fs::write(root.join("openmohaa"), []).expect("unix client");
        assert_eq!(detect_client(root), ClientKind::OpenMohaa);
        fs::remove_file(root.join("openmohaa")).expect("remove unix client");

        fs::write(root.join("launch_openmohaa_base"), []).expect("unix launcher");
        assert_eq!(detect_client(root), ClientKind::OpenMohaa);
        fs::remove_file(root.join("launch_openmohaa_base")).expect("remove unix launcher");

        fs::write(root.join("launch_openmohaa_spearhead.exe"), []).expect("windows launcher");
        assert_eq!(detect_client(root), ClientKind::OpenMohaa);
        fs::write(root.join("omohaaded"), []).expect("dedicated server is not a client");
        fs::remove_file(root.join("launch_openmohaa_spearhead.exe")).expect("remove launcher");
        assert_eq!(detect_client(root), ClientKind::Retail);
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
    fn unix_process_list_matches_bare_names_paths_and_truncated_comms() {
        let running = unix_comm_release_activity(
            "zsh\n\
             /opt/OpenMoHAA/openmohaa\n\
             launch_openmoha\n\
             omohaaded\n",
        );
        assert_eq!(
            running.running_programs(),
            &[
                OpenMohaaProgram::Game,
                OpenMohaaProgram::Launcher,
                OpenMohaaProgram::DedicatedServer
            ]
        );
        assert_eq!(
            unix_comm_release_activity("zsh\nfinder\n").client_activity(),
            ClientActivity::ConfirmedStopped
        );
        assert_eq!(
            unix_comm_release_activity("openmohaa.bak\nnot-openmohaa\n").client_activity(),
            ClientActivity::ConfirmedStopped
        );
        assert_eq!(
            classify_process_comm("launch_openmohaa_breakthrough"),
            Some(OpenMohaaProgram::Launcher)
        );
        assert_eq!(classify_process_comm("open"), None);
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
            default_client(root, TargetGame::AlliedAssault, ClientKind::Reborn),
            root.join("MOHAA.exe")
        );
        assert_eq!(ClientKind::Reborn.dialect(), LaunchDialect::Retail);
    }

    #[cfg(windows)]
    #[test]
    fn windows_openmohaa_launches_openmohaa_exe() {
        let root = Path::new(r"C:\Games\MOHAA");
        assert_eq!(
            default_client(root, TargetGame::Spearhead, ClientKind::OpenMohaa),
            root.join("openmohaa.exe")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_openmohaa_launches_the_product_helper() {
        let root = Path::new("/games/mohaa");
        assert_eq!(
            default_client(root, TargetGame::AlliedAssault, ClientKind::OpenMohaa),
            root.join("launch_openmohaa_base")
        );
        assert_eq!(
            default_client(root, TargetGame::Spearhead, ClientKind::OpenMohaa),
            root.join("launch_openmohaa_spearhead")
        );
        assert_eq!(
            default_client(root, TargetGame::Breakthrough, ClientKind::OpenMohaa),
            root.join("launch_openmohaa_breakthrough")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_openmohaa_falls_back_to_the_client_when_the_launcher_is_absent() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::write(root.join("openmohaa"), b"client").expect("unix client");
        assert_eq!(
            default_client(root, TargetGame::Spearhead, ClientKind::OpenMohaa),
            root.join("openmohaa")
        );
        fs::write(root.join("launch_openmohaa_spearhead"), b"launcher").expect("launcher");
        assert_eq!(
            default_client(root, TargetGame::Spearhead, ClientKind::OpenMohaa),
            root.join("launch_openmohaa_spearhead")
        );
    }

    #[test]
    fn an_expansion_search_path_keeps_main_underneath_it() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::create_dir(root.join("main")).expect("main directory");
        fs::create_dir(root.join("mainta")).expect("mainta directory");

        assert_eq!(
            content_search_path(root, TargetGame::AlliedAssault, ClientKind::Retail),
            [root.join("main")]
        );
        // Lowest precedence first: `mainta` is added after `main` and therefore searched before it.
        assert_eq!(
            content_search_path(root, TargetGame::Spearhead, ClientKind::Retail),
            [root.join("main"), root.join("mainta")]
        );
        // Breakthrough has no `maintt` here, and a directory that does not exist is left out
        // rather than reported: the engine simply finds nothing in it.
        assert_eq!(
            content_search_path(root, TargetGame::Breakthrough, ClientKind::Retail),
            [root.join("main")]
        );
    }

    #[test]
    fn the_home_copy_of_a_directory_outranks_the_installed_one() {
        // The home root is supplied rather than read from `%APPDATA%`, so this asserts the engine
        // ordering itself instead of whatever happens to exist on the machine running it.
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path().join("install");
        let home = temporary.path().join("home");
        for directory in [&root, &home] {
            fs::create_dir_all(directory.join("main")).expect("main directory");
            fs::create_dir_all(directory.join("mainta")).expect("mainta directory");
        }

        // Lowest precedence first, so this reads: main, home main, mainta, home mainta — and the
        // engine loads the last of them first. A later game directory outranks every path of an
        // earlier one (`files.cpp:3640-3645` with `3246-3257`).
        assert_eq!(
            search_path_with_home(&root, TargetGame::Spearhead, Some(&home)),
            [
                root.join("main"),
                home.join("main"),
                root.join("mainta"),
                home.join("mainta"),
            ]
        );
    }

    #[test]
    fn only_openmohaa_reads_a_home_path() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::create_dir(root.join("main")).expect("main directory");

        // Retail and Reborn predate the home path split, so the choice of client decides whether
        // one is consulted at all. `openmohaa_home_root` is environment-dependent; what is
        // asserted here is only that the retail path never grows beyond the installation.
        assert_eq!(
            content_search_path(root, TargetGame::AlliedAssault, ClientKind::Retail),
            [root.join("main")]
        );
        assert!(
            content_search_path(root, TargetGame::AlliedAssault, ClientKind::OpenMohaa)
                .starts_with(&[root.join("main")])
        );
    }

    #[test]
    fn a_bare_program_name_is_resolved_rather_than_treated_as_a_path() {
        // `Command` resolves a bare name against `PATH`; `Path::is_file` does not. The CLI's
        // default join client is the bare name `openmohaa`, so a pre-spawn path check refuses a
        // client that is installed. This launches a program that is certainly on `PATH` and
        // certainly not a file in the working directory, and kills it immediately; what is being
        // asserted is that it started at all.
        let program = if cfg!(windows) { "cmd" } else { "sh" };
        let command = LaunchCommand::new(
            program,
            LaunchProfile::new(TargetGame::AlliedAssault),
            reveille_core::join::FsGame::new("").expect("empty mod directory"),
            std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(203, 0, 113, 7), 12_203),
        )
        .expect("launch command");

        let mut child = launch_client(&command, ClientKind::Retail)
            .expect("a program on PATH is not a missing client");
        drop(child.kill());
        drop(child.wait());
    }

    #[test]
    fn an_install_without_the_expansion_client_says_which_program_is_missing() {
        let temporary = TempDir::new().expect("temporary directory");
        let program = default_client(temporary.path(), TargetGame::Spearhead, ClientKind::Retail);
        let command = LaunchCommand::new(
            program.to_string_lossy().into_owned(),
            LaunchProfile::new(TargetGame::Spearhead),
            reveille_core::join::FsGame::new("").expect("empty mod directory"),
            std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(203, 0, 113, 7), 12_203),
        )
        .expect("launch command");

        assert!(matches!(
            launch_client(&command, ClientKind::Retail),
            Err(PlatformError::ClientMissing {
                target: TargetGame::Spearhead,
                ..
            })
        ));
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
