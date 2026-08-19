// SPDX-License-Identifier: GPL-2.0-only

//! moh-db catalogue access. Name matches are provisional until archive inspection.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::fs;

use super::WantedMap;
use super::archive::{DownloadedArchive, MohDbIntegrity, validate_package_filename};
use crate::mapindex::MapKey;

// Public endpoint documented by moh-db; gameType is intentionally not sent because it is ignored.
// The current public MapDto also omits gameType, so no reliable pre-download local filter exists.
const MAPS_ENDPOINT: &str = "https://api.moh-db.com/api/external/v1/maps";
const PAGE_SIZE: usize = 100;
// moh-db rejects generic library user agents with HTTP 403.
const USER_AGENT: &str = "Reveille/0.1 (MOHAA content resolver)";

/// Archive size in bytes as published by a catalogue source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FileSize(u64);

impl FileSize {
    /// Construct a byte count.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the underlying byte count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One downloadable moh-db record, still only a name-level candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogueCandidate {
    /// moh-db node identifier.
    pub id: u64,
    /// Original catalogue spelling, including meaningful evidence such as stray whitespace.
    pub map_name: String,
    /// The shared five-step normalisation of `map_name`.
    pub map_key: MapKey,
    /// Filename that must be preserved through staging and installation.
    pub filename: String,
    /// Published byte size. This is metadata, not an integrity digest.
    pub file_size: FileSize,
    /// Whether moh-db marks the file as tested.
    pub map_file_tested: bool,
    /// Catalogue download count used only as a secondary ranking signal.
    pub downloads: u64,
    /// Direct archive URL returned by moh-db.
    pub download_url: String,
}

/// One Spring-style catalogue page after unusable records are excluded.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CataloguePage {
    /// Downloadable records in this page.
    pub content: Vec<CatalogueCandidate>,
    /// Upstream result count across every page.
    pub total_elements: usize,
}

/// A name-only outcome. Only `exact` may be selected automatically, and even it remains
/// unconfirmed until the downloaded archive contains the requested BSP path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ResolutionOutcome {
    /// One exact normalised name match selected by tested/download ranking.
    Exact {
        /// Highest-ranked exact name match; this is not yet archive-confirmed.
        name_match: CatalogueCandidate,
        /// Other exact normalised records, in the same deterministic ranking order.
        alternatives: Vec<CatalogueCandidate>,
    },
    /// Non-exact candidates that require an explicit user decision.
    ChoiceRequired {
        /// Presented choices in tested/download ranking order. None is auto-applied.
        choices: Vec<CatalogueCandidate>,
    },
    /// No catalogue record was found.
    NoSource,
}

/// Successful catalogue resolution for one wanted map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogueResolution {
    /// Original and normalised server map name.
    pub wanted: WantedMap,
    /// Number of candidates considered before exact-name filtering.
    pub hits: usize,
    /// Exactly one of the three product outcomes.
    #[serde(flatten)]
    pub outcome: ResolutionOutcome,
}

/// Why one map lookup produced no resolution result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CatalogueNonResultReason {
    /// The request deadline elapsed.
    Timeout,
    /// The catalogue returned an HTTP error such as 403.
    HttpStatus {
        /// Numeric response status.
        status: u16,
    },
    /// Transport failure other than timeout.
    Network {
        /// Diagnostic detail.
        message: String,
    },
    /// The JSON response did not match the catalogue schema.
    Malformed {
        /// Diagnostic detail.
        message: String,
    },
}

/// Recorded non-result for one map; it does not abort the surrounding pass.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogueNonResult {
    /// Map whose lookup failed.
    pub wanted: WantedMap,
    /// Structured failure reason.
    pub reason: CatalogueNonResultReason,
}

/// Partial catalogue results with every per-map failure retained.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CatalogueResolutionPass {
    /// Maps whose lookups produced one of the three outcomes.
    pub resolutions: Vec<CatalogueResolution>,
    /// Maps whose individual catalogue requests failed.
    pub non_results: Vec<CatalogueNonResult>,
}

/// Configured moh-db client with an identifying user agent and request deadline.
#[derive(Clone, Debug)]
pub struct MohDbClient {
    client: Client,
}

