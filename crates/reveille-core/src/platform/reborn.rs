// SPDX-License-Identifier: GPL-2.0-only

//! Frozen legacy Reborn player packages and provider-neutral-safe extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read as _};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::ZipArchive;

use super::package as package_io;
use crate::install::Product;

/// Immutable official documentation repository revision containing the supported packages.
pub const SOURCE_COMMIT: &str = "15451e40274e718870dcf8ba295bb8fcde745857";
const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024;
const USER_AGENT: &str = "Reveille/0.1";

/// Reborn player archive selected from installed product data directories.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornProductSet {
    Aa,
    AaSh,
    AaBt,
    AaShBt,
}

impl RebornProductSet {
    /// Select the smallest official package covering every detected product.
    #[must_use]
    pub fn from_products(products: &[Product]) -> Self {
        let spearhead = products.contains(&Product::Spearhead);
        let breakthrough = products.contains(&Product::Breakthrough);
        match (spearhead, breakthrough) {
            (false, false) => Self::Aa,
            (true, false) => Self::AaSh,
            (false, true) => Self::AaBt,
            (true, true) => Self::AaShBt,
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::Aa => "aa",
            Self::AaSh => "aa_sh",
            Self::AaBt => "aa_bt",
            Self::AaShBt => "aa_sh_bt",
        }
    }

    /// Canonical executable names this package must contain.
    #[must_use]
    pub fn executables(self) -> &'static [&'static str] {
        match self {
            Self::Aa => &["MOHAA.exe"],
            Self::AaSh => &["MOHAA.exe", "moh_spearhead.exe"],
            Self::AaBt => &["MOHAA.exe", "moh_breakthrough.exe"],
            Self::AaShBt => &["MOHAA.exe", "moh_spearhead.exe", "moh_breakthrough.exe"],
        }
    }
}

/// Exact executable digest in every supported pinned package.
#[must_use]
pub fn expected_executable_sha256(filename: &str) -> Option<&'static str> {
    match filename.to_ascii_lowercase().as_str() {
        "mohaa.exe" => Some("4cd6c9a2558d90adc91c1987a4488690a1fc00a89e181d26365ce8361896fab4"),
        "moh_spearhead.exe" => {
            Some("7a29e243ac7559ee53095004959d475a5e8f2fc9d9df1c1a2488d2696ffe0b82")
        }
        "moh_breakthrough.exe" => {
            Some("7b6f73863952a289e1fea91a40036f7fa4efba0ac94d89c1ea9582e3604370ad")
        }
        _ => None,
    }
}

/// One pinned official Reborn legacy player archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RebornPackage {
    pub product_set: RebornProductSet,
    pub version: &'static str,
    pub filename: String,
    pub download_url: String,
    pub size: u64,
    pub sha256: &'static str,
}

/// Package metadata frozen from the immutable repository revision.
#[must_use]
pub fn package(product_set: RebornProductSet) -> RebornPackage {
    let (size, sha256) = match product_set {
        RebornProductSet::Aa => (
            733_577,
            "e38a41810a81e40239245c57d549ee19250f84e46595c1d93d1cddea71d6f333",
        ),
        RebornProductSet::AaSh => (
            1_537_186,
            "fc586d1739fc390709bf07ea9237ae02a24aab84f504a295b5975d0cbc349a45",
        ),
        RebornProductSet::AaBt => (
            1_553_694,
            "7ac402f4d74893c4df06c6d418162812cbfb3060ce353e914bee2d21908e9dc0",
        ),
        RebornProductSet::AaShBt => (
            2_357_315,
            "425cbe3a4253f62b9f088c7715d393b17b929b56631f230ebd99de88d45be457",
        ),
    };
    let filename = format!("mohreborn_{}.zip", product_set.slug());
    RebornPackage {
        product_set,
        version: "Reborn 1.12",
        download_url: format!(
            "https://raw.githubusercontent.com/mohreborn/mohreborn-docs/{SOURCE_COMMIT}/docs/assets/moh-binaries/{filename}"
        ),
        filename,
        size,
        sha256,
    }
}

/// One verified executable extracted from a pinned package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebornExecutable {
    pub filename: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Bytes received while downloading a Reborn archive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

/// HTTP client for the immutable legacy distribution.
#[derive(Clone, Debug)]
pub struct RebornClient {
    client: Client,
}

