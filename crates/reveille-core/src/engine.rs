// SPDX-License-Identifier: GPL-2.0-only

//! Engine identity shared by every front end.

use serde::{Deserialize, Serialize};

/// Game program selected for browsing, previewing, and launching.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    /// The executables supplied with the player's game.
    Original,
    /// The modern rebuilt engine.
    Openmohaa,
    /// The classic executable family with community fixes.
    Reborn,
}