impl MohDbClient {
    /// Build the production client.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be configured.
    pub fn new(timeout: Duration) -> Result<Self, MohDbError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()
            .map_err(MohDbError::Client)?;
        Ok(Self { client })
    }

    /// Query all pages for one search term.
    ///
    /// # Errors
    ///
    /// Returns a per-map catalogue error. Call [`Self::resolve_all`] to retain these as data.
    pub async fn lookup(&self, map_name: &str) -> Result<CataloguePage, MohDbError> {
        let mut page_number = 0_usize;
        let mut records = Vec::new();
        let mut records_seen = 0_usize;
        let total_elements = loop {
            let response = self
                .client
                .get(MAPS_ENDPOINT)
                .query(&[
                    ("size", PAGE_SIZE.to_string()),
                    ("page", page_number.to_string()),
                    ("mapName", map_name.to_owned()),
                ])
                .send()
                .await
                .map_err(classify_request_error)?;
            let status = response.status();
            if !status.is_success() {
                return Err(MohDbError::Status(status));
            }
            let page = response
                .json::<PageWire>()
                .await
                .map_err(MohDbError::Malformed)?;
            let total = page.total_elements;
            records_seen += page.content.len();
            records.extend(
                page.content
                    .into_iter()
                    .filter_map(CatalogueCandidate::from_wire),
            );
            if records_seen >= total || total == 0 {
                break total;
            }
            page_number += 1;
        };
        Ok(CataloguePage {
            content: records,
            total_elements,
        })
    }

    /// Resolve every wanted map independently, preserving timeouts, 403s, and malformed replies.
    pub async fn resolve_all(&self, wanted: &[WantedMap]) -> CatalogueResolutionPass {
        let mut pass = CatalogueResolutionPass::default();
        for map in wanted {
            match self.resolve_one(map).await {
                Ok(resolution) => pass.resolutions.push(resolution),
                Err(error) => pass.non_results.push(CatalogueNonResult {
                    wanted: map.clone(),
                    reason: error.into(),
                }),
            }
        }
        pass
    }

    async fn resolve_one(&self, wanted: &WantedMap) -> Result<CatalogueResolution, MohDbError> {
        let direct = self.lookup(&wanted.name).await?;
        if !direct.content.is_empty() {
            return Ok(resolve_candidates(wanted.clone(), direct.content));
        }

        let mut choices = Vec::new();
        if let Some(term) = choice_search_term(&wanted.key) {
            let page = self.lookup(&term).await?;
            for candidate in page.content {
                if is_narrow_choice_candidate(&candidate, &term)
                    && !choices
                        .iter()
                        .any(|known: &CatalogueCandidate| known.id == candidate.id)
                {
                    choices.push(candidate);
                }
            }
        }
        Ok(resolve_candidates(wanted.clone(), choices))
    }
}

/// Resolve supplied API candidates without fuzzy auto-application.
#[must_use]
pub fn resolve_candidates(
    wanted: WantedMap,
    mut candidates: Vec<CatalogueCandidate>,
) -> CatalogueResolution {
    candidates.sort_by_key(candidate_rank);
    let hits = candidates.len();
    let mut exact = candidates
        .iter()
        .filter(|candidate| candidate.map_key == wanted.key)
        .cloned()
        .collect::<Vec<_>>();
    exact.sort_by_key(candidate_rank);
    let outcome = if exact.is_empty() {
        if candidates.is_empty() {
            ResolutionOutcome::NoSource
        } else {
            ResolutionOutcome::ChoiceRequired {
                choices: candidates,
            }
        }
    } else {
        let name_match = exact.remove(0);
        ResolutionOutcome::Exact {
            name_match,
            alternatives: exact,
        }
    };
    CatalogueResolution {
        wanted,
        hits,
        outcome,
    }
}