impl RebornClient {
    /// Construct a finite-deadline client.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be configured.
    pub fn new(timeout: Duration) -> Result<Self, RebornError> {
        Ok(Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .timeout(timeout)
                .build()
                .map_err(RebornError::Client)?,
        })
    }

    /// Download and verify a pinned archive, with cancellation between response chunks.
    ///
    /// # Errors
    ///
    /// Returns an error for transfer, cancellation, size, or digest failures.
    pub async fn download_reporting<F>(
        &self,
        package: &RebornPackage,
        mut report: F,
    ) -> Result<Vec<u8>, RebornError>
    where
        F: FnMut(DownloadProgress) -> ControlFlow<()>,
    {
        let bytes = package_io::download_reporting(
            &self.client,
            &package.download_url,
            package.size,
            MAX_PACKAGE_BYTES,
            |progress| {
                report(DownloadProgress {
                    received: progress.received,
                    total: progress.total,
                })
            },
        )
        .await
        .map_err(|error| match error {
            package_io::DownloadError::Network(source) => RebornError::Network(source),
            package_io::DownloadError::HttpStatus(status) => RebornError::HttpStatus(status),
            package_io::DownloadError::TooLarge { size, .. } => RebornError::TooLarge(size),
            package_io::DownloadError::Cancelled => RebornError::Cancelled,
        })?;
        verify_archive(package, &bytes)?;
        Ok(bytes)
    }
}

/// Verify package identity and return only its exact expected executables.
///
/// # Errors
///
/// Returns an error for identity mismatch or any unsafe, missing, duplicate, or unexpected entry.
pub fn inspect_package(
    package: &RebornPackage,
    bytes: &[u8],
) -> Result<Vec<RebornExecutable>, RebornError> {
    verify_archive(package, bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(RebornError::InvalidZip)?;
    let expected = package
        .product_set
        .executables()
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let prefix = format!("mohreborn_{}/", package.product_set.slug());
    let mut found = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(RebornError::InvalidZip)?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(RebornError::UnsafeEntry(entry.name().to_owned()));
        }
        let path = safe_path(entry.name())?;
        let normalized = path.to_string_lossy().replace('\\', "/");
        let filename = normalized
            .strip_prefix(&prefix)
            .filter(|name| !name.contains('/'))
            .ok_or_else(|| RebornError::UnexpectedEntry(entry.name().to_owned()))?;
        let key = filename.to_ascii_lowercase();
        if !expected.contains(&key) {
            return Err(RebornError::UnexpectedEntry(entry.name().to_owned()));
        }
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(RebornError::ArchiveIo)?;
        let hash = format!("{:x}", Sha256::digest(&contents));
        if found
            .insert(
                key,
                RebornExecutable {
                    filename: filename.to_owned(),
                    bytes: contents,
                    sha256: hash,
                },
            )
            .is_some()
        {
            return Err(RebornError::DuplicateEntry(filename.to_owned()));
        }
    }
    let missing = expected
        .iter()
        .find(|name| !found.contains_key(*name))
        .cloned();
    if let Some(filename) = missing {
        return Err(RebornError::MissingExecutable(filename));
    }
    Ok(found.into_values().collect())
}

fn verify_archive(package: &RebornPackage, bytes: &[u8]) -> Result<(), RebornError> {
    package_io::verify_sha256(bytes, package.size, package.sha256)
        .map(|_| ())
        .map_err(|error| match error {
            package_io::VerifyError::Size { expected, actual } => {
                RebornError::SizeMismatch { expected, actual }
            }
            package_io::VerifyError::Digest { actual, .. } => RebornError::DigestMismatch {
                expected: package.sha256,
                actual,
            },
        })
}

fn safe_path(value: &str) -> Result<PathBuf, RebornError> {
    package_io::safe_archive_path(value).ok_or_else(|| RebornError::UnsafeEntry(value.to_owned()))
}

