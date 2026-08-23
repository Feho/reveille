// SPDX-License-Identifier: GPL-2.0-only

//! Provider-neutral verified-package download and filesystem mechanics.

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};

use reqwest::Client;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

/// One progress sample shared by engine package providers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

/// Download bytes with a hard size ceiling and cancellation between chunks.
///
/// # Errors
///
/// Returns an error for network/HTTP failures, size overflow, or requested cancellation.
pub async fn download_reporting<F>(
    client: &Client,
    url: &str,
    expected_size: u64,
    maximum: u64,
    mut report: F,
) -> Result<Vec<u8>, DownloadError>
where
    F: FnMut(DownloadProgress) -> ControlFlow<()>,
{
    if report(DownloadProgress {
        received: 0,
        total: Some(expected_size),
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(DownloadError::Network)?;
    if !response.status().is_success() {
        return Err(DownloadError::HttpStatus(response.status().as_u16()));
    }
    let declared = response.content_length();
    if let Some(size) = declared.filter(|size| *size > maximum) {
        return Err(DownloadError::TooLarge { size, maximum });
    }
    let total = declared.or(Some(expected_size));
    if report(DownloadProgress { received: 0, total }).is_break() {
        return Err(DownloadError::Cancelled);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(expected_size).unwrap_or_default());
    while let Some(chunk) = response.chunk().await.map_err(DownloadError::Network)? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > maximum {
            return Err(DownloadError::TooLarge {
                size: bytes.len() as u64,
                maximum,
            });
        }
        if report(DownloadProgress {
            received: bytes.len() as u64,
            total,
        })
        .is_break()
        {
            return Err(DownloadError::Cancelled);
        }
    }
    Ok(bytes)
}

/// Verify exact byte count and SHA-256, returning the actual lowercase digest.
///
/// # Errors
///
/// Returns the observed mismatch without accepting partial or differently hashed bytes.
pub fn verify_sha256(bytes: &[u8], size: u64, expected: &str) -> Result<String, VerifyError> {
    if bytes.len() as u64 != size {
        return Err(VerifyError::Size {
            expected: size,
            actual: bytes.len() as u64,
        });
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(VerifyError::Digest {
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(actual)
}

/// Normalize a ZIP path while rejecting traversal and Windows absolute/device syntax.
#[must_use]
pub fn safe_archive_path(value: &str) -> Option<PathBuf> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    let path = PathBuf::from(normalized);
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        .then_some(path)
}

/// One staged file to transactionally overlay at a final path.
pub struct OverlayFile<'a> {
    pub source: &'a Path,
    pub target: PathBuf,
    pub executable: bool,
}

struct PreparedFile {
    target: PathBuf,
    replacement: Option<NamedTempFile>,
    backup: Option<NamedTempFile>,
}

/// Replace all targets as one rollback-capable overlay.
///
/// # Errors
///
/// Returns an error when preparation or commit fails; committed targets are rolled back.
pub fn transactional_overlay(files: &[OverlayFile<'_>]) -> Result<(), ApplyError> {
    transactional_overlay_before_commit(files, || Ok(()))
}

fn transactional_overlay_before_commit<F>(
    files: &[OverlayFile<'_>],
    before_commit: F,
) -> Result<(), ApplyError>
where
    F: FnOnce() -> Result<(), ApplyError>,
{
    let mut prepared = Vec::with_capacity(files.len());
    for file in files {
        if file.target.is_dir() {
            return Err(ApplyError::TargetDirectory(file.target.clone()));
        }
        let parent = file
            .target
            .parent()
            .ok_or_else(|| ApplyError::NoParent(file.target.clone()))?;
        fs::create_dir_all(parent).map_err(|source| ApplyError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
        let backup = if file.target.exists() {
            let backup =
                NamedTempFile::new_in(parent).map_err(|source| ApplyError::Filesystem {
                    path: parent.to_path_buf(),
                    source,
                })?;
            fs::copy(&file.target, backup.path()).map_err(|source| ApplyError::Filesystem {
                path: file.target.clone(),
                source,
            })?;
            Some(backup)
        } else {
            None
        };
        let mut replacement =
            NamedTempFile::new_in(parent).map_err(|source| ApplyError::Filesystem {
                path: parent.to_path_buf(),
                source,
            })?;
        let mut input = File::open(file.source).map_err(|source| ApplyError::Filesystem {
            path: file.source.to_path_buf(),
            source,
        })?;
        io::copy(&mut input, &mut replacement).map_err(|source| ApplyError::Filesystem {
            path: file.target.clone(),
            source,
        })?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(replacement.path(), fs::Permissions::from_mode(0o755)).map_err(
                |source| ApplyError::Filesystem {
                    path: file.target.clone(),
                    source,
                },
            )?;
        }
        replacement
            .flush()
            .map_err(|source| ApplyError::Filesystem {
                path: file.target.clone(),
                source,
            })?;
        prepared.push(PreparedFile {
            target: file.target.clone(),
            replacement: Some(replacement),
            backup,
        });
    }
    before_commit()?;
    for index in 0..prepared.len() {
        let replacement = prepared[index]
            .replacement
            .take()
            .ok_or(ApplyError::Incomplete)?;
        if let Err(error) = replacement.persist(&prepared[index].target) {
            rollback(&mut prepared, index);
            return Err(ApplyError::Filesystem {
                path: prepared[index].target.clone(),
                source: error.error,
            });
        }
    }
    Ok(())
}

fn rollback(files: &mut [PreparedFile], committed: usize) {
    for file in files[..committed].iter_mut().rev() {
        if let Some(backup) = file.backup.take() {
            let _ = backup.persist(&file.target);
        } else {
            let _ = fs::remove_file(&file.target);
        }
    }
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("package request failed")]
    Network(#[source] reqwest::Error),
    #[error("package server returned HTTP {0}")]
    HttpStatus(u16),
    #[error("package is {size} bytes; maximum is {maximum}")]
    TooLarge { size: u64, maximum: u64 },
    #[error("package download was cancelled")]
    Cancelled,
}

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("package size differs: expected {expected}, downloaded {actual}")]
    Size { expected: u64, actual: u64 },
    #[error("package digest differs: expected {expected}, downloaded {actual}")]
    Digest { expected: String, actual: String },
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("overlay target has no parent: {0}")]
    NoParent(PathBuf),
    #[error("overlay target is a directory: {0}")]
    TargetDirectory(PathBuf),
    #[error("overlay transaction was internally incomplete")]
    Incomplete,
    #[error("filesystem operation failed at {path}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn cancellation_can_stop_before_any_network_request() {
        let client = Client::builder()
            .timeout(Duration::from_millis(1))
            .build()
            .expect("client");
        let result = download_reporting(
            &client,
            "https://example.invalid/never-requested",
            1,
            2,
            |_| ControlFlow::Break(()),
        )
        .await;
        assert!(matches!(result, Err(DownloadError::Cancelled)));
    }

    #[test]
    fn a_partial_commit_restores_every_canonical_target() {
        let temporary = tempfile::TempDir::new().expect("temporary directory");
        let source_one = temporary.path().join("source-one");
        let source_two = temporary.path().join("source-two");
        let target_one = temporary.path().join("target-one");
        let target_two = temporary.path().join("target-two");
        fs::write(&source_one, b"new one").expect("first source");
        fs::write(&source_two, b"new two").expect("second source");
        fs::write(&target_one, b"old one").expect("first target");
        let files = [
            OverlayFile {
                source: &source_one,
                target: target_one.clone(),
                executable: false,
            },
            OverlayFile {
                source: &source_two,
                target: target_two.clone(),
                executable: false,
            },
        ];
        let result = transactional_overlay_before_commit(&files, || {
            fs::create_dir(&target_two).map_err(|source| ApplyError::Filesystem {
                path: target_two.clone(),
                source,
            })
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(&target_one).expect("restored first target"),
            b"old one"
        );
        assert!(target_two.is_dir());
    }
}
