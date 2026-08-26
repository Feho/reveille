// SPDX-License-Identifier: GPL-2.0-only

//! Build a map index that mirrors MOHAA's content search precedence.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::bsp::{self, Checksum, Header, Ident};

/// A map name normalized exactly like engine and catalogue lookups.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MapKey(String);

impl MapKey {
    /// Normalize a BSP path, server map name, or catalogue map name.
    ///
    /// This trims outer whitespace, changes backslashes to slashes, folds ASCII case, then
    /// strips an optional `maps/` prefix and `.bsp` suffix. It intentionally does nothing else.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        let normalized = name.trim().replace('\\', "/").to_ascii_lowercase();
        let normalized = normalized.strip_prefix("maps/").unwrap_or(&normalized);
        let normalized = normalized.strip_suffix(".bsp").unwrap_or(normalized);
        (!normalized.is_empty()).then(|| Self(normalized.to_owned()))
    }

    /// Borrow the normalized key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MapKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A source of a BSP file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    /// A BSP entry in a `.pk3` archive.
    Pk3 {
        /// Archive path.
        archive: PathBuf,
        /// Original entry spelling within the archive.
        entry: String,
        /// Informational BSP marker.
        ident: Ident,
        /// Engine-compatible BSP version.
        version: i32,
        /// BSP checksum from the entry header.
        checksum: Checksum,
    },
    /// A BSP inside an unpacked `.pk3dir` tree.
    Pk3Dir {
        /// Root of the `.pk3dir` provider.
        directory: PathBuf,
        /// Original relative entry spelling.
        entry: String,
        /// Informational BSP marker.
        ident: Ident,
        /// Engine-compatible BSP version.
        version: i32,
        /// BSP checksum from the file header.
        checksum: Checksum,
    },
    /// A loose BSP in the game directory. Loose files override every package.
    Loose {
        /// Full file path.
        path: PathBuf,
        /// Original path spelling relative to the game directory.
        entry: String,
        /// Informational BSP marker.
        ident: Ident,
        /// Engine-compatible BSP version.
        version: i32,
        /// BSP checksum from the file header.
        checksum: Checksum,
    },
}

impl Provider {
    /// Return the checksum advertised by this provider's BSP header.
    #[must_use]
    pub const fn checksum(&self) -> Checksum {
        match self {
            Self::Pk3 { checksum, .. }
            | Self::Pk3Dir { checksum, .. }
            | Self::Loose { checksum, .. } => *checksum,
        }
    }

    /// Return the informational four-byte marker classification.
    #[must_use]
    pub const fn ident(&self) -> Ident {
        match self {
            Self::Pk3 { ident, .. } | Self::Pk3Dir { ident, .. } | Self::Loose { ident, .. } => {
                *ident
            }
        }
    }

    /// Return the BSP version accepted by the engine compatibility gate.
    #[must_use]
    pub const fn version(&self) -> i32 {
        match self {
            Self::Pk3 { version, .. }
            | Self::Pk3Dir { version, .. }
            | Self::Loose { version, .. } => *version,
        }
    }
}

/// All providers for one normalized map name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Map {
    /// Case-folded, separator-normalized name without `maps/` or `.bsp`.
    pub name: MapKey,
    /// Original spelling from the provider the engine will load.
    pub display_name: String,
    /// Providers in engine lookup order (highest priority first).
    pub providers: Vec<Provider>,
}

impl Map {
    /// Return the highest-priority provider, which is the file the engine loads.
    #[must_use]
    pub fn effective_provider(&self) -> Option<&Provider> {
        self.providers.first()
    }
}

/// Counts collected while scanning a game directory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ScanStats {
    /// Number of `.pk3` archives scanned.
    pub archives: usize,
    /// Number of `.pk3dir` trees scanned.
    pub package_directories: usize,
    /// Number of loose BSP files scanned below `maps/`.
    pub loose_bsp_files: usize,
    /// Number of distinct normalized maps.
    pub maps: usize,
    /// Number of maps provided by more than one source.
    pub multi_provider_maps: usize,
    /// Number of BSP-looking entries excluded as malformed or engine-incompatible.
    pub skipped_entries: usize,
}

