// SPDX-License-Identifier: GPL-2.0-only

//! Platform-neutral identification of a user-selected MOHAA installation.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A game whose asset directory is present.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    /// Medal of Honor: Allied Assault (`main`).
    AlliedAssault,
    /// Spearhead (`mainta`).
    Spearhead,
    /// Breakthrough (`maintt`).
    Breakthrough,
}

/// A recognized client executable and its SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BinaryFingerprint {
    /// Original path to the client binary.
    pub path: PathBuf,
    /// Lowercase hexadecimal SHA-256 digest.
    pub sha256: String,
    /// Curated version label, when this digest is known-good.
    pub known_version: Option<String>,
}

/// How an installation was identified. Consumers must handle uncertainty explicitly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum IdentificationMethod {
    /// At least one client binary matched the curated known-good digest corpus.
    KnownBinaryHashes,
    /// Client binary names were recognized, but none of their hashes are in the corpus yet.
    RecognizedBinaryUnknownHashes,
    /// Asset directories exist, but no recognized client executable was found.
    DataDirectoriesOnly,
}

/// Result of identifying a path selected by the user.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Installation {
    /// Canonicalized installation root.
    pub root: PathBuf,
    /// Products inferred from data directories, in base-to-expansion order.
    pub products: Vec<Product>,
    /// Recognized client binaries and their identities.
    pub binaries: Vec<BinaryFingerprint>,
    /// Evidence used for this identification.
    pub identification: IdentificationMethod,
}

/// An error encountered while identifying an install.
#[derive(Debug, Error)]
pub enum Error {
    /// The selected path is not a directory.
    #[error("install path is not a directory: {0}")]
    NotDirectory(PathBuf),
    /// None of the MOHAA asset directories are present.
    #[error("no main, mainta, or maintt data directory found in {0}")]
    NoDataDirectories(PathBuf),
    /// Filesystem metadata could not be read.
    #[error("could not inspect {path}")]
    Io {
        /// Path being inspected.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

const DATA_DIRECTORIES: [(&str, Product); 3] = [
    ("main", Product::AlliedAssault),
    ("mainta", Product::Spearhead),
    ("maintt", Product::Breakthrough),
];

const CLIENT_BINARIES: [&str; 7] = [
    "mohaa.exe",
    "mohaas.exe",
    "mohaab.exe",
    "moh_spearhead.exe",
    "moh_breakthrough.exe",
    "openmohaa.exe",
    "openmohaa",
];

// Measured from a French retail-disc install on Windows, 19 Aug 2026.
const KNOWN_BINARY_VERSIONS: [(&str, &str); 3] = [
    (
        "ed028e97cb56ea3a89a821635b07e0ed87bcbab751b6e13e88edc9c02dfc88cc",
        "Medal of Honor: Allied Assault 1.11 (retail disc, French)",
    ),
    (
        "74de88a1721277d509172966600fd00c34a71d4003ea008af48e230468154ac6",
        "Medal of Honor: Allied Assault Spearhead 2.15 (retail disc, French)",
    ),
    (
        "7a6ee79a01b82dce2fce36e8eb474cc32718bccb58b039847798c222e0113ccf",
        "Medal of Honor: Allied Assault Breakthrough 2.40 (retail disc, French)",
    ),
];

/// Identify a user-selected install root without applying platform discovery policy.
///
/// # Errors
///
/// Returns an error when the path is not a directory, has no recognized asset directory, or a
/// recognized client binary cannot be read and hashed.
pub fn identify(path: impl AsRef<Path>) -> Result<Installation, Error> {
    let path = path.as_ref();
    if !path.is_dir() {
        return Err(Error::NotDirectory(path.to_path_buf()));
    }

    let root = path.canonicalize().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let products = DATA_DIRECTORIES
        .iter()
        .filter_map(|(directory, product)| root.join(directory).is_dir().then_some(*product))
        .collect::<Vec<_>>();
    if products.is_empty() {
        return Err(Error::NoDataDirectories(root));
    }

    let mut binaries = Vec::new();
    for file_name in CLIENT_BINARIES {
        let binary = root.join(file_name);
        if binary.is_file() {
            let sha256 = hash_file(&binary)?;
            let known_version = known_version(&sha256).map(str::to_owned);
            binaries.push(BinaryFingerprint {
                path: binary,
                sha256,
                known_version,
            });
        }
    }

    let identification = if binaries.iter().any(|binary| binary.known_version.is_some()) {
        IdentificationMethod::KnownBinaryHashes
    } else if binaries.is_empty() {
        IdentificationMethod::DataDirectoriesOnly
    } else {
        IdentificationMethod::RecognizedBinaryUnknownHashes
    };

    Ok(Installation {
        root,
        products,
        binaries,
        identification,
    })
}

fn known_version(hash: &str) -> Option<&'static str> {
    KNOWN_BINARY_VERSIONS
        .iter()
        .find_map(|(known_hash, version)| (*known_hash == hash).then_some(*version))
}

fn hash_file(path: &Path) -> Result<String, Error> {
    let file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{Error, IdentificationMethod, Product, identify, known_version};

    #[test]
    fn identifies_products_and_exposes_an_unknown_binary_hash() {
        let temporary = TempDir::new().expect("temporary directory");
        fs::create_dir(temporary.path().join("main")).expect("main directory");
        fs::create_dir(temporary.path().join("mainta")).expect("mainta directory");
        fs::write(temporary.path().join("mohaa.exe"), b"synthetic client").expect("client");

        let install = identify(temporary.path()).expect("identify install");
        assert_eq!(
            install.products,
            vec![Product::AlliedAssault, Product::Spearhead]
        );
        assert_eq!(
            install.identification,
            IdentificationMethod::RecognizedBinaryUnknownHashes
        );
        assert_eq!(install.binaries.len(), 1);
        assert_eq!(
            install.binaries[0].sha256,
            "186cb4adf190a9e198815dc58eadf188c96d5eff95a8eb6f23d5693311a55268"
        );
        assert_eq!(install.binaries[0].known_version, None);
    }

    #[test]
    fn accepts_assets_without_claiming_a_binary_version() {
        let temporary = TempDir::new().expect("temporary directory");
        fs::create_dir(temporary.path().join("main")).expect("main directory");

        let install = identify(temporary.path()).expect("identify assets");
        assert_eq!(
            install.identification,
            IdentificationMethod::DataDirectoriesOnly
        );
    }

    #[test]
    fn rejects_unrelated_directories() {
        let temporary = TempDir::new().expect("temporary directory");
        assert!(matches!(
            identify(temporary.path()),
            Err(Error::NoDataDirectories(_))
        ));
    }

    #[test]
    fn identifies_the_measured_retail_disc_hash() {
        assert_eq!(
            known_version("ed028e97cb56ea3a89a821635b07e0ed87bcbab751b6e13e88edc9c02dfc88cc"),
            Some("Medal of Honor: Allied Assault 1.11 (retail disc, French)")
        );
    }
}
