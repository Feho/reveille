// SPDX-License-Identifier: GPL-2.0-only

//! GOG Galaxy `goggame-<product-id>.info` mini-manifest parsing.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// GOG product identifier, kept as text because Galaxy's schema represents it as a string.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct GogProductId(String);

impl GogProductId {
    /// Validate a numeric GOG product identifier.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or non-numeric identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, GogError> {
        let value = value.into();
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(GogError::InvalidProductId(value));
        }
        Ok(Self(value))
    }

    /// Return the manifest representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GogProductId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Evidence extracted from one Galaxy mini-manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GogInstallation {
    /// Product described by the file.
    pub product_id: GogProductId,
    /// Root product, which differs for DLC manifests.
    pub root_product_id: GogProductId,
    /// Display name from the manifest.
    pub name: String,
    /// Manifest file used as evidence.
    pub manifest_path: PathBuf,
    /// Installation root: Galaxy places the mini-manifest inside the game directory.
    pub installation_root: PathBuf,
    /// Safe primary `FileTask`, resolved relative to the installation root.
    pub primary_executable: Option<PathBuf>,
}

/// Parse a Galaxy mini-manifest. Its parent directory is the installation root.
///
/// # Errors
///
/// Returns an error for unreadable JSON, invalid product identifiers, a filename/product mismatch,
/// or an unsafe primary executable path.
pub fn parse_gog_manifest(
    manifest_path: impl AsRef<Path>,
    text: &str,
) -> Result<GogInstallation, GogError> {
    let manifest_path = manifest_path.as_ref();
    let installation_root = manifest_path
        .parent()
        .ok_or_else(|| GogError::MissingInstallationRoot(manifest_path.to_path_buf()))?;
    let raw: RawManifest = serde_json::from_str(text).map_err(GogError::MalformedJson)?;
    let product_id = GogProductId::new(raw.game_id)?;
    let root_product_id = GogProductId::new(raw.root_game_id)?;
    if let Some(file_product_id) = product_id_from_filename(manifest_path)
        && file_product_id != product_id.as_str()
    {
        return Err(GogError::FilenameProductMismatch {
            filename: file_product_id.to_owned(),
            manifest: product_id,
        });
    }
    let primary_executable = raw
        .play_tasks
        .iter()
        .find(|task| task.is_primary && task.kind == "FileTask")
        .map(|task| safe_task_path(installation_root, &task.path))
        .transpose()?;
    Ok(GogInstallation {
        product_id,
        root_product_id,
        name: raw.name,
        manifest_path: manifest_path.to_path_buf(),
        installation_root: installation_root.to_path_buf(),
        primary_executable,
    })
}

/// Read and parse one Galaxy mini-manifest.
///
/// # Errors
///
/// Returns an error when the file cannot be read or its contents are invalid.
pub fn read_gog_manifest(path: impl AsRef<Path>) -> Result<GogInstallation, GogError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| GogError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_gog_manifest(path, &text)
}

fn product_id_from_filename(path: &Path) -> Option<&str> {
    let filename = path.file_name()?.to_str()?;
    filename.strip_prefix("goggame-")?.strip_suffix(".info")
}

fn safe_task_path(root: &Path, value: &str) -> Result<PathBuf, GogError> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return Err(GogError::UnsafeTaskPath(value.to_owned()));
    }
    let relative = Path::new(&normalized);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(GogError::UnsafeTaskPath(value.to_owned()));
    }
    Ok(root.join(relative))
}

#[derive(Deserialize)]
struct RawManifest {
    #[serde(rename = "gameId")]
    game_id: String,
    #[serde(rename = "rootGameId")]
    root_game_id: String,
    name: String,
    #[serde(rename = "playTasks", default)]
    play_tasks: Vec<RawPlayTask>,
}

#[derive(Deserialize)]
struct RawPlayTask {
    #[serde(rename = "isPrimary", default)]
    is_primary: bool,
    #[serde(rename = "type")]
    kind: String,
    path: String,
}

/// GOG mini-manifest error.
#[derive(Debug, Error)]
pub enum GogError {
    /// Manifest file could not be read.
    #[error("could not read GOG manifest {path}")]
    Read {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Manifest JSON did not match the Galaxy mini-manifest shape.
    #[error("malformed GOG Galaxy manifest")]
    MalformedJson(#[source] serde_json::Error),
    /// Product identifiers must be non-empty decimal strings.
    #[error("invalid GOG product id {0:?}")]
    InvalidProductId(String),
    /// A `goggame-<id>.info` filename contradicted its body.
    #[error("GOG manifest filename id {filename} differs from body id {manifest}")]
    FilenameProductMismatch {
        /// Product identifier in the filename.
        filename: String,
        /// Product identifier in the JSON body.
        manifest: GogProductId,
    },
    /// A path without a parent cannot identify its installation root.
    #[error("GOG manifest has no parent installation directory: {0}")]
    MissingInstallationRoot(PathBuf),
    /// A primary task path was absolute or traversed out of the installation root.
    #[error("unsafe GOG primary task path {0:?}")]
    UnsafeTaskPath(String),
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{GogError, parse_gog_manifest, read_gog_manifest};

    #[test]
    fn parses_primary_file_task_relative_to_manifest_directory() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path().join("Medal of Honor Allied Assault");
        fs::create_dir(&root).expect("installation root");
        let path = root.join("goggame-1234567890.info");
        fs::write(
            &path,
            include_str!("../../tests/fixtures/goggame-1234567890.info"),
        )
        .expect("manifest");

        let installation = read_gog_manifest(&path).expect("GOG installation");
        assert_eq!(installation.product_id.as_str(), "1234567890");
        assert_eq!(installation.installation_root, root);
        assert_eq!(
            installation.primary_executable,
            Some(root.join("game").join("mohaa.exe"))
        );
    }

    #[test]
    fn rejects_filename_mismatch_and_traversing_primary_task() {
        let fixture = include_str!("../../tests/fixtures/goggame-1234567890.info");
        assert!(matches!(
            parse_gog_manifest("/games/goggame-999.info", fixture),
            Err(GogError::FilenameProductMismatch { .. })
        ));

        let hostile = fixture.replace("game\\\\mohaa.exe", "..\\\\outside.exe");
        assert!(matches!(
            parse_gog_manifest("/games/goggame-1234567890.info", &hostile),
            Err(GogError::UnsafeTaskPath(_))
        ));
    }
}
