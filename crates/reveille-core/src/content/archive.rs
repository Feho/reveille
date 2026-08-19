// SPDX-License-Identifier: GPL-2.0-only

//! Hostile-package inspection, BSP confirmation, and no-clobber installation.

use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use zip::ZipArchive;

use crate::bsp::{self, Checksum};
use crate::mapindex::MapKey;

/// Integrity evidence available for a moh-db archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "integrity", rename_all = "snake_case")]
pub enum MohDbIntegrity {
    /// SHA-256 recorded on first download for later trust-on-first-use comparisons.
    RecordedSha256(String),
}

/// Integrity evidence available for a `PakRadar` archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "integrity", rename_all = "snake_case")]
pub enum PakRadarIntegrity {
    /// MD5 matched the digest published by the server's `PakRadar` manifest.
    VerifiedMd5(String),
}

/// An archive downloaded into a staging directory with its source-specific integrity evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadedArchive<Integrity> {
    /// Staged archive path.
    pub path: PathBuf,
    /// Source filename preserved verbatim after safety validation.
    pub filename: String,
    /// Evidence that differs deliberately between moh-db and `PakRadar`.
    pub integrity: Integrity,
}

/// One engine-loadable BSP found inside a safe archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchiveMap {
    /// Original ZIP entry spelling.
    pub entry: String,
    /// Shared five-step normalised map name.
    pub key: MapKey,
    /// BSP checksum from the engine header.
    pub checksum: Checksum,
}

/// Safety and content inspection performed after download.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchiveInspection {
    /// Every loadable BSP path in the archive.
    pub maps: Vec<ArchiveMap>,
}

/// Post-download confirmation that the archive really carries the wanted BSP.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfirmedMap {
    /// Original server spelling.
    pub wanted: String,
    /// Matching archive BSP.
    pub archive_map: ArchiveMap,
    /// Whether a server-published BSP checksum was also matched.
    pub checksum_confirmed: bool,
}

/// Inspect every archive path before an archive is eligible for installation.
///
/// # Errors
///
/// Rejects unreadable ZIPs, absolute or parent-traversing paths, executable-library entries,
/// and malformed or engine-incompatible BSP entries.
pub fn inspect_archive(path: impl AsRef<Path>) -> Result<ArchiveInspection, ArchiveError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| ArchiveError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive =
        ZipArchive::new(BufReader::new(file)).map_err(|source| ArchiveError::InvalidZip {
            path: path.to_path_buf(),
            source,
        })?;
    let mut maps = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| ArchiveError::InvalidZip {
                path: path.to_path_buf(),
                source,
            })?;
        let name = entry.name().to_owned();
        validate_entry_path(&name)?;
        if entry.is_dir() {
            continue;
        }
        let normalized_path = name.replace('\\', "/");
        let lower = normalized_path.to_ascii_lowercase();
        let extension = Path::new(&normalized_path).extension();
        if extension.is_some_and(|extension| {
            extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("dll")
        }) {
            return Err(ArchiveError::ForbiddenEntry { entry: name });
        }
        if !lower.starts_with("maps/")
            || !extension.is_some_and(|extension| extension.eq_ignore_ascii_case("bsp"))
        {
            continue;
        }
        let key = MapKey::new(&normalized_path).ok_or_else(|| ArchiveError::InvalidEntryPath {
            entry: name.clone(),
        })?;
        // Unlike M1's tolerant scan of an existing install, a stranger's archive must be
        // wholly engine-loadable before Reveille is willing to install any of it.
        let header = bsp::read_header(&mut entry).map_err(|source| ArchiveError::Bsp {
            entry: name.clone(),
            source,
        })?;
        maps.push(ArchiveMap {
            entry: name,
            key,
            checksum: header.checksum,
        });
    }
    Ok(ArchiveInspection { maps })
}

