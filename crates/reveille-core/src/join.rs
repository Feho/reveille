// SPDX-License-Identifier: GPL-2.0-only

//! Headless join preparation: honest compatibility states, launch arguments, and post-hoc
//! rejection explanations.

use std::fmt;
use std::net::SocketAddrV4;

use serde::Serialize;
use thiserror::Error;

use crate::content::{CatalogueResolutionPass, ResolutionOutcome};
use crate::discovery::{Server, TargetGame};
use crate::mapindex::MapIndex;
use crate::preflight::{PublishedChecksum, Report, Verdict};

/// Number of distinct maps known to require different or absent local content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MapsNeeded(usize);

impl MapsNeeded {
    /// Construct a count from a preflight report.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    /// Return the underlying count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl fmt::Display for MapsNeeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The four compatibility states shown to a player before launch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CompatibilityState {
    /// Nothing in the server-published rotation that Reveille could check is wrong.
    ///
    /// This does not predict admission: bans, capacity, and ping are evaluated later by
    /// `SV_DirectConnect`.
    Compatible,
    /// The published rotation proves that local content is absent or differs.
    NeedsMaps {
        /// Number of affected distinct maps.
        count: MapsNeeded,
        /// Optional source-resolution details. The verdict does not depend on their presence.
        shopping_list: Option<CatalogueResolutionPass>,
    },
    /// Every needed entry was conclusively looked up and none has a catalogue source.
    NoSource {
        /// Number of affected distinct maps.
        count: MapsNeeded,
    },
    /// The server did not publish a usable rotation, so there is no preflight evidence.
    CantTell,
}

/// Classify preflight evidence, optionally attaching a later catalogue resolution pass.
///
/// `preflight` must be `None` when the server did not publish a usable `sv_maplist`. Keeping it
/// separate from `resolution` makes `Can't tell` reachable without contacting moh-db.
#[must_use]
pub fn classify(
    preflight: Option<&Report>,
    resolution: Option<&CatalogueResolutionPass>,
) -> CompatibilityState {
    let Some(preflight) = preflight else {
        return CompatibilityState::CantTell;
    };
    let Verdict::ProblemsFound {
        absent,
        checksum_mismatches,
    } = preflight.verdict
    else {
        return CompatibilityState::Compatible;
    };
    let count = MapsNeeded::new(absent + checksum_mismatches);
    let conclusive_no_source = resolution.is_some_and(|pass| {
        pass.non_results.is_empty()
            && pass.resolutions.len() == count.get()
            && pass
                .resolutions
                .iter()
                .all(|item| item.outcome == ResolutionOutcome::NoSource)
    });
    if conclusive_no_source {
        CompatibilityState::NoSource { count }
    } else {
        CompatibilityState::NeedsMaps {
            count,
            shopping_list: resolution.cloned(),
        }
    }
}

/// Preflight and classify one complete discovery model.
///
/// An empty rotation is treated as unpublished evidence, never as an empty compatible rotation.
#[must_use]
pub fn classify_server(
    index: &MapIndex,
    server: &Server,
    resolution: Option<&CatalogueResolutionPass>,
) -> CompatibilityAssessment {
    if server.rotation.is_empty() {
        return CompatibilityAssessment {
            state: classify(None, resolution),
            preflight: None,
        };
    }
    let published_checksum =
        server
            .current_map
            .as_ref()
            .zip(server.map_checksum)
            .map(|(map, checksum)| PublishedChecksum {
                map: map.clone(),
                checksum,
            });
    let preflight = crate::preflight::check(index, &server.rotation, published_checksum.as_ref());
    let state = classify(Some(&preflight), resolution);
    CompatibilityAssessment {
        state,
        preflight: Some(preflight),
    }
}

/// Gate state together with the server evidence from which it was derived.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompatibilityAssessment {
    /// Exactly one of the four player-visible states.
    pub state: CompatibilityState,
    /// Rotation preflight, absent precisely when the server published no usable rotation.
    pub preflight: Option<Report>,
}

/// A validated `fs_game` mod-directory value. Empty means the selected profile's base game.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FsGame(String);

impl FsGame {
    /// Validate the same single-directory constraint enforced by the engine.
    ///
    /// # Errors
    ///
    /// Rejects current/parent directory names, separators, control characters, and cvar command
    /// delimiters.
    pub fn new(value: impl Into<String>) -> Result<Self, LaunchError> {
        let value = value.into();
        // files.cpp:3304 (FS_InvalidGameDir) rejects traversal and subdirectories. The extra
        // control/quote/semicolon checks keep an untrusted server value from becoming commands.
        if value == "."
            || value == ".."
            || value.contains('/')
            || value.contains('\\')
            || value
                .chars()
                .any(|character| character.is_control() || matches!(character, '"' | ';' | '+'))
        {
            return Err(LaunchError::InvalidFsGame(value));
        }
        Ok(Self(value))
    }

