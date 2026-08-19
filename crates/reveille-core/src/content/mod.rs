// SPDX-License-Identifier: GPL-2.0-only

//! Content-source resolution and safe package installation.

mod archive;
mod mohdb;
mod pakradar;

use serde::Serialize;

use crate::mapindex::MapKey;

pub use archive::{
    ArchiveError, ArchiveInspection, ArchiveMap, ConfirmedMap, DownloadedArchive, MohDbIntegrity,
    PakRadarIntegrity, confirm_map, disambiguate_by_checksum, inspect_archive, install_archive,
};
pub use mohdb::{
    CatalogueCandidate, CatalogueNonResult, CatalogueNonResultReason, CataloguePage,
    CatalogueResolution, CatalogueResolutionPass, FileSize, MohDbClient, MohDbError,
    ResolutionOutcome, download_mohdb_archive, resolve_candidates,
};
pub use pakradar::{
    Md5Digest, PakRadarEntry, PakRadarError, download_pakradar_archive, fetch_filelist,
    parse_filelist,
};

/// A server map name paired with the exact normalised key used everywhere in the pipeline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WantedMap {
    /// Original server spelling for display.
    pub name: String,
    /// Five-step normalised lookup key.
    pub key: MapKey,
}

impl WantedMap {
    /// Construct a wanted map from a server rotation entry.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        MapKey::new(&name).map(|key| Self { name, key })
    }
}