/// Confirm a provisional name match from the archive's actual BSP paths and optional checksum.
///
/// # Errors
///
/// Returns an error when the wanted map path is absent or no matching path has the server's
/// published checksum.
pub fn confirm_map(
    inspection: &ArchiveInspection,
    wanted: &str,
    published_checksum: Option<Checksum>,
) -> Result<ConfirmedMap, ArchiveError> {
    let wanted_key = MapKey::new(wanted).ok_or_else(|| ArchiveError::InvalidWantedMap {
        map: wanted.to_owned(),
    })?;
    let matching = inspection
        .maps
        .iter()
        .filter(|map| map.key == wanted_key)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Err(ArchiveError::WantedMapMissing {
            map: wanted.to_owned(),
        });
    }
    let selected = match published_checksum {
        Some(checksum) => matching
            .iter()
            .find(|map| map.checksum == checksum)
            .copied()
            .ok_or_else(|| ArchiveError::ChecksumMismatch {
                map: wanted.to_owned(),
                expected: checksum,
                found: matching.iter().map(|map| map.checksum).collect(),
            })?,
        None => matching
            .first()
            .copied()
            .ok_or_else(|| ArchiveError::WantedMapMissing {
                map: wanted.to_owned(),
            })?,
    };
    Ok(ConfirmedMap {
        wanted: wanted.to_owned(),
        archive_map: selected.clone(),
        checksum_confirmed: published_checksum.is_some(),
    })
}

/// Select the downloaded candidate whose actual BSP path and checksum match the server.
///
/// The returned index maps back to the caller's ranked catalogue candidates. Ranking breaks ties,
/// but can never override a checksum disagreement.
///
/// # Errors
///
/// Returns an error when none of the inspected candidate archives contains the wanted map with
/// the server-published checksum.
pub fn disambiguate_by_checksum(
    inspections: &[ArchiveInspection],
    wanted: &str,
    published_checksum: Checksum,
) -> Result<(usize, ConfirmedMap), ArchiveError> {
    for (index, inspection) in inspections.iter().enumerate() {
        if let Ok(confirmed) = confirm_map(inspection, wanted, Some(published_checksum)) {
            return Ok((index, confirmed));
        }
    }
    Err(ArchiveError::NoArchiveMatchedChecksum {
        map: wanted.to_owned(),
        expected: published_checksum,
    })
}