    /// Return the validated wire/config value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A launch profile selects the engine's game family and its base data directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LaunchProfile {
    /// Game family selected for the client.
    pub target: TargetGame,
}

impl LaunchProfile {
    /// Construct a launch profile for a master-list family.
    #[must_use]
    pub const fn new(target: TargetGame) -> Self {
        Self { target }
    }

    /// Value consumed by `OpenMoHAA`'s `com_target_game` cvar.
    #[must_use]
    pub const fn target_game_id(self) -> u8 {
        // common.c:3151-3214 maps 0/1/2 to AA/Spearhead/Breakthrough and selects the matching
        // main/mainta/maintt base directory and network protocol.
        match self.target {
            TargetGame::AlliedAssault => 0,
            TargetGame::Spearhead => 1,
            TargetGame::Breakthrough => 2,
        }
    }

    /// Base data directory selected by the profile.
    #[must_use]
    pub const fn data_directory(self) -> &'static str {
        match self.target {
            TargetGame::AlliedAssault => "main",
            TargetGame::Spearhead => "mainta",
            TargetGame::Breakthrough => "maintt",
        }
    }
}

/// A process-neutral command description. M4 constructs this but never launches it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LaunchCommand {
    /// Client path or program name selected by the caller/profile layer.
    pub program: String,
    /// Typed game profile.
    pub profile: LaunchProfile,
    /// Validated optional mod directory.
    pub fs_game: FsGame,
    /// Authoritative game address.
    pub server: SocketAddrV4,
    /// `OpenMoHAA` argument vector retained for serialization and display.
    pub arguments: Vec<String>,
}

/// Engine command-line dialect selected by the platform layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchDialect {
    /// One `OpenMoHAA` executable selects AA/SH/BT through `com_target_game`.
    OpenMohaa,
    /// Retail selects AA/SH/BT by executable and does not define `com_target_game`.
    Retail,
}

impl LaunchCommand {
    /// Build OpenMoHAA-compatible arguments without starting a process.
    ///
    /// # Errors
    ///
    /// Returns an error when the program is empty.
    pub fn new(
        program: impl Into<String>,
        profile: LaunchProfile,
        fs_game: FsGame,
        server: SocketAddrV4,
    ) -> Result<Self, LaunchError> {
        let program = program.into();
        if program.is_empty() {
            return Err(LaunchError::EmptyProgram);
        }
        let mut command = Self {
            program,
            profile,
            fs_game,
            server,
            arguments: Vec::new(),
        };
        command.arguments = command.arguments_for(LaunchDialect::OpenMohaa);
        Ok(command)
    }

    /// Return the argument vector understood by the selected engine.
    #[must_use]
    pub fn arguments_for(&self, dialect: LaunchDialect) -> Vec<String> {
        match dialect {
            LaunchDialect::OpenMohaa => vec![
                "+set".to_owned(),
                "com_target_game".to_owned(),
                self.profile.target_game_id().to_string(),
                "+set".to_owned(),
                "fs_game".to_owned(),
                self.fs_game.as_str().to_owned(),
                "+connect".to_owned(),
                self.server.to_string(),
            ],
            // Retail binaries select the product by executable and do not define
            // OpenMoHAA's `com_target_game` cvar.
            LaunchDialect::Retail => vec![
                "+set".to_owned(),
                "fs_game".to_owned(),
                self.fs_game.as_str().to_owned(),
                "+connect".to_owned(),
                self.server.to_string(),
            ],
        }
    }
}

/// Invalid launch-command input.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LaunchError {
    /// The platform/profile layer did not select a client program.
    #[error("client program is empty")]
    EmptyProgram,
    /// An unsafe or non-directory `fs_game` value was supplied.
    #[error("invalid fs_game value {0:?}")]
    InvalidFsGame(String),
}

/// Stable category for a rejection observed after an attempted join.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionKind {
    Banned,
    RequiresBreakthrough,
    WrongProtocol,
    UserinfoTooLong,
    ServerFull,
    ServerRejected,
    Kicked,
    BadChallenge,
    PingRestricted,
    CdKeyAuthorization,
}