/// A BSP-looking entry excluded from the usable map index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkippedEntry {
    /// Archive or filesystem path containing the entry.
    pub source_path: PathBuf,
    /// Original entry spelling.
    pub entry: String,
    /// Why the entry cannot be loaded by the engine or inspected by Reveille.
    pub reason: SkippedReason,
}

/// Structured reason for excluding one BSP-looking entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SkippedReason {
    /// The twelve-byte header could not be read.
    UnreadableHeader {
        /// Underlying I/O or decompression detail.
        message: String,
    },
    /// The engine rejects this BSP version.
    UnsupportedVersion {
        /// Version read from the BSP header.
        version: i32,
    },
}

impl From<bsp::Error> for SkippedReason {
    fn from(error: bsp::Error) -> Self {
        match error {
            bsp::Error::Read(source) => Self::UnreadableHeader {
                message: source.to_string(),
            },
            bsp::Error::UnsupportedVersion { version } => Self::UnsupportedVersion { version },
        }
    }
}

/// An engine-ordered index of maps in one game directory.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MapIndex {
    maps: BTreeMap<MapKey, Map>,
    skipped: Vec<SkippedEntry>,
    stats: ScanStats,
}

impl MapIndex {
    /// Scan one game directory such as `main`, `mainta`, or `maintt`.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or one of its packages cannot be read. Malformed and
    /// engine-incompatible BSP entries are recorded in [`Self::skipped`] and do not abort a scan.
    pub fn scan(game_directory: impl AsRef<Path>) -> Result<Self, Error> {
        Self::scan_chain(&[game_directory])
    }

    /// Scan every game directory the engine reads, **lowest precedence first**.
    ///
    /// This is the whole search path, not one directory: Spearhead reads `main` and then
    /// `mainta`, and a Breakthrough install reads `main` and then `maintt`
    /// (`LaunchProfile::search_directories`). The engine prepends each search path it adds, so a
    /// map present in both directories is provided by the later one — and both providers are
    /// listed, in that order, because a checksum comparison needs the file the engine will
    /// actually load and the one it shadows.
    ///
    /// # Errors
    ///
    /// Returns an error when a directory or one of its packages cannot be read. Malformed and
    /// engine-incompatible BSP entries are recorded in [`Self::skipped`] and do not abort a scan.
    pub fn scan_chain<P: AsRef<Path>>(directories: &[P]) -> Result<Self, Error> {
        let mut index = Self::default();
        for directory in directories {
            index.scan_game_directory(directory.as_ref())?;
        }
        index.update_stats();
        Ok(index)
    }

    fn scan_game_directory(&mut self, game_directory: &Path) -> Result<(), Error> {
        if !game_directory.is_dir() {
            return Err(Error::NotDirectory(game_directory.to_path_buf()));
        }

        let entries = fs::read_dir(game_directory)
            .map_err(|source| Error::ReadDirectory {
                path: game_directory.to_path_buf(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| Error::ReadDirectory {
                path: game_directory.to_path_buf(),
                source,
            })?;
        let mut packages = entries
            .into_iter()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.to_ascii_lowercase().ends_with(".pk3")
                    || (entry.path().is_dir() && name.to_ascii_lowercase().ends_with(".pk3dir"))
            })
            .collect::<Vec<_>>();

        packages.sort_by(|left, right| {
            left.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .cmp(&right.file_name().to_string_lossy().to_ascii_lowercase())
                .then_with(|| left.file_name().cmp(&right.file_name()))
        });

        for package in packages {
            let path = package.path();
            if path.is_dir() {
                self.scan_pk3dir(&path)?;
                self.stats.package_directories += 1;
            } else {
                self.scan_pk3(&path)?;
                self.stats.archives += 1;
            }
        }

        self.scan_loose(game_directory)?;
        Ok(())
    }