/// Reborn package discovery, verification, or archive failure.
#[derive(Debug, Error)]
pub enum RebornError {
    #[error("could not configure the Reborn download client")]
    Client(#[source] reqwest::Error),
    #[error("Reborn download failed")]
    Network(#[source] reqwest::Error),
    #[error("Reborn download returned HTTP {0}")]
    HttpStatus(u16),
    #[error("Reborn download was cancelled")]
    Cancelled,
    #[error("Reborn archive is too large: {0} bytes")]
    TooLarge(u64),
    #[error("Reborn archive size differs: expected {expected}, downloaded {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("Reborn archive digest differs: expected {expected}, downloaded {actual}")]
    DigestMismatch {
        expected: &'static str,
        actual: String,
    },
    #[error("Reborn package is not a readable ZIP archive")]
    InvalidZip(#[source] zip::result::ZipError),
    #[error("could not read a Reborn archive entry")]
    ArchiveIo(#[source] std::io::Error),
    #[error("unsafe Reborn archive entry {0:?}")]
    UnsafeEntry(String),
    #[error("unexpected Reborn archive entry {0:?}")]
    UnexpectedEntry(String),
    #[error("duplicate Reborn archive entry {0:?}")]
    DuplicateEntry(String),
    #[error("Reborn archive is missing expected executable {0:?}")]
    MissingExecutable(String),
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn all_product_sets_map_to_the_frozen_metadata() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/reborn_packages.json"))
                .expect("metadata fixture");
        for product_set in [
            RebornProductSet::Aa,
            RebornProductSet::AaSh,
            RebornProductSet::AaBt,
            RebornProductSet::AaShBt,
        ] {
            let package = package(product_set);
            let row = &fixture[product_set.slug()];
            assert_eq!(row["filename"], package.filename);
            assert_eq!(row["size"], package.size);
            assert_eq!(row["sha256"], package.sha256);
            assert!(package.download_url.contains(SOURCE_COMMIT));
        }
    }

    #[test]
    fn product_detection_selects_all_four_packages() {
        assert_eq!(
            RebornProductSet::from_products(&[Product::AlliedAssault]),
            RebornProductSet::Aa
        );
        assert_eq!(
            RebornProductSet::from_products(&[Product::AlliedAssault, Product::Spearhead]),
            RebornProductSet::AaSh
        );
        assert_eq!(
            RebornProductSet::from_products(&[Product::AlliedAssault, Product::Breakthrough]),
            RebornProductSet::AaBt
        );
        assert_eq!(
            RebornProductSet::from_products(&[
                Product::AlliedAssault,
                Product::Spearhead,
                Product::Breakthrough
            ]),
            RebornProductSet::AaShBt
        );
    }

    #[test]
    fn archive_requires_the_exact_expected_executables_and_rejects_other_entries() {
        let valid = archive(&[("mohreborn_aa/MOHAA.exe", b"client")]);
        let package = fixture_package(RebornProductSet::Aa, &valid);
        let files = inspect_package(&package, &valid).expect("strict package");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "MOHAA.exe");

        let missing = archive(&[("mohreborn_aa/readme.txt", b"not the client")]);
        let missing_package = fixture_package(RebornProductSet::Aa, &missing);
        assert!(matches!(
            inspect_package(&missing_package, &missing),
            Err(RebornError::UnexpectedEntry(_))
        ));

        let absent = archive(&[]);
        let absent_package = fixture_package(RebornProductSet::Aa, &absent);
        assert!(matches!(
            inspect_package(&absent_package, &absent),
            Err(RebornError::MissingExecutable(_))
        ));

        let unsafe_archive = archive(&[("../MOHAA.exe", b"client")]);
        let unsafe_package = fixture_package(RebornProductSet::Aa, &unsafe_archive);
        assert!(matches!(
            inspect_package(&unsafe_package, &unsafe_archive),
            Err(RebornError::UnsafeEntry(_))
        ));
    }

    #[test]
    fn size_and_digest_mismatches_stop_before_zip_inspection() {
        let bytes = archive(&[("mohreborn_aa/MOHAA.exe", b"client")]);
        let mut wrong_size = fixture_package(RebornProductSet::Aa, &bytes);
        wrong_size.size += 1;
        assert!(matches!(
            inspect_package(&wrong_size, &bytes),
            Err(RebornError::SizeMismatch { .. })
        ));

        let mut wrong_digest = fixture_package(RebornProductSet::Aa, &bytes);
        wrong_digest.sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(matches!(
            inspect_package(&wrong_digest, &bytes),
            Err(RebornError::DigestMismatch { .. })
        ));
    }

    fn fixture_package(product_set: RebornProductSet, bytes: &[u8]) -> RebornPackage {
        let digest = Box::leak(format!("{:x}", Sha256::digest(bytes)).into_boxed_str());
        RebornPackage {
            product_set,
            version: "fixture",
            filename: format!("mohreborn_{}.zip", product_set.slug()),
            download_url: "https://example.invalid/reborn.zip".to_owned(),
            size: bytes.len() as u64,
            sha256: digest,
        }
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut archive = ZipWriter::new(&mut bytes);
            for (name, contents) in entries {
                archive
                    .start_file(*name, SimpleFileOptions::default())
                    .expect("archive entry");
                archive.write_all(contents).expect("entry contents");
            }
            archive.finish().expect("finish archive");
        }
        bytes.into_inner()
    }
}
