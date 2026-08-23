// SPDX-License-Identifier: GPL-2.0-only

//! Per-installation engine selection and transactional Original/Reborn activation.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use reveille_core::engine::EngineChoice;
use reveille_core::platform::package::{self as package_io, OverlayFile};
use reveille_core::platform::reborn::{
    RebornExecutable, RebornPackage, SOURCE_COMMIT, expected_executable_sha256,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

const MANAGED_DIRECTORY: &str = ".reveille-engines";
const STATE_FORMAT: u8 = 1;
const RETAIL_EXECUTABLES: [&str; 3] = ["MOHAA.exe", "moh_spearhead.exe", "moh_breakthrough.exe"];

/// Conservative process-list state for canonical retail/Reborn programs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineActivity {
    ConfirmedStopped,
    Running(Vec<String>),
    Unknown,
}

/// Installed and selected state shown by setup.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the UI needs independent installed/current facts for three coexistable engines"
)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EngineInventory {
    pub original_installed: bool,
    pub openmohaa_installed: bool,
    pub reborn_installed: bool,
    pub reborn_current: bool,
    pub reborn_build: RebornInstalledBuild,
    pub selected: Option<EngineChoice>,
}

/// Evidence-backed identity of managed or active Reborn files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RebornInstalledBuild {
    Absent,
    Current,
    KnownOther { version: String },
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StateFile {
    format: u8,
    selected: Option<EngineChoice>,
    #[serde(default)]
    original: BTreeMap<String, FileReceipt>,
    reborn: Option<RebornReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FileReceipt {
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RebornReceipt {
    version: String,
    source_commit: String,
    filename: String,
    archive_sha256: String,
    files: BTreeMap<String, FileReceipt>,
}

/// Query the Windows task list for any canonical retail/Reborn executable.
#[must_use]
pub fn retail_activity() -> EngineActivity {
    #[cfg(windows)]
    {
        let Ok(output) = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
        else {
            return EngineActivity::Unknown;
        };
        if !output.status.success() {
            return EngineActivity::Unknown;
        }
        parse_tasklist(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(windows))]
    {
        EngineActivity::Unknown
    }
}

#[cfg(any(windows, test))]
fn parse_tasklist(output: &str) -> EngineActivity {
    let mut running = Vec::new();
    let mut valid_rows = 0_usize;
    for line in output.lines() {
        let Some(rest) = line.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some((image, after)) = rest.split_once('"') else {
            continue;
        };
        if !after.starts_with(',') {
            continue;
        }
        valid_rows += 1;
        if RETAIL_EXECUTABLES
            .iter()
            .any(|known| image.eq_ignore_ascii_case(known))
            && !running
                .iter()
                .any(|known: &String| known.eq_ignore_ascii_case(image))
        {
            running.push(image.to_owned());
        }
    }
    if !output.trim().is_empty() && valid_rows == 0 {
        EngineActivity::Unknown
    } else if running.is_empty() {
        EngineActivity::ConfirmedStopped
    } else {
        EngineActivity::Running(running)
    }
}

/// Inspect available engines and validate receipts against managed files.
#[must_use]
pub fn inventory(root: &Path) -> EngineInventory {
    let state = read_state(root).ok().flatten();
    let original_installed = RETAIL_EXECUTABLES.iter().any(|name| {
        root.join(MANAGED_DIRECTORY)
            .join("original")
            .join(name)
            .is_file()
            || (root.join(name).is_file()
                && hash_file(&root.join(name)).ok().as_deref() != expected_executable_sha256(name))
    });
    let openmohaa_installed = root.join("openmohaa.exe").is_file();
    let receipt_valid = state
        .as_ref()
        .and_then(|state| state.reborn.as_ref())
        .is_some_and(|receipt| validate_receipt_files(root, receipt));
    let canonical_reborn = hash_file(&root.join("MOHAA.exe")).ok().as_deref()
        == expected_executable_sha256("MOHAA.exe");
    let reborn_current = canonical_reborn
        || (receipt_valid
            && state
                .as_ref()
                .and_then(|state| state.reborn.as_ref())
                .is_some_and(|receipt| receipt.source_commit == SOURCE_COMMIT));
    let reborn_build = if reborn_current {
        RebornInstalledBuild::Current
    } else if receipt_valid {
        RebornInstalledBuild::KnownOther {
            version: state
                .as_ref()
                .and_then(|state| state.reborn.as_ref())
                .map_or_else(
                    || "known package".to_owned(),
                    |receipt| receipt.version.clone(),
                ),
        }
    } else if state.as_ref().is_some_and(|state| state.reborn.is_some())
        || RETAIL_EXECUTABLES.iter().any(|name| {
            root.join(MANAGED_DIRECTORY)
                .join("reborn")
                .join(name)
                .is_file()
        })
    {
        RebornInstalledBuild::Unknown
    } else {
        RebornInstalledBuild::Absent
    };
    let reborn_installed = !matches!(reborn_build, RebornInstalledBuild::Absent);
    EngineInventory {
        original_installed,
        openmohaa_installed,
        reborn_installed,
        reborn_current,
        reborn_build,
        selected: state.and_then(|state| state.selected),
    }
}

/// Resolve selection without silently replacing an unavailable saved choice.
///
/// # Errors
///
/// Returns an error for an unavailable requested/saved engine or an ambiguous first selection.
pub fn resolve_choice(
    root: &Path,
    requested: Option<EngineChoice>,
) -> Result<EngineChoice, EngineError> {
    let inventory = inventory(root);
    if let Some(choice) = requested {
        ensure_available(choice, &inventory)?;
        return Ok(choice);
    }
    if let Some(saved) = inventory.selected {
        ensure_available(saved, &inventory)
            .map_err(|_| EngineError::SavedChoiceUnavailable(saved))?;
        return Ok(saved);
    }
    match (inventory.openmohaa_installed, inventory.reborn_installed) {
        (true, false) => Ok(EngineChoice::Openmohaa),
        (false, true) => Ok(EngineChoice::Reborn),
        (false, false) => Ok(EngineChoice::Original),
        (true, true) => Err(EngineError::ChoiceRequired),
    }
}

/// Store and activate one already-installed engine.
///
/// # Errors
///
/// Returns an error for unavailable engines, unsafe process state, or transactional I/O failure.
pub fn activate(
    root: &Path,
    choice: EngineChoice,
    activity: EngineActivity,
) -> Result<(), EngineError> {
    let inventory = inventory(root);
    ensure_available(choice, &inventory)?;
    if matches!(choice, EngineChoice::Original | EngineChoice::Reborn) {
        require_stopped(activity)?;
        let source = root.join(MANAGED_DIRECTORY).join(match choice {
            EngineChoice::Original => "original",
            EngineChoice::Reborn => "reborn",
            EngineChoice::Openmohaa => unreachable!(),
        });
        let files = RETAIL_EXECUTABLES
            .iter()
            .filter(|name| source.join(name).is_file())
            .map(|name| (source.join(name), root.join(name)))
            .collect::<Vec<_>>();
        if files.is_empty() && choice == EngineChoice::Original {
            // A never-switched retail install is already active.
        } else {
            transactional_copy(&files)?;
        }
    }
    let mut state = read_state(root)?.unwrap_or_else(|| StateFile {
        format: STATE_FORMAT,
        ..StateFile::default()
    });
    state.selected = Some(choice);
    write_state(root, &state)
}

/// Preserve first-seen originals, retain verified Reborn copies, activate them, and record hashes.
///
/// # Errors
///
/// Returns an error before writes for unsafe process state, or after rolling canonical files back
/// when a transactional overlay cannot complete.
pub fn install_reborn(
    root: &Path,
    package: &RebornPackage,
    executables: &[RebornExecutable],
    activity: EngineActivity,
) -> Result<(), EngineError> {
    require_stopped(activity)?;
    let managed = root.join(MANAGED_DIRECTORY);
    let original = managed.join("original");
    let reborn = managed.join("reborn");
    fs::create_dir_all(&original).map_err(|source| EngineError::Filesystem {
        path: original.clone(),
        source,
    })?;
    fs::create_dir_all(&reborn).map_err(|source| EngineError::Filesystem {
        path: reborn.clone(),
        source,
    })?;

    let mut state = read_state(root)?.unwrap_or_else(|| StateFile {
        format: STATE_FORMAT,
        ..StateFile::default()
    });
    for executable in executables {
        let canonical = root.join(&executable.filename);
        let backup = original.join(&executable.filename);
        let canonical_is_current_reborn = hash_file(&canonical).ok().as_deref()
            == expected_executable_sha256(&executable.filename);
        if canonical.is_file() && !canonical_is_current_reborn && !backup.exists() {
            copy_noclobber(&canonical, &backup)?;
            state.original.insert(
                executable.filename.clone(),
                FileReceipt {
                    sha256: hash_file(&backup)?,
                },
            );
        }
    }

    let managed_files = executables
        .iter()
        .map(|executable| {
            let source = write_staging(&managed, &executable.bytes)?;
            Ok((source, reborn.join(&executable.filename)))
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    let managed_result = transactional_copy(&managed_files);
    for (source, _) in &managed_files {
        drop(fs::remove_file(source));
    }
    managed_result?;

    let files = executables
        .iter()
        .map(|executable| {
            (
                reborn.join(&executable.filename),
                root.join(&executable.filename),
            )
        })
        .collect::<Vec<_>>();
    transactional_copy(&files)?;
    state.reborn = Some(RebornReceipt {
        version: package.version.to_owned(),
        source_commit: SOURCE_COMMIT.to_owned(),
        filename: package.filename.clone(),
        archive_sha256: package.sha256.to_owned(),
        files: executables
            .iter()
            .map(|file| {
                (
                    file.filename.clone(),
                    FileReceipt {
                        sha256: file.sha256.clone(),
                    },
                )
            })
            .collect(),
    });
    state.selected = Some(EngineChoice::Reborn);
    write_state(root, &state)
}

fn require_stopped(activity: EngineActivity) -> Result<(), EngineError> {
    match activity {
        EngineActivity::ConfirmedStopped => Ok(()),
        EngineActivity::Running(programs) => Err(EngineError::ProgramsRunning(programs)),
        EngineActivity::Unknown => Err(EngineError::ProcessStateUnknown),
    }
}

fn ensure_available(choice: EngineChoice, inventory: &EngineInventory) -> Result<(), EngineError> {
    let available = match choice {
        EngineChoice::Original => inventory.original_installed,
        EngineChoice::Openmohaa => inventory.openmohaa_installed,
        EngineChoice::Reborn => inventory.reborn_installed,
    };
    available
        .then_some(())
        .ok_or(EngineError::Unavailable(choice))
}

fn validate_receipt_files(root: &Path, receipt: &RebornReceipt) -> bool {
    !receipt.files.is_empty()
        && receipt.files.iter().all(|(name, expected)| {
            hash_file(&root.join(MANAGED_DIRECTORY).join("reborn").join(name))
                .ok()
                .as_ref()
                == Some(&expected.sha256)
        })
}

fn state_path(root: &Path) -> PathBuf {
    root.join(MANAGED_DIRECTORY).join("state.json")
}

fn read_state(root: &Path) -> Result<Option<StateFile>, EngineError> {
    let path = state_path(root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(EngineError::Filesystem { path, source }),
    };
    let state: StateFile = serde_json::from_slice(&bytes)?;
    if state.format != STATE_FORMAT {
        return Err(EngineError::UnsupportedState(state.format));
    }
    Ok(Some(state))
}

fn write_state(root: &Path, state: &StateFile) -> Result<(), EngineError> {
    let path = state_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| EngineError::NoParent(path.clone()))?;
    fs::create_dir_all(parent).map_err(|source| EngineError::Filesystem {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|source| EngineError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
    serde_json::to_writer_pretty(&mut temporary, state)?;
    temporary
        .flush()
        .map_err(|source| EngineError::Filesystem {
            path: path.clone(),
            source,
        })?;
    temporary
        .persist(&path)
        .map_err(|error| EngineError::Filesystem {
            path,
            source: error.error,
        })?;
    Ok(())
}

fn copy_noclobber(source: &Path, target: &Path) -> Result<(), EngineError> {
    let mut input = File::open(source).map_err(|source_error| EngineError::Filesystem {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|source_error| EngineError::Filesystem {
            path: target.to_path_buf(),
            source: source_error,
        })?;
    io::copy(&mut input, &mut output).map_err(|source_error| EngineError::Filesystem {
        path: target.to_path_buf(),
        source: source_error,
    })?;
    output
        .flush()
        .map_err(|source_error| EngineError::Filesystem {
            path: target.to_path_buf(),
            source: source_error,
        })
}

fn write_staging(directory: &Path, bytes: &[u8]) -> Result<PathBuf, EngineError> {
    let mut file = NamedTempFile::new_in(directory).map_err(|source| EngineError::Filesystem {
        path: directory.to_path_buf(),
        source,
    })?;
    file.write_all(bytes)
        .map_err(|source| EngineError::Filesystem {
            path: directory.to_path_buf(),
            source,
        })?;
    let (_handle, path) = file.keep().map_err(|error| EngineError::Filesystem {
        path: directory.to_path_buf(),
        source: error.error,
    })?;
    Ok(path)
}

fn transactional_copy(files: &[(PathBuf, PathBuf)]) -> Result<(), EngineError> {
    let overlays = files
        .iter()
        .map(|(source, target)| OverlayFile {
            source,
            target: target.clone(),
            executable: false,
        })
        .collect::<Vec<_>>();
    package_io::transactional_overlay(&overlays).map_err(EngineError::PackageApply)
}

fn hash_file(path: &Path) -> Result<String, EngineError> {
    let mut file = File::open(path).map_err(|source| EngineError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| EngineError::Filesystem {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("both OpenMoHAA and Reborn are installed; choose which engine to use")]
    ChoiceRequired,
    #[error("saved engine {0:?} is no longer available; choose another engine explicitly")]
    SavedChoiceUnavailable(EngineChoice),
    #[error("selected engine {0:?} is not available")]
    Unavailable(EngineChoice),
    #[error("close these game programs before changing engines: {0:?}")]
    ProgramsRunning(Vec<String>),
    #[error("the running-game check did not complete; engine files were not changed")]
    ProcessStateUnknown,
    #[error("unsupported engine state format {0}")]
    UnsupportedState(u8),
    #[error("engine state is malformed")]
    Json(#[from] serde_json::Error),
    #[error("path has no parent: {0}")]
    NoParent(PathBuf),
    #[error("engine overlay failed")]
    PackageApply(#[source] package_io::ApplyError),
    #[error("filesystem operation failed at {path}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn tasklist_is_case_insensitive_and_conservative_about_malformed_rows() {
        assert_eq!(
            parse_tasklist("garbage\n\"MOHAA.exe\""),
            EngineActivity::Unknown
        );
        assert_eq!(
            parse_tasklist("\"explorer.exe\",\"1\""),
            EngineActivity::ConfirmedStopped
        );
        assert_eq!(
            parse_tasklist("\"MoHaA.ExE\",\"1\",\"Console\",\"1\",\"1 K\""),
            EngineActivity::Running(vec!["MoHaA.ExE".to_owned()])
        );
        assert_eq!(
            parse_tasklist("\"moh_spearhead.exe\",\"1\"\n\"MOH_BREAKTHROUGH.EXE\",\"2\""),
            EngineActivity::Running(vec![
                "moh_spearhead.exe".to_owned(),
                "MOH_BREAKTHROUGH.EXE".to_owned()
            ])
        );
    }

    #[test]
    fn choice_defaults_and_never_falls_back_from_an_unavailable_saved_choice() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::write(root.join("MOHAA.exe"), b"retail").expect("retail");
        assert_eq!(
            resolve_choice(root, None).expect("original default"),
            EngineChoice::Original
        );
        fs::write(root.join("openmohaa.exe"), b"open").expect("openmohaa");
        assert_eq!(
            resolve_choice(root, None).expect("sole community engine"),
            EngineChoice::Openmohaa
        );
        activate(root, EngineChoice::Openmohaa, EngineActivity::Unknown).expect("select openmohaa");
        fs::remove_file(root.join("openmohaa.exe")).expect("remove selected engine");
        assert!(matches!(
            resolve_choice(root, None),
            Err(EngineError::SavedChoiceUnavailable(EngineChoice::Openmohaa))
        ));
    }

    #[test]
    fn reborn_install_preserves_original_once_and_switches_both_directions() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::write(root.join("MOHAA.exe"), b"original").expect("original client");
        let package = reveille_core::platform::reborn::package(
            reveille_core::platform::reborn::RebornProductSet::Aa,
        );
        let files = [RebornExecutable {
            filename: "MOHAA.exe".to_owned(),
            bytes: b"reborn".to_vec(),
            sha256: format!("{:x}", Sha256::digest(b"reborn")),
        }];

        install_reborn(root, &package, &files, EngineActivity::ConfirmedStopped)
            .expect("first install");
        assert_eq!(fs::read(root.join("MOHAA.exe")).expect("active"), b"reborn");
        assert_eq!(
            fs::read(root.join(MANAGED_DIRECTORY).join("original/MOHAA.exe"))
                .expect("preserved original"),
            b"original"
        );

        fs::write(root.join("MOHAA.exe"), b"external change").expect("external change");
        install_reborn(root, &package, &files, EngineActivity::ConfirmedStopped)
            .expect("reinstall");
        assert_eq!(
            fs::read(root.join(MANAGED_DIRECTORY).join("original/MOHAA.exe"))
                .expect("unchanged original"),
            b"original"
        );
        activate(
            root,
            EngineChoice::Original,
            EngineActivity::ConfirmedStopped,
        )
        .expect("activate original");
        assert_eq!(
            fs::read(root.join("MOHAA.exe")).expect("original active"),
            b"original"
        );
        activate(root, EngineChoice::Reborn, EngineActivity::ConfirmedStopped)
            .expect("activate Reborn");
        assert_eq!(
            fs::read(root.join("MOHAA.exe")).expect("Reborn active"),
            b"reborn"
        );
        assert!(state_path(root).is_file());

        let mut historical = read_state(root).expect("read state").expect("state");
        historical
            .reborn
            .as_mut()
            .expect("Reborn receipt")
            .source_commit = "older".to_owned();
        write_state(root, &historical).expect("historical receipt");
        assert!(matches!(
            inventory(root).reborn_build,
            RebornInstalledBuild::KnownOther { .. }
        ));
        fs::write(
            root.join(MANAGED_DIRECTORY).join("reborn/MOHAA.exe"),
            b"externally changed",
        )
        .expect("change managed Reborn");
        assert_eq!(inventory(root).reborn_build, RebornInstalledBuild::Unknown);
    }

    #[test]
    fn running_or_unknown_process_state_changes_no_canonical_files() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path();
        fs::write(root.join("MOHAA.exe"), b"original").expect("original client");
        let package = reveille_core::platform::reborn::package(
            reveille_core::platform::reborn::RebornProductSet::Aa,
        );
        let files = [RebornExecutable {
            filename: "MOHAA.exe".to_owned(),
            bytes: b"reborn".to_vec(),
            sha256: "fixture".to_owned(),
        }];
        for activity in [
            EngineActivity::Running(vec!["MOHAA.exe".to_owned()]),
            EngineActivity::Unknown,
        ] {
            assert!(install_reborn(root, &package, &files, activity).is_err());
            assert_eq!(
                fs::read(root.join("MOHAA.exe")).expect("unchanged"),
                b"original"
            );
        }
    }

    #[test]
    #[ignore = "requires the four official pinned archives in REVEILLE_REBORN_FIXTURE_DIR"]
    fn installs_every_pinned_package_only_into_scratch_roots() {
        use reveille_core::platform::reborn::{RebornProductSet, inspect_package, package};

        let fixtures = PathBuf::from(
            std::env::var_os("REVEILLE_REBORN_FIXTURE_DIR").expect("fixture directory"),
        );
        for product_set in [
            RebornProductSet::Aa,
            RebornProductSet::AaSh,
            RebornProductSet::AaBt,
            RebornProductSet::AaShBt,
        ] {
            let package = package(product_set);
            let bytes = fs::read(fixtures.join(&package.filename)).expect("pinned archive");
            let executables = inspect_package(&package, &bytes).expect("verified archive");
            let scratch = TempDir::new().expect("scratch installation");
            for executable in &executables {
                fs::write(
                    scratch.path().join(&executable.filename),
                    format!("original {}", executable.filename),
                )
                .expect("dummy original");
            }
            install_reborn(
                scratch.path(),
                &package,
                &executables,
                EngineActivity::ConfirmedStopped,
            )
            .expect("install pinned package");
            for executable in &executables {
                assert_eq!(
                    hash_file(&scratch.path().join(&executable.filename))
                        .expect("active Reborn hash"),
                    executable.sha256
                );
                assert!(
                    scratch
                        .path()
                        .join(MANAGED_DIRECTORY)
                        .join("original")
                        .join(&executable.filename)
                        .is_file()
                );
            }
        }
    }
}