/// Download a moh-db name match into staging and record a SHA-256 digest for TOFU.
///
/// The result carries only a recorded digest because moh-db publishes no integrity digest.
/// Archive-path confirmation is a separate required step.
///
/// # Errors
///
/// Returns an error for unsafe filenames, network failures, size disagreement, or staging I/O.
pub async fn download_mohdb_archive(
    client: &MohDbClient,
    candidate: &CatalogueCandidate,
    staging_directory: &Path,
) -> Result<DownloadedArchive<MohDbIntegrity>, MohDbError> {
    validate_package_filename(&candidate.filename).map_err(MohDbError::Archive)?;
    fs::create_dir_all(staging_directory)
        .await
        .map_err(|source| MohDbError::Staging {
            path: staging_directory.to_path_buf(),
            source,
        })?;
    let response = client
        .client
        .get(&candidate.download_url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(classify_request_error)?;
    if !response.status().is_success() {
        return Err(MohDbError::Status(response.status()));
    }
    let bytes = response.bytes().await.map_err(classify_request_error)?;
    let actual_size = u64::try_from(bytes.len()).map_err(|_| MohDbError::SizeOverflow)?;
    if actual_size != candidate.file_size.get() {
        return Err(MohDbError::SizeMismatch {
            published: candidate.file_size,
            actual: FileSize::new(actual_size),
        });
    }
    let digest = Sha256::digest(&bytes);
    let temporary =
        NamedTempFile::new_in(staging_directory).map_err(|source| MohDbError::Staging {
            path: staging_directory.to_path_buf(),
            source,
        })?;
    fs::write(temporary.path(), &bytes)
        .await
        .map_err(|source| MohDbError::Staging {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    let target = staging_directory.join(&candidate.filename);
    temporary
        .persist_noclobber(&target)
        .map_err(|error| MohDbError::Staging {
            path: target.clone(),
            source: error.error,
        })?;
    Ok(DownloadedArchive {
        path: target,
        filename: candidate.filename.clone(),
        integrity: MohDbIntegrity::RecordedSha256(hex_lower(&digest)),
    })
}

/// Errors from one moh-db request or download.
#[derive(Debug, Error)]
pub enum MohDbError {
    /// HTTP client construction failed.
    #[error("could not configure moh-db client")]
    Client(#[source] reqwest::Error),
    /// One request timed out.
    #[error("moh-db request timed out")]
    Timeout,
    /// One request failed before an HTTP response was received.
    #[error("moh-db network request failed")]
    Network(#[source] reqwest::Error),
    /// moh-db returned an unsuccessful status.
    #[error("moh-db returned HTTP {0}")]
    Status(StatusCode),
    /// The response body did not match the public schema.
    #[error("malformed moh-db response")]
    Malformed(#[source] reqwest::Error),
    /// The archive filename could escape its intended directory.
    #[error(transparent)]
    Archive(#[from] super::archive::ArchiveError),
    /// A byte count did not fit the platform-independent model.
    #[error("download size does not fit in u64")]
    SizeOverflow,
    /// Download length disagreed with catalogue metadata.
    #[error("download size {actual:?} differs from published size {published:?}")]
    SizeMismatch {
        /// Size advertised by moh-db.
        published: FileSize,
        /// Bytes actually received.
        actual: FileSize,
    },
    /// Staging filesystem operation failed.
    #[error("could not write staged archive {path}")]
    Staging {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl From<MohDbError> for CatalogueNonResultReason {
    fn from(error: MohDbError) -> Self {
        match error {
            MohDbError::Timeout => Self::Timeout,
            MohDbError::Status(status) => Self::HttpStatus {
                status: status.as_u16(),
            },
            MohDbError::Malformed(source) => Self::Malformed {
                message: source.to_string(),
            },
            MohDbError::Client(source) | MohDbError::Network(source) => Self::Network {
                message: source.to_string(),
            },
            other => Self::Malformed {
                message: other.to_string(),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct PageWire {
    content: Vec<MapWire>,
    #[serde(rename = "totalElements")]
    total_elements: usize,
}

#[derive(Debug, Deserialize)]
struct MapWire {
    nid: u64,
    #[serde(rename = "mapName")]
    map_name: String,
    #[serde(rename = "mapFileTested")]
    map_file_tested: Option<String>,
    downloads: Option<u64>,
    #[serde(rename = "mapFile")]
    map_file: Option<FileWire>,
    #[serde(rename = "downloadLink")]
    download_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileWire {
    filename: String,
    filesize: u64,
    #[serde(rename = "downloadLink")]
    download_link: Option<String>,
}

impl CatalogueCandidate {
    fn from_wire(record: MapWire) -> Option<Self> {
        let map_key = MapKey::new(&record.map_name)?;
        let file = record.map_file?;
        validate_package_filename(&file.filename).ok()?;
        let download_url = record.download_link.or(file.download_link)?;
        Some(Self {
            id: record.nid,
            map_name: record.map_name,
            map_key,
            filename: file.filename,
            file_size: FileSize::new(file.filesize),
            map_file_tested: record
                .map_file_tested
                .is_some_and(|value| !value.trim().is_empty()),
            downloads: record.downloads.unwrap_or(0),
            download_url,
        })
    }
}

fn candidate_rank(candidate: &CatalogueCandidate) -> (Reverse<bool>, Reverse<u64>, String) {
    (
        Reverse(candidate.map_file_tested),
        Reverse(candidate.downloads),
        candidate.filename.clone(),
    )
}

fn choice_search_term(wanted: &MapKey) -> Option<String> {
    let basename = wanted
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(wanted.as_str());
    let without_prefix = basename.strip_prefix("obj_").unwrap_or(basename);
    let without_suffix = without_prefix
        .strip_suffix("_obj")
        .unwrap_or(without_prefix);
    (!without_suffix.is_empty() && without_suffix != basename).then(|| without_suffix.to_owned())
}

fn is_narrow_choice_candidate(candidate: &CatalogueCandidate, search_term: &str) -> bool {
    candidate
        .map_key
        .as_str()
        .rsplit('/')
        .next()
        .is_some_and(|basename| basename == search_term)
}

fn classify_request_error(error: reqwest::Error) -> MohDbError {
    if error.is_timeout() {
        MohDbError::Timeout
    } else {
        MohDbError::Network(error)
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogueCandidate, FileSize, PageWire, ResolutionOutcome, is_narrow_choice_candidate,
        resolve_candidates,
    };
    use crate::content::WantedMap;
    use crate::mapindex::MapKey;

    fn candidate(id: u64, map: &str, tested: bool, downloads: u64) -> CatalogueCandidate {
        CatalogueCandidate {
            id,
            map_name: map.to_owned(),
            map_key: MapKey::new(map).expect("valid map key"),
            filename: format!("map-{id}.pk3"),
            file_size: FileSize::new(100),
            map_file_tested: tested,
            downloads,
            download_url: format!("https://example.invalid/map-{id}.pk3"),
        }
    }

    #[test]
    fn trims_catalogue_names_and_ranks_tested_then_downloads() {
        let wanted = WantedMap::new("obj/obj_howitzer").expect("valid wanted map");
        let resolution = resolve_candidates(
            wanted,
            vec![
                candidate(1, "obj/obj_howitzer ", false, 10_000),
                candidate(2, "OBJ\\OBJ_HOWITZER", true, 5),
                candidate(3, "obj/obj_howitzer", true, 50),
            ],
        );

        let ResolutionOutcome::Exact {
            name_match,
            alternatives,
        } = resolution.outcome
        else {
            panic!("normalised equality must be exact");
        };
        assert_eq!(name_match.id, 3);
        assert_eq!(
            alternatives.iter().map(|item| item.id).collect::<Vec<_>>(),
            [2, 1]
        );
    }

    #[test]
    fn non_exact_candidates_always_require_a_choice() {
        let wanted = WantedMap::new("obj/obj_rush_party").expect("valid wanted map");
        let resolution = resolve_candidates(wanted, vec![candidate(1, "dm/rush_party", true, 9)]);

        assert!(matches!(
            resolution.outcome,
            ResolutionOutcome::ChoiceRequired { .. }
        ));
    }

    #[test]
    fn narrow_choice_filter_rejects_the_unrelated_morning_result() {
        let quest = candidate(1, "dm/Questufou_s_Yvette", true, 9);
        let rush = candidate(2, "dm/rush_party", true, 9);
        let morning = candidate(3, "dm/dm_morning2", true, 9);

        assert!(is_narrow_choice_candidate(&quest, "questufou_s_yvette"));
        assert!(is_narrow_choice_candidate(&rush, "rush_party"));
        assert!(!is_narrow_choice_candidate(&morning, "morning2"));
    }

    #[test]
    fn parses_the_frozen_spring_page_shape() {
        let page: PageWire = serde_json::from_str(
            r#"{
              "content": [{
                "nid": 2597,
                "mapName": "obj/obj_howitzer ",
                "mapFileTested": "File tested and Validated",
                "downloads": 669,
                "mapFile": {
                  "filename": "obj_howitzer_v1_1.pk3",
                  "filesize": 1604726,
                  "downloadLink": "https://storage.moh-db.com/MOHAA-MAP-FILE/obj_howitzer_v1_1.pk3"
                },
                "downloadLink": "https://storage.moh-db.com/MOHAA-MAP-FILE/obj_howitzer_v1_1.pk3"
              }],
              "totalElements": 1
            }"#,
        )
        .expect("valid frozen API page");
        let candidate =
            CatalogueCandidate::from_wire(page.content.into_iter().next().expect("one API record"))
                .expect("record is downloadable");

        assert_eq!(page.total_elements, 1);
        assert_eq!(candidate.map_key.as_str(), "obj/obj_howitzer");
        assert_eq!(candidate.filename, "obj_howitzer_v1_1.pk3");
        assert!(candidate.map_file_tested);
        assert_eq!(candidate.downloads, 669);
    }
}