/// Install a staged archive without changing its filename or overwriting an existing package.
///
/// The archive is inspected again immediately before copying, so callers cannot accidentally
/// bypass the hostile-entry checks.
///
/// # Errors
///
/// Returns an error for unsafe archives, invalid filenames, destination I/O, or collisions.
pub fn install_archive<Integrity>(
    archive: &DownloadedArchive<Integrity>,
    game_directory: impl AsRef<Path>,
) -> Result<PathBuf, ArchiveError> {
    inspect_archive(&archive.path)?;
    validate_package_filename(&archive.filename)?;
    let game_directory = game_directory.as_ref();
    fs::create_dir_all(game_directory).map_err(|source| ArchiveError::Install {
        path: game_directory.to_path_buf(),
        source,
    })?;
    let target = game_directory.join(&archive.filename);
    if target.exists() {
        return Err(ArchiveError::AlreadyExists(target));
    }
    let mut source = File::open(&archive.path).map_err(|source| ArchiveError::Open {
        path: archive.path.clone(),
        source,
    })?;
    let mut temporary =
        NamedTempFile::new_in(game_directory).map_err(|source| ArchiveError::Install {
            path: game_directory.to_path_buf(),
            source,
        })?;
    io::copy(&mut source, &mut temporary).map_err(|source| ArchiveError::Install {
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary.flush().map_err(|source| ArchiveError::Install {
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary
        .persist_noclobber(&target)
        .map_err(|error| ArchiveError::Install {
            path: target.clone(),
            source: error.error,
        })?;
    Ok(target)
}

pub(super) fn validate_package_filename(filename: &str) -> Result<(), ArchiveError> {
    let device_basename = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved_device = matches!(device_basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || device_basename
            .strip_prefix("COM")
            .or_else(|| device_basename.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains(':')
        || filename == "."
        || filename == ".."
        || filename.ends_with(['.', ' '])
        || reserved_device
        || !filename.to_ascii_lowercase().ends_with(".pk3")
    {
        return Err(ArchiveError::UnsafeFilename(filename.to_owned()));
    }
    Ok(())
}

fn validate_entry_path(entry: &str) -> Result<(), ArchiveError> {
    let path = entry.replace('\\', "/");
    let bytes = path.as_bytes();
    let windows_absolute = bytes.len() >= 2 && bytes[1] == b':';
    if path.starts_with('/')
        || windows_absolute
        || path.split('/').any(|component| component == "..")
    {
        return Err(ArchiveError::InvalidEntryPath {
            entry: entry.to_owned(),
        });
    }
    Ok(())
}

/// Package inspection or installation error.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// The outer package filename is not a single `.pk3` basename.
    #[error("unsafe package filename {0:?}")]
    UnsafeFilename(String),
    /// The ZIP could not be opened.
    #[error("could not open archive {path}")]
    Open {
        /// Archive path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The file is not a readable ZIP archive.
    #[error("invalid ZIP archive {path}")]
    InvalidZip {
        /// Archive path.
        path: PathBuf,
        /// ZIP error.
        #[source]
        source: zip::result::ZipError,
    },
    /// A ZIP path is absolute, traverses to a parent, or is otherwise unusable.
    #[error("unsafe archive entry path {entry:?}")]
    InvalidEntryPath {
        /// Hostile entry spelling.
        entry: String,
    },
    /// Executables and dynamic libraries are forbidden in map packages.
    #[error("archive contains forbidden executable entry {entry:?}")]
    ForbiddenEntry {
        /// Rejected entry spelling.
        entry: String,
    },
    /// A BSP entry is malformed or rejected by the engine's version gate.
    #[error("archive BSP {entry:?} is not engine-loadable")]
    Bsp {
        /// BSP entry spelling.
        entry: String,
        /// Header error.
        #[source]
        source: bsp::Error,
    },
    /// The requested server map name cannot be normalised.
    #[error("invalid wanted map name {map:?}")]
    InvalidWantedMap {
        /// Server spelling.
        map: String,
    },
    /// The provisional catalogue name match was not present in the archive.
    #[error("archive does not contain wanted map {map:?}")]
    WantedMapMissing {
        /// Server spelling.
        map: String,
    },
    /// Matching BSP paths exist but none carry the server-published checksum.
    #[error("archive map {map:?} does not match published checksum {expected}")]
    ChecksumMismatch {
        /// Server spelling.
        map: String,
        /// Server-published checksum.
        expected: Checksum,
        /// Checksums found on matching BSP paths.
        found: Vec<Checksum>,
    },
    /// No downloaded candidate matched both the wanted BSP path and server checksum.
    #[error("no candidate archive contains {map:?} with published checksum {expected}")]
    NoArchiveMatchedChecksum {
        /// Server spelling.
        map: String,
        /// Server-published checksum.
        expected: Checksum,
    },
    /// Installation never overwrites an existing package.
    #[error("package already exists at {0}")]
    AlreadyExists(PathBuf),
    /// A staging-to-game-directory operation failed.
    #[error("could not install archive at {path}")]
    Install {
        /// Affected destination path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{
        ArchiveError, ArchiveInspection, ArchiveMap, DownloadedArchive, MohDbIntegrity,
        confirm_map, disambiguate_by_checksum, inspect_archive, install_archive,
        validate_package_filename,
    };
    use crate::bsp::Checksum;

    fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).expect("create archive");
        let mut archive = ZipWriter::new(file);
        for (name, bytes) in entries {
            archive
                .start_file(*name, SimpleFileOptions::default())
                .expect("start archive entry");
            archive.write_all(bytes).expect("write archive entry");
        }
        archive.finish().expect("finish archive");
    }

    fn bsp(checksum: i32) -> [u8; 12] {
        let mut bytes = *b"2015\x13\0\0\0\0\0\0\0";
        bytes[8..12].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn confirms_the_actual_archive_path_and_checksum_after_download() {
        let temporary = TempDir::new().expect("temporary directory");
        let path = temporary.path().join("map.pk3");
        write_archive(&path, &[("MAPS\\OBJ\\Example.BSP", &bsp(42))]);

        let inspection = inspect_archive(&path).expect("safe archive");
        let confirmed = confirm_map(&inspection, " obj/example ", Some(Checksum::new(42)))
            .expect("map and checksum match");

        assert_eq!(confirmed.archive_map.key.as_str(), "obj/example");
        assert!(confirmed.checksum_confirmed);
    }

    #[test]
    fn rejects_parent_traversal_in_a_hostile_archive() {
        let temporary = TempDir::new().expect("temporary directory");
        let path = temporary.path().join("hostile.pk3");
        write_archive(&path, &[("../../outside.cfg", b"hostile")]);

        assert!(matches!(
            inspect_archive(&path),
            Err(ArchiveError::InvalidEntryPath { .. })
        ));
    }

    #[test]
    fn rejects_windows_traversal_and_executable_entries() {
        let temporary = TempDir::new().expect("temporary directory");
        let traversal = temporary.path().join("traversal.pk3");
        write_archive(&traversal, &[("..\\outside.cfg", b"hostile")]);
        assert!(matches!(
            inspect_archive(&traversal),
            Err(ArchiveError::InvalidEntryPath { .. })
        ));

        let executable = temporary.path().join("executable.pk3");
        write_archive(&executable, &[("tools/installer.EXE", b"hostile")]);
        assert!(matches!(
            inspect_archive(&executable),
            Err(ArchiveError::ForbiddenEntry { .. })
        ));

        let library = temporary.path().join("library.pk3");
        write_archive(&library, &[("bin/helper.DLL", b"hostile")]);
        assert!(matches!(
            inspect_archive(&library),
            Err(ArchiveError::ForbiddenEntry { .. })
        ));
    }

    #[test]
    fn rejects_windows_device_names_and_trailing_dots_or_spaces() {
        for filename in [
            "CON.pk3",
            "prn.extra.pk3",
            "AUX.PK3",
            "nul.pk3",
            "COM1.pk3",
            "com9.extra.pk3",
            "LPT1.pk3",
            "lpt9.extra.pk3",
            "evil.pk3.",
            "evil.pk3 ",
        ] {
            assert!(matches!(
                validate_package_filename(filename),
                Err(ArchiveError::UnsafeFilename(_))
            ));
        }
        for filename in ["COM0.pk3", "COM10.pk3", "LPT0.pk3", "LPT10.pk3", "safe.pk3"] {
            validate_package_filename(filename).expect("safe package filename");
        }
    }

    #[test]
    fn installs_with_the_source_filename_and_never_overwrites() {
        let temporary = TempDir::new().expect("temporary directory");
        let staged = temporary.path().join("staging");
        let main = temporary.path().join("main");
        std::fs::create_dir_all(&staged).expect("staging directory");
        let path = staged.join("Original_Name.pk3");
        write_archive(&path, &[("maps/obj/example.bsp", &bsp(42))]);
        let download = DownloadedArchive {
            path,
            filename: "Original_Name.pk3".to_owned(),
            integrity: MohDbIntegrity::RecordedSha256("recorded".to_owned()),
        };

        let installed = install_archive(&download, &main).expect("safe install");
        assert_eq!(installed, main.join("Original_Name.pk3"));
        assert!(matches!(
            install_archive(&download, &main),
            Err(ArchiveError::AlreadyExists(_))
        ));
    }

    #[test]
    fn server_checksum_disambiguates_ranked_downloaded_candidates() {
        let key = crate::mapindex::MapKey::new("obj/example").expect("valid map key");
        let inspections = [
            ArchiveInspection {
                maps: vec![ArchiveMap {
                    entry: "maps/obj/example.bsp".to_owned(),
                    key: key.clone(),
                    checksum: Checksum::new(41),
                }],
            },
            ArchiveInspection {
                maps: vec![ArchiveMap {
                    entry: "maps/obj/example.bsp".to_owned(),
                    key,
                    checksum: Checksum::new(42),
                }],
            },
        ];

        let (index, confirmed) =
            disambiguate_by_checksum(&inspections, "obj/example", Checksum::new(42))
                .expect("second candidate matches server checksum");

        assert_eq!(index, 1);
        assert!(confirmed.checksum_confirmed);
    }
}