/// Player-facing explanation of a server response that has already happened.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RejectionExplanation {
    /// Stable programmatic category.
    pub kind: RejectionKind,
    /// Plain-language copy for the player.
    pub message: String,
}

/// Translate a post-connect server rejection. This is deliberately not an input to [`classify`].
///
/// v1 has no caller because it does not tail client logs; this remains tested v2 groundwork.
#[must_use]
pub fn explain_rejection(raw: &str) -> Option<RejectionExplanation> {
    let text = raw.trim_matches(['\0', '\r', '\n']);

    // sv_client.c:393 and :396.
    if let Some(position) = text.find("You are banned from this server.") {
        let tail = &text[position + "You are banned from this server.".len()..];
        let reason = tail
            .trim()
            .strip_prefix("Reason:")
            .map(str::trim)
            .filter(|reason| !reason.is_empty());
        return Some(RejectionExplanation {
            kind: RejectionKind::Banned,
            message: reason.map_or_else(
                || "You're banned from this server.".to_owned(),
                |reason| format!("You're banned from this server. Reason: {reason}"),
            ),
        });
    }
    // sv_client.c:409.
    if text.contains("Requires Medal of Honor Allied Assault Breakthrough") {
        return Some(explanation(
            RejectionKind::RequiresBreakthrough,
            "This server requires the Breakthrough profile.",
        ));
    }
    // sv_client.c:436.
    if let Some(tail) = text.split("Server uses protocol version ").nth(1) {
        let version = tail
            .split(|character: char| !character.is_ascii_digit())
            .next()
            .filter(|version| !version.is_empty())
            .unwrap_or("unknown");
        return Some(explanation(
            RejectionKind::WrongProtocol,
            &format!("Version mismatch: the server uses protocol {version}. Switch profile."),
        ));
    }
    // sv_client.c:470.
    if text.contains("Userinfo string length exceeded") {
        return Some(explanation(
            RejectionKind::UserinfoTooLong,
            "Your game configuration sends too much user information. Remove setu cvars and try again.",
        ));
    }
    // sv_client.c:600.
    if text.contains("Server is full") {
        return Some(explanation(
            RejectionKind::ServerFull,
            "The server is full.",
        ));
    }
    // sv_client.c:2214 and :2223. Check before the generic game rejection at :639.
    if let Some(position) = text.find("Kicked from server") {
        let tail = &text[position + "Kicked from server".len()..];
        let reason = tail
            .trim()
            .strip_prefix("for:")
            .map(str::trim)
            .filter(|reason| !reason.is_empty());
        return Some(RejectionExplanation {
            kind: RejectionKind::Kicked,
            message: reason.map_or_else(
                || "You were kicked from this server.".to_owned(),
                |reason| format!("You were kicked from this server. Reason: {reason}"),
            ),
        });
    }
    // sv_client.c:494.
    if text.contains("No or bad challenge for your address") {
        return Some(explanation(
            RejectionKind::BadChallenge,
            "The server refused the handshake. Try again.",
        ));
    }
    // sv_client.c:511 and :517.
    if text.contains("Server is for high pings only") {
        return Some(explanation(
            RejectionKind::PingRestricted,
            "This server only accepts high-ping clients.",
        ));
    }
    if text.contains("Server is for low pings only") {
        return Some(explanation(
            RejectionKind::PingRestricted,
            "Your ping is above this server's limit.",
        ));
    }
    // cl_main.cpp emits this while a post-connect CD-key decision is pending.
    if text.contains("Awaiting CD key authorization") {
        return Some(explanation(
            RejectionKind::CdKeyAuthorization,
            "The server is waiting for CD-key authorisation.",
        ));
    }
    // sv_client.c:639 carries arbitrary game-module denial copy.
    text.split_once("droperror\n").and_then(|(_, detail)| {
        let detail = detail.trim();
        (!detail.is_empty()).then(|| RejectionExplanation {
            kind: RejectionKind::ServerRejected,
            message: format!("The server rejected the connection: {detail}"),
        })
    })
}