    /// Return a map by a case- and separator-insensitive server name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Map> {
        MapKey::new(name).and_then(|name| self.maps.get(&name))
    }

    /// Iterate maps in normalized lexical order.
    #[must_use]
    pub fn maps(&self) -> impl ExactSizeIterator<Item = &Map> {
        self.maps.values()
    }

    /// Return scan counts.
    #[must_use]
    pub const fn stats(&self) -> ScanStats {
        self.stats
    }

    /// Return malformed or engine-incompatible BSP-looking entries.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    fn scan_pk3(&mut self, path: &Path) -> Result<(), Error> {
        let file = File::open(path).map_err(|source| Error::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let mut archive =
            ZipArchive::new(BufReader::new(file)).map_err(|source| Error::Archive {
                path: path.to_path_buf(),
                source,
            })?;

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|source| Error::Archive {
                path: path.to_path_buf(),
                source,
            })?;
            let original = entry.name().to_owned();
            let Some((name, display_name)) = normalized_bsp_path(&original) else {
                continue;
            };
            let header = match bsp::read_header(&mut entry) {
                Ok(header) => header,
                Err(reason) => {
                    // M1 inventories the user's existing install, so one junk BSP is recorded
                    // and skipped instead of invalidating every otherwise usable map.
                    self.skip(path, original, reason.into());
                    continue;
                }
            };
            self.insert(
                name,
                display_name,
                Provider::Pk3 {
                    archive: path.to_path_buf(),
                    entry: original,
                    ident: header.ident,
                    version: header.version,
                    checksum: header.checksum,
                },
            );
        }
        Ok(())
    }

    fn scan_pk3dir(&mut self, directory: &Path) -> Result<(), Error> {
        let maps = directory.join("maps");
        if maps.is_dir() {
            self.scan_tree(directory, &maps, |_path, entry, header| Provider::Pk3Dir {
                directory: directory.to_path_buf(),
                entry,
                ident: header.ident,
                version: header.version,
                checksum: header.checksum,
            })?;
        }
        Ok(())
    }

    fn scan_loose(&mut self, game_directory: &Path) -> Result<(), Error> {
        let maps = game_directory.join("maps");
        if maps.is_dir() {
            self.stats.loose_bsp_files +=
                self.scan_tree(game_directory, &maps, |path, entry, header| {
                    Provider::Loose {
                        path: path.to_path_buf(),
                        entry,
                        ident: header.ident,
                        version: header.version,
                        checksum: header.checksum,
                    }
                })?;
        }
        Ok(())
    }

    fn scan_tree(
        &mut self,
        root: &Path,
        maps: &Path,
        provider: impl Fn(&Path, String, Header) -> Provider,
    ) -> Result<usize, Error> {
        let mut files = WalkDir::new(maps)
            .follow_links(false)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| Error::Walk {
                path: maps.to_path_buf(),
                source,
            })?;
        files.sort_by_key(|entry| entry.path().to_string_lossy().to_ascii_lowercase());

        let mut candidates = 0;
        for file in files {
            if !file.file_type().is_file()
                || !file
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("bsp"))
            {
                continue;
            }
            candidates += 1;
            let relative =
                file.path()
                    .strip_prefix(root)
                    .map_err(|source| Error::PathOutsideRoot {
                        path: file.path().to_path_buf(),
                        root: root.to_path_buf(),
                        source,
                    })?;
            let original = slash_path(relative);
            let Some((name, display_name)) = normalized_bsp_path(&original) else {
                continue;
            };
            let bsp_file = match File::open(file.path()) {
                Ok(file) => file,
                Err(source) => {
                    self.skip(
                        file.path(),
                        original,
                        SkippedReason::UnreadableHeader {
                            message: source.to_string(),
                        },
                    );
                    continue;
                }
            };
            let header = match bsp::read_header(BufReader::new(bsp_file)) {
                Ok(header) => header,
                Err(reason) => {
                    // M1 inventories the user's existing install, so one junk BSP is recorded
                    // and skipped instead of invalidating every otherwise usable map.
                    self.skip(file.path(), original, reason.into());
                    continue;
                }
            };
            self.insert(name, display_name, provider(file.path(), original, header));
        }
        Ok(candidates)
    }

    fn insert(&mut self, name: MapKey, display_name: String, provider: Provider) {
        let map = self.maps.entry(name.clone()).or_insert_with(|| Map {
            name,
            display_name: display_name.clone(),
            providers: Vec::new(),
        });
        map.display_name = display_name;
        map.providers.push(provider);
    }

    fn skip(&mut self, source_path: &Path, entry: String, reason: SkippedReason) {
        self.skipped.push(SkippedEntry {
            source_path: source_path.to_path_buf(),
            entry,
            reason,
        });
    }

    fn update_stats(&mut self) {
        // Game directories were scanned lowest precedence first, and within each one the
        // packages were added alphabetically and the loose directory last. Because the engine
        // prepends every search path, reverse that addition order into lookup order.
        for map in self.maps.values_mut() {
            map.providers.reverse();
        }
        self.stats.maps = self.maps.len();
        self.stats.multi_provider_maps = self
            .maps
            .values()
            .filter(|map| map.providers.len() > 1)
            .count();
        self.stats.skipped_entries = self.skipped.len();
    }
}

