// SPDX-License-Identifier: GPL-2.0-only

//! Signed, player-initiated updates for the Reveille application itself.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager as _};
use tauri_plugin_updater::{Update, UpdaterExt as _};
use thiserror::Error;
use tokio::sync::Notify;

pub const EVENT: &str = "reveille://self-update";
pub const PUBLIC_KEY: &str = match option_env!("REVEILLE_UPDATER_PUBKEY") {
    Some(key) => key,
    None => "",
};

const ENDPOINT: &str = "https://github.com/Feho/reveille/releases/latest/download/latest.json";
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_EVENT_STRIDE: u64 = 256 * 1024;

#[derive(Default)]
pub struct SelfUpdateState {
    operation: tokio::sync::Mutex<()>,
    pending: Mutex<Option<Update>>,
    cancel_generation: AtomicU64,
    cancel_download: Notify,
}

#[derive(Clone, Serialize)]
pub struct Offer {
    version: String,
    current_version: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Progress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Installing,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("another Reveille update operation is already running")]
    Busy,
    #[error("there is no checked Reveille update to install")]
    NoPendingUpdate,
    #[error("Reveille's update state is unavailable")]
    StateUnavailable,
    #[error("the Reveille update download was stopped")]
    Cancelled,
    #[error("Reveille's update endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error(transparent)]
    Updater(#[from] tauri_plugin_updater::Error),
}

impl Serialize for SelfUpdateError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Check the latest *published* GitHub release and retain the exact signed offer in Rust.
///
/// A development build has no embedded release key and therefore performs no network request.
#[tauri::command]
pub async fn check_reveille_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, SelfUpdateState>,
) -> Result<Option<Offer>, SelfUpdateError> {
    if PUBLIC_KEY.trim().is_empty() {
        return Ok(None);
    }

    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| SelfUpdateError::Busy)?;
    let endpoint = ENDPOINT
        .parse::<tauri::Url>()
        .map_err(|error| SelfUpdateError::InvalidEndpoint(error.to_string()))?;
    let update = app
        .updater_builder()
        .target("windows-x86_64")
        .pubkey(PUBLIC_KEY)
        .timeout(CHECK_TIMEOUT)
        .endpoints(vec![endpoint])?
        .build()?
        .check()
        .await?;
    let offer = update.as_ref().map(|update| Offer {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
    });
    *state
        .pending
        .lock()
        .map_err(|_| SelfUpdateError::StateUnavailable)? = update;
    Ok(offer)
}

/// Download, verify, and apply the exact update retained by [`check_reveille_update`].
///
/// On Windows `Update::install` exits Reveille before starting the NSIS replacement. The download
/// remains cancellable; cancellation is deliberately no longer observed after verification and
/// the atomic hand-off to the installer begins.
#[tauri::command]
pub async fn install_reveille_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, SelfUpdateState>,
) -> Result<(), SelfUpdateError> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| SelfUpdateError::Busy)?;
    let update = state
        .pending
        .lock()
        .map_err(|_| SelfUpdateError::StateUnavailable)?
        .clone()
        .ok_or(SelfUpdateError::NoPendingUpdate)?;
    let cancel_generation = state.cancel_generation.load(Ordering::Acquire);

    let progress_app = app.clone();
    let finish_app = app.clone();
    let mut received = 0_u64;
    let mut announced = 0_u64;
    let download = update.download(
        move |chunk_length, total| {
            received = received.saturating_add(chunk_length as u64);
            if received != total.unwrap_or_default()
                && received < announced.saturating_add(DOWNLOAD_EVENT_STRIDE)
            {
                return;
            }
            announced = received;
            drop(progress_app.emit(EVENT, Progress::Downloading { received, total }));
        },
        move || {
            drop(finish_app.emit(EVENT, Progress::Verifying));
        },
    );
    tokio::pin!(download);
    let cancelled = async {
        loop {
            if state.cancel_generation.load(Ordering::Acquire) != cancel_generation {
                return;
            }
            state.cancel_download.notified().await;
        }
    };
    tokio::pin!(cancelled);
    let bytes = tokio::select! {
        biased;
        () = &mut cancelled => None,
        result = &mut download => Some(result?),
    };
    let Some(bytes) = bytes else {
        drop(app.emit(EVENT, Progress::Cancelled));
        return Err(SelfUpdateError::Cancelled);
    };
    if state.cancel_generation.load(Ordering::Acquire) != cancel_generation {
        drop(app.emit(EVENT, Progress::Cancelled));
        return Err(SelfUpdateError::Cancelled);
    }

    drop(app.emit(EVENT, Progress::Installing));
    update.install(bytes)?;
    Ok(())
}

#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri resolves managed state only for by-value command parameters"
)]
pub fn cancel_reveille_update(state: tauri::State<'_, SelfUpdateState>) {
    state.cancel_generation.fetch_add(1, Ordering::AcqRel);
    state.cancel_download.notify_waiters();
}

pub fn register(app: &mut tauri::App) {
    app.manage(SelfUpdateState::default());
}