fn explanation(kind: RejectionKind, message: &str) -> RejectionExplanation {
    RejectionExplanation {
        kind,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use super::{CompatibilityState, classify_server};
    use super::{
        FsGame, LaunchCommand, LaunchDialect, LaunchProfile, RejectionKind, explain_rejection,
    };
    use crate::discovery::{
        GamePort, MasterEndpoint, QueryPort, ReportedOccupancy, Server, TargetGame,
    };
    use crate::mapindex::MapIndex;

    #[test]
    fn constructs_target_profile_mod_and_connect_arguments() {
        let command = LaunchCommand::new(
            "openmohaa",
            LaunchProfile::new(TargetGame::Spearhead),
            FsGame::new("reborn").expect("valid mod directory"),
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 23_900),
        )
        .expect("launch command");

        assert_eq!(command.profile.data_directory(), "mainta");
        assert_eq!(
            command.arguments,
            command.arguments_for(LaunchDialect::OpenMohaa)
        );
        assert_eq!(
            command.arguments,
            [
                "+set",
                "com_target_game",
                "1",
                "+set",
                "fs_game",
                "reborn",
                "+connect",
                "203.0.113.7:23900",
            ]
        );
    }

    #[test]
    fn retail_dialect_omits_openmohaa_profile_cvar() {
        let command = LaunchCommand::new(
            "MOHAA.exe",
            LaunchProfile::new(TargetGame::AlliedAssault),
            FsGame::new("").expect("base game"),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 12_203),
        )
        .expect("launch command");

        assert_eq!(
            command.arguments_for(LaunchDialect::Retail),
            ["+set", "fs_game", "", "+connect", "127.0.0.1:12203"]
        );
    }

    #[test]
    fn rejects_unsafe_server_published_mod_directories() {
        for value in [
            ".",
            "..",
            "../main",
            "mods\\evil",
            "mod;quit",
            "mod+quit",
            "mod\nquit",
        ] {
            assert!(FsGame::new(value).is_err());
        }
        assert_eq!(FsGame::new("").expect("base game").as_str(), "");
    }

    #[test]
    fn rejection_copy_covers_all_nine_direct_connect_and_kick_sites() {
        let cases = [
            (
                "droperror\nYou are banned from this server.\nReason: griefing\n",
                RejectionKind::Banned,
                "You're banned from this server. Reason: griefing",
            ),
            (
                "droperror\nYou are banned from this server.\n",
                RejectionKind::Banned,
                "You're banned from this server.",
            ),
            (
                "droperror\nRequires Medal of Honor Allied Assault Breakthrough\n",
                RejectionKind::RequiresBreakthrough,
                "This server requires the Breakthrough profile.",
            ),
            (
                "droperror\nServer uses protocol version 17 (yours is 8).\n",
                RejectionKind::WrongProtocol,
                "Version mismatch: the server uses protocol 17. Switch profile.",
            ),
            (
                "droperror\nUserinfo string length exceeded. Try removing setu cvars from your config.\n",
                RejectionKind::UserinfoTooLong,
                "Your game configuration sends too much user information. Remove setu cvars and try again.",
            ),
            (
                "droperror\nServer is full\n",
                RejectionKind::ServerFull,
                "The server is full.",
            ),
            (
                "droperror\nInvalid password\n",
                RejectionKind::ServerRejected,
                "The server rejected the connection: Invalid password",
            ),
            (
                "droperror\nKicked from server for:\nteam killing",
                RejectionKind::Kicked,
                "You were kicked from this server. Reason: team killing",
            ),
            (
                "droperror\nKicked from server",
                RejectionKind::Kicked,
                "You were kicked from this server.",
            ),
        ];

        for (raw, expected_kind, expected_message) in cases {
            let explanation = explain_rejection(raw).expect("known rejection");
            assert_eq!(explanation.kind, expected_kind);
            assert_eq!(explanation.message, expected_message);
        }
    }

    #[test]
    fn discovered_server_without_rotation_is_cant_tell_not_compatible() {
        let index = MapIndex::default();
        let no_rotation = server(Vec::new());
        let published_rotation = server(vec!["obj/missing".to_owned()]);

        assert_eq!(
            classify_server(&index, &no_rotation, None).state,
            CompatibilityState::CantTell
        );
        assert!(matches!(
            classify_server(&index, &published_rotation, None).state,
            CompatibilityState::NeedsMaps { count, .. } if count.get() == 1
        ));
    }

    fn server(rotation: Vec<String>) -> Server {
        Server {
            endpoint: MasterEndpoint {
                address: Ipv4Addr::LOCALHOST,
                query_port: QueryPort::new(12_300),
            },
            game_port: GamePort::new(12_203),
            hostname: "fixture".to_owned(),
            game_name: Some("mohaa".to_owned()),
            game_version: None,
            version: None,
            protocol: Some("8".to_owned()),
            current_map: None,
            rotation,
            allow_download: None,
            map_checksum: None,
            pr_downloads: None,
            minimum_ping: None,
            maximum_ping: None,
            join_window: None,
            reserved_slots: None,
            occupancy: ReportedOccupancy::default(),
            client_capacity: None,
            pure: None,
        }
    }
}