/// An error encountered while indexing a game directory.
#[derive(Debug, Error)]
pub enum Error {
    /// The requested path was not a directory.
    #[error("game path is not a directory: {0}")]
    NotDirectory(PathBuf),
    /// A directory could not be enumerated.
    #[error("could not read directory {path}")]
    ReadDirectory {
        /// Directory being read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// A file could not be opened.
    #[error("could not open {path}")]
    Open {
        /// File being opened.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// A pk3 was not a readable ZIP archive.
    #[error("could not read pk3 archive {path}")]
    Archive {
        /// Archive being read.
        path: PathBuf,
        /// ZIP parser error.
        #[source]
        source: zip::result::ZipError,
    },
    /// A filesystem tree could not be walked.
    #[error("could not walk {path}")]
    Walk {
        /// Tree being walked.
        path: PathBuf,
        /// Walker error.
        #[source]
        source: walkdir::Error,
    },
    /// A walked path unexpectedly fell outside its declared provider root.
    #[error("walked path {path} is outside provider root {root}")]
    PathOutsideRoot {
        /// Path returned by the walker.
        path: PathBuf,
        /// Expected provider root.
        root: PathBuf,
        /// Prefix stripping error.
        #[source]
        source: std::path::StripPrefixError,
    },
}

fn normalized_bsp_path(path: &str) -> Option<(MapKey, String)> {
    let path = path.trim().replace('\\', "/");
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("maps/")
        || !Path::new(&lower)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bsp"))
    {
        return None;
    }
    let display = &path[5..path.len() - 4];
    MapKey::new(&path).map(|key| (key, display.to_owned()))
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;

    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{MapIndex, MapKey, Provider, SkippedReason};
    use crate::bsp::{Checksum, Ident};

    fn bsp(checksum: i32) -> [u8; 12] {
        bsp_header(*b"2015", 19, checksum)
    }

    fn bsp_header(ident: [u8; 4], version: i32, checksum: i32) -> [u8; 12] {
        let mut bytes = [0_u8; 12];
        bytes[0..4].copy_from_slice(&ident);
        bytes[4..8].copy_from_slice(&version.to_le_bytes());
        bytes[8..12].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn write_pk3(path: &Path, entry: &str, checksum: i32) {
        let file = File::create(path).expect("create pk3");
        let mut archive = ZipWriter::new(file);
        archive
            .start_file(entry, SimpleFileOptions::default())
            .expect("start entry");
        archive.write_all(&bsp(checksum)).expect("write entry");
        archive.finish().expect("finish pk3");
    }

    #[test]
    fn mirrors_package_and_loose_file_precedence() {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir_all(main.join("B.pk3dir/maps/DM")).expect("pk3dir maps");
        fs::create_dir_all(main.join("maps/dm")).expect("loose maps");

        write_pk3(&main.join("a.pk3"), "maps/DM/Test.bsp", 1);
        fs::write(main.join("B.pk3dir/maps/DM/test.BSP"), bsp(2)).expect("pk3dir BSP");
        write_pk3(&main.join("c.PK3"), "maps\\dm\\TEST.bsp", 3);
        fs::write(main.join("maps/dm/teST.bsp"), bsp(4)).expect("loose BSP");

        let index = MapIndex::scan(&main).expect("scan index");
        let map = index.get("DM\\TEST").expect("case-folded map");

        assert_eq!(map.name, MapKey::new("dm/test").expect("map key"));
        assert_eq!(map.display_name, "dm/teST");
        assert_eq!(map.providers.len(), 4);
        assert!(matches!(
            map.providers[0],
            Provider::Loose { checksum, .. } if checksum == crate::bsp::Checksum::new(4)
        ));
        assert!(matches!(
            map.providers[1],
            Provider::Pk3 { checksum, .. } if checksum == crate::bsp::Checksum::new(3)
        ));
        assert!(matches!(
            map.providers[2],
            Provider::Pk3Dir { checksum, .. } if checksum == crate::bsp::Checksum::new(2)
        ));
        assert!(matches!(
            map.providers[3],
            Provider::Pk3 { checksum, .. } if checksum == crate::bsp::Checksum::new(1)
        ));
        assert_eq!(
            map.effective_provider().map(Provider::checksum),
            Some(crate::bsp::Checksum::new(4))
        );
        assert_eq!(index.stats().archives, 2);
        assert_eq!(index.stats().package_directories, 1);
        assert_eq!(index.stats().maps, 1);
        assert_eq!(index.stats().multi_provider_maps, 1);
    }

    #[test]
    fn an_expansion_directory_shadows_main_without_hiding_it() {
        // `main` is added first and `mainta` after it, so a map in both is loaded from `mainta`
        // and the copy in `main` is still listed underneath it (join.rs `search_directories`).
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        let mainta = temporary.path().join("mainta");
        fs::create_dir_all(&main).expect("main directory");
        fs::create_dir_all(&mainta).expect("mainta directory");
        write_pk3(&main.join("pak0.pk3"), "maps/dm/mohdm6.bsp", 11);
        write_pk3(&main.join("pak1.pk3"), "maps/dm/aa_only.bsp", 12);
        write_pk3(&mainta.join("pak1.pk3"), "maps/dm/mohdm6.bsp", 21);

        let index = MapIndex::scan_chain(&[&main, &mainta]).expect("scan the search path");

        let shared = index.get("dm/mohdm6").expect("shared map");
        assert_eq!(shared.providers.len(), 2);
        assert_eq!(
            shared.effective_provider().map(Provider::checksum),
            Some(crate::bsp::Checksum::new(21))
        );
        assert!(matches!(
            &shared.providers[1],
            Provider::Pk3 { archive, .. } if archive.starts_with(&main)
        ));
        // A base-game map stays reachable from the expansion.
        assert!(index.get("dm/aa_only").is_some());
        assert_eq!(index.stats().archives, 3);
        assert_eq!(index.stats().multi_provider_maps, 1);
    }

    #[test]
    fn every_scan_count_accumulates_across_the_chain() {
        // Each per-directory counter is written by the same code for every directory in the
        // chain, so an assignment where an addition was meant would silently report only the
        // last directory's files. Loose files are the one that was actually wrong once.
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        let mainta = temporary.path().join("mainta");
        fs::create_dir_all(main.join("maps/dm")).expect("main loose maps");
        fs::create_dir_all(mainta.join("maps/dm")).expect("mainta loose maps");
        fs::create_dir_all(main.join("base.pk3dir/maps/dm")).expect("main pk3dir");
        fs::create_dir_all(mainta.join("extra.pk3dir/maps/dm")).expect("mainta pk3dir");

        write_pk3(&main.join("pak0.pk3"), "maps/dm/packaged.bsp", 1);
        write_pk3(&mainta.join("pak1.pk3"), "maps/dm/packaged.bsp", 2);
        fs::write(main.join("base.pk3dir/maps/dm/shared.bsp"), bsp(3)).expect("main pk3dir BSP");
        fs::write(mainta.join("extra.pk3dir/maps/dm/shared.bsp"), bsp(4))
            .expect("mainta pk3dir BSP");
        fs::write(main.join("maps/dm/loose_base.bsp"), bsp(5)).expect("main loose BSP");
        fs::write(mainta.join("maps/dm/loose_expansion.bsp"), bsp(6)).expect("mainta loose BSP");
        // Malformed in the *first* directory of the chain: a later pass must not drop it.
        fs::write(main.join("maps/dm/truncated.bsp"), b"2015").expect("truncated loose BSP");

        let index = MapIndex::scan_chain(&[&main, &mainta]).expect("scan the search path");

        let stats = index.stats();
        assert_eq!(stats.archives, 2);
        assert_eq!(stats.package_directories, 2);
        assert_eq!(stats.loose_bsp_files, 3);
        assert_eq!(stats.skipped_entries, 1);
        assert_eq!(stats.multi_provider_maps, 2);
        assert_eq!(index.skipped().len(), 1);

        // A loose file in `mainta` beats every package, in `mainta` or in `main`; a loose file in
        // `main` beats `main`'s packages and loses to everything in `mainta`.
        let expansion_loose = index
            .get("dm/loose_expansion")
            .expect("expansion loose map");
        assert!(matches!(
            expansion_loose.providers.as_slice(),
            [Provider::Loose { .. }]
        ));
        let shared = index.get("dm/shared").expect("map in both pk3dirs");
        assert_eq!(
            shared.effective_provider().map(Provider::checksum),
            Some(crate::bsp::Checksum::new(4))
        );
        let base_loose = index.get("dm/loose_base").expect("base loose map");
        assert!(matches!(
            base_loose.providers.as_slice(),
            [Provider::Loose { .. }]
        ));
    }

    #[test]
    fn accepts_server_names_with_maps_prefix_and_extension() {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir_all(&main).expect("main directory");
        write_pk3(&main.join("maps.pk3"), "maps/dm/example.bsp", 42);

        let index = MapIndex::scan(&main).expect("scan index");
        assert!(index.get("maps/dm/example.bsp").is_some());
    }

    #[test]
    fn indexes_unknown_idents_and_skips_bad_versions_and_truncated_headers() {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir_all(main.join("maps/dm")).expect("loose map directory");

        let file = File::create(main.join("mixed.pk3")).expect("create pk3");
        let mut archive = ZipWriter::new(file);
        for (entry, bytes) in [
            ("maps/dm/unknown_ident.bsp", bsp_header(*b"JUNK", 19, 7)),
            ("maps/dm/too_old.bsp", bsp_header(*b"2015", 16, 8)),
            ("maps/dm/too_new.bsp", bsp_header(*b"2015", 22, 9)),
        ] {
            archive
                .start_file(entry, SimpleFileOptions::default())
                .expect("start entry");
            archive.write_all(&bytes).expect("write entry");
        }
        archive.finish().expect("finish pk3");
        fs::write(main.join("maps/dm/truncated.bsp"), b"2015").expect("truncated loose BSP");

        let index = MapIndex::scan(&main).expect("malformed entries do not abort the scan");
        let provider = index
            .get("dm/unknown_ident")
            .and_then(|map| map.effective_provider())
            .expect("unknown identifier with a valid version is indexed");
        assert_eq!(provider.ident(), Ident::Unknown(*b"JUNK"));
        assert_eq!(provider.version(), 19);
        assert_eq!(provider.checksum(), Checksum::new(7));
        assert!(index.get("dm/too_old").is_none());
        assert!(index.get("dm/too_new").is_none());
        assert!(index.get("dm/truncated").is_none());
        assert_eq!(index.stats().maps, 1);
        assert_eq!(index.stats().loose_bsp_files, 1);
        assert_eq!(index.stats().skipped_entries, 3);
        assert_eq!(index.skipped().len(), 3);
        assert!(index.skipped().iter().any(|entry| matches!(
            entry.reason,
            SkippedReason::UnsupportedVersion { version: 16 }
        )));
        assert!(index.skipped().iter().any(|entry| matches!(
            entry.reason,
            SkippedReason::UnsupportedVersion { version: 22 }
        )));
        assert!(
            index
                .skipped()
                .iter()
                .any(|entry| matches!(entry.reason, SkippedReason::UnreadableHeader { .. }))
        );
    }
}
