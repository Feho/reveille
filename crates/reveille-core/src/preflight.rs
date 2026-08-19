// SPDX-License-Identifier: GPL-2.0-only

//! Compare a server rotation with locally indexed content.

use serde::Serialize;

use crate::bsp::Checksum;
use crate::mapindex::{MapIndex, MapKey};

/// A checksum published by a server for its current map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublishedChecksum {
    /// Server map name to which the checksum applies.
    pub map: String,
    /// `sv_mapChecksum` value.
    pub checksum: Checksum,
}

/// Local status of one server rotation entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MapStatus {
    /// The effective local BSP exists and no checked fact differs.
    Present {
        /// Effective local BSP checksum.
        checksum: Checksum,
        /// Whether this checksum was checked against a published server value.
        checksum_checked: bool,
    },
    /// The map exists, but the BSP the engine loads differs from the server value.
    ChecksumDiffers {
        /// Effective local checksum.
        local: Checksum,
        /// Published server checksum.
        server: Checksum,
    },
    /// No local provider exists.
    Absent,
}

/// Result for one normalized rotation entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MapResult {
    /// Name as published by the server.
    pub map: String,
    /// Local content status.
    pub status: MapStatus,
}

/// Structured overall verdict. It deliberately cannot be mistaken for a boolean certainty.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing Reveille could check is wrong.
    Compatible,
    /// One or more maps are absent or have a checked checksum mismatch.
    ProblemsFound {
        /// Number of absent rotation entries.
        absent: usize,
        /// Number of checked checksum mismatches.
        checksum_mismatches: usize,
    },
}

/// Complete content preflight result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Report {
    /// Overall structured verdict.
    pub verdict: Verdict,
    /// Per-rotation-entry details.
    pub maps: Vec<MapResult>,
}

/// Check a server rotation and, when available, the current map's published checksum.
#[must_use]
pub fn check(
    index: &MapIndex,
    rotation: &[impl AsRef<str>],
    published_checksum: Option<&PublishedChecksum>,
) -> Report {
    let mut absent = 0;
    let mut checksum_mismatches = 0;
    let maps = rotation
        .iter()
        .map(|server_name| {
            let server_name = server_name.as_ref();
            let local = index.get(server_name);
            let expected = published_checksum
                .filter(|published| MapKey::new(&published.map) == MapKey::new(server_name));
            let status = match (local, expected) {
                (None, _) => {
                    absent += 1;
                    MapStatus::Absent
                }
                (Some(map), Some(expected)) => {
                    let local = map
                        .effective_provider()
                        .map(crate::mapindex::Provider::checksum);
                    let Some(local) = local else {
                        absent += 1;
                        return MapResult {
                            map: server_name.to_owned(),
                            status: MapStatus::Absent,
                        };
                    };
                    if local == expected.checksum {
                        MapStatus::Present {
                            checksum: local,
                            checksum_checked: true,
                        }
                    } else {
                        checksum_mismatches += 1;
                        MapStatus::ChecksumDiffers {
                            local,
                            server: expected.checksum,
                        }
                    }
                }
                (Some(map), None) => {
                    if let Some(provider) = map.effective_provider() {
                        MapStatus::Present {
                            checksum: provider.checksum(),
                            checksum_checked: false,
                        }
                    } else {
                        absent += 1;
                        MapStatus::Absent
                    }
                }
            };
            MapResult {
                map: server_name.to_owned(),
                status,
            }
        })
        .collect();

    let verdict = if absent == 0 && checksum_mismatches == 0 {
        Verdict::Compatible
    } else {
        Verdict::ProblemsFound {
            absent,
            checksum_mismatches,
        }
    };
    Report { verdict, maps }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;

    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{MapStatus, PublishedChecksum, Verdict, check};
    use crate::bsp::Checksum;
    use crate::mapindex::MapIndex;

    fn fixture_index() -> (TempDir, MapIndex) {
        let temporary = TempDir::new().expect("temporary directory");
        let main = temporary.path().join("main");
        fs::create_dir(&main).expect("main directory");
        let file = File::create(main.join("maps.pk3")).expect("create archive");
        let mut archive = ZipWriter::new(file);
        archive
            .start_file("maps/dm/example.bsp", SimpleFileOptions::default())
            .expect("start entry");
        let mut header = *b"2015\x13\0\0\0\0\0\0\0";
        header[8..12].copy_from_slice(&42_i32.to_le_bytes());
        archive.write_all(&header).expect("write BSP");
        archive.finish().expect("finish archive");
        let index = MapIndex::scan(&main).expect("scan map index");
        (temporary, index)
    }

    #[test]
    fn reports_present_absent_and_mismatched_maps() {
        let (_temporary, index) = fixture_index();
        let rotation = ["DM/example", "obj/missing"];
        let published = PublishedChecksum {
            map: "dm/example".into(),
            checksum: Checksum::new(99),
        };

        let report = check(&index, &rotation, Some(&published));
        assert_eq!(
            report.verdict,
            Verdict::ProblemsFound {
                absent: 1,
                checksum_mismatches: 1,
            }
        );
        assert!(matches!(
            report.maps[0].status,
            MapStatus::ChecksumDiffers {
                local,
                server
            } if local == Checksum::new(42) && server == Checksum::new(99)
        ));
        assert_eq!(report.maps[1].status, MapStatus::Absent);
    }

    #[test]
    fn compatible_means_nothing_checkable_is_wrong() {
        let (_temporary, index) = fixture_index();
        let rotation = ["dm/example"];

        let report = check(&index, &rotation, None);
        assert_eq!(report.verdict, Verdict::Compatible);
        assert!(matches!(
            report.maps[0].status,
            MapStatus::Present {
                checksum,
                checksum_checked: false
            } if checksum == Checksum::new(42)
        ));
    }
}
