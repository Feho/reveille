// SPDX-License-Identifier: GPL-2.0-only

//! Registry installation evidence.
//!
//! The parsers and discovery functions remain platform-neutral. On Windows, [`read_live_hives`]
//! translates string values from the live registry into the same [`RegistryKey`] shape.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

use crate::install::{self, Installation};

/// One registry key with string values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RegistryKey {
    /// Full key path without surrounding brackets.
    pub path: String,
    /// Named string values. Non-string data is deliberately ignored.
    pub values: BTreeMap<String, String>,
}

impl RegistryKey {
    fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()))
    }
}

/// Store metadata that yielded an EA-family install root.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EaInstallSource {
    /// `...CurrentVersion\Uninstall` entry used by EA App-managed installs.
    EaAppUninstall,
    /// Classic `EA Games\<title>` entry used by Origin-era installs.
    OriginEaGames,
}

/// A candidate root. Its title is discovery evidence, never expansion ownership evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EaInstallRoot {
    /// Registry layout that supplied the path.
    pub source: EaInstallSource,
    /// Display/edition name retained only for diagnostics.
    pub display_name: String,
    /// Candidate game installation root.
    pub root: PathBuf,
}

/// Identified candidate paired with its store evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IdentifiedEaInstallation {
    /// Store evidence.
    pub discovery: EaInstallRoot,
    /// Filesystem evidence from [`install::identify`].
    pub installation: Installation,
}

/// Candidate that did not identify as an installed game.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkippedEaInstallation {
    /// Candidate store evidence.
    pub discovery: EaInstallRoot,
    /// Identification failure retained as data.
    pub reason: String,
}

/// Partial identification across EA App and Origin roots.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EaDiscovery {
    /// Valid installations.
    pub installations: Vec<IdentifiedEaInstallation>,
    /// Stale or unusable registry roots.
    pub skipped: Vec<SkippedEaInstallation>,
}

/// Parse a UTF-8 `.reg` export containing ordinary quoted string values.
///
/// # Errors
///
/// Returns an error for a value outside a key, an unterminated key, or malformed quoted strings.
pub fn parse_registry_export(text: &str) -> Result<Vec<RegistryKey>, RegistryError> {
    let mut keys = Vec::new();
    let mut current: Option<RegistryKey> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.is_empty()
            || line.starts_with(';')
            || line.eq_ignore_ascii_case("Windows Registry Editor Version 5.00")
            || line.eq_ignore_ascii_case("REGEDIT4")
        {
            continue;
        }
        if let Some(path) = line.strip_prefix('[') {
            let path = path
                .strip_suffix(']')
                .ok_or(RegistryError::MalformedLine { line: line_number })?;
            if let Some(previous) = current.replace(RegistryKey {
                path: path.to_owned(),
                values: BTreeMap::new(),
            }) {
                keys.push(previous);
            }
            continue;
        }
        let Some(key) = current.as_mut() else {
            return Err(RegistryError::ValueOutsideKey { line: line_number });
        };
        if !line.starts_with('"') {
            continue;
        }
        let (name, rest) = parse_registry_quoted(line, line_number)?;
        let Some(value_text) = rest.trim_start().strip_prefix('=') else {
            return Err(RegistryError::MalformedLine { line: line_number });
        };
        let value_text = value_text.trim_start();
        if value_text.starts_with('"') {
            let (value, trailing) = parse_registry_quoted(value_text, line_number)?;
            if !trailing.trim().is_empty() {
                return Err(RegistryError::MalformedLine { line: line_number });
            }
            key.values.insert(name, value);
        }
    }
    if let Some(last) = current {
        keys.push(last);
    }
    Ok(keys)
}

/// Extract MOHAA-like roots from EA App uninstall entries and classic Origin `EA Games` keys.
#[must_use]
pub fn discover_ea_install_roots(keys: &[RegistryKey]) -> Vec<EaInstallRoot> {
    keys.iter()
        .filter_map(|key| {
            let path = key.path.to_ascii_lowercase();
            // Microsoft Windows CurrentVersion\Uninstall application-registration layout.
            let uninstall = path.contains("\\currentversion\\uninstall\\");
            // Origin-era Electronic Arts game keys expose an `Install Dir` string.
            let origin = path.contains("\\ea games\\");
            let display_name = key
                .value("DisplayName")
                .or_else(|| key.path.rsplit('\\').next())?;
            if !looks_like_mohaa(display_name) {
                return None;
            }
            let (source, root) = if uninstall {
                (
                    EaInstallSource::EaAppUninstall,
                    key.value("InstallLocation")?,
                )
            } else if origin {
                (EaInstallSource::OriginEaGames, key.value("Install Dir")?)
            } else {
                return None;
            };
            (!root.trim().is_empty()).then(|| EaInstallRoot {
                source,
                display_name: display_name.to_owned(),
                root: PathBuf::from(root.trim_end_matches(['\\', '/'])),
            })
        })
        .collect()
}

/// Extract the War Chest root from the GOG registry key used by the engine.
#[must_use]
pub fn discover_gog_install_root(keys: &[RegistryKey]) -> Option<PathBuf> {
    keys.iter().find_map(|key| {
        key.path
            .eq_ignore_ascii_case("HKEY_LOCAL_MACHINE\\SOFTWARE\\GOG.com\\Games\\1441704920")
            .then(|| key.value("PATH"))
            .flatten()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path.trim_end_matches(['\\', '/'])))
    })
}

/// Read relevant installation keys from the live Windows registry.
///
/// Both registry views are enumerated explicitly. The GOG key is read from the 32-bit view, as
/// `OpenMoHAA` does in `Sys_GogPath` (`code/sys/sys_win32.c:191-219`). Missing roots and entries are
/// normal and produce no key.
///
/// # Errors
///
/// Returns an error when a present registry root or one of its children cannot be read.
#[cfg(windows)]
pub fn read_live_hives() -> Result<Vec<RegistryKey>, LiveRegistryError> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    const UNINSTALL: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    const ORIGIN: &str = "SOFTWARE\\Electronic Arts\\EA Games";
    const GOG: &str = "SOFTWARE\\GOG.com\\Games\\1441704920";

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut keys = Vec::new();
    for (hive, hive_name) in [(&hklm, "HKEY_LOCAL_MACHINE"), (&hkcu, "HKEY_CURRENT_USER")] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            read_children(hive, hive_name, UNINSTALL, KEY_READ | view, &mut keys)?;
            read_children(hive, hive_name, ORIGIN, KEY_READ | view, &mut keys)?;
        }
    }
    read_one(
        &hklm,
        "HKEY_LOCAL_MACHINE",
        GOG,
        KEY_READ | KEY_WOW64_32KEY,
        &mut keys,
    )?;
    Ok(keys)
}

#[cfg(windows)]
fn read_children(
    hive: &winreg::RegKey,
    hive_name: &str,
    path: &str,
    flags: u32,
    keys: &mut Vec<RegistryKey>,
) -> Result<(), LiveRegistryError> {
    use std::io::ErrorKind;

    let root = match hive.open_subkey_with_flags(path, flags) {
        Ok(root) => root,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(LiveRegistryError::Read {
                path: path.into(),
                source,
            });
        }
    };
    for child in root.enum_keys() {
        let child = child.map_err(|source| LiveRegistryError::Read {
            path: path.into(),
            source,
        })?;
        let child_path = format!("{path}\\{child}");
        read_one(hive, hive_name, &child_path, flags, keys)?;
    }
    Ok(())
}

#[cfg(windows)]
fn read_one(
    hive: &winreg::RegKey,
    hive_name: &str,
    path: &str,
    flags: u32,
    keys: &mut Vec<RegistryKey>,
) -> Result<(), LiveRegistryError> {
    use std::io::ErrorKind;
    use winreg::types::FromRegValue;

    let key = match hive.open_subkey_with_flags(path, flags) {
        Ok(key) => key,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(LiveRegistryError::Read {
                path: path.into(),
                source,
            });
        }
    };
    let mut values = BTreeMap::new();
    for value in key.enum_values() {
        let (name, raw) = value.map_err(|source| LiveRegistryError::Read {
            path: path.into(),
            source,
        })?;
        if let Ok(value) = String::from_reg_value(&raw) {
            values.insert(name, value);
        }
    }
    keys.push(RegistryKey {
        path: format!("{hive_name}\\{path}"),
        values,
    });
    Ok(())
}

/// Identify candidates by filesystem evidence, retaining stale registry entries as non-results.
///
/// Edition names are never passed as product evidence: only `main`, `mainta`, and `maintt`
/// directories observed by [`install::identify`] determine available games.
#[must_use]
pub fn identify_ea_installations(candidates: &[EaInstallRoot]) -> EaDiscovery {
    let mut discovery = EaDiscovery::default();
    for candidate in candidates {
        match install::identify(&candidate.root) {
            Ok(installation) => discovery.installations.push(IdentifiedEaInstallation {
                discovery: candidate.clone(),
                installation,
            }),
            Err(error) => discovery.skipped.push(SkippedEaInstallation {
                discovery: candidate.clone(),
                reason: error.to_string(),
            }),
        }
    }
    discovery
}

fn looks_like_mohaa(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("medal of honor")
        && (value.contains("allied assault") || value.contains("war chest"))
}

fn parse_registry_quoted(text: &str, line: usize) -> Result<(String, &str), RegistryError> {
    let mut value = String::new();
    let mut characters = text.char_indices();
    if characters.next().map(|(_, character)| character) != Some('"') {
        return Err(RegistryError::MalformedLine { line });
    }
    while let Some((position, character)) = characters.next() {
        match character {
            '"' => return Ok((value, &text[position + 1..])),
            '\\' => match characters.next() {
                Some((_, '\\')) => value.push('\\'),
                Some((_, '"')) => value.push('"'),
                Some((_, other)) => {
                    value.push('\\');
                    value.push(other);
                }
                None => return Err(RegistryError::MalformedLine { line }),
            },
            other => value.push(other),
        }
    }
    Err(RegistryError::MalformedLine { line })
}

/// Registry fixture parsing error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RegistryError {
    #[error("registry value appears outside a key on line {line}")]
    ValueOutsideKey { line: usize },
    #[error("malformed registry export on line {line}")]
    MalformedLine { line: usize },
}

/// Failure while translating a live registry hive into portable registry evidence.
#[cfg(windows)]
#[derive(Debug, Error)]
pub enum LiveRegistryError {
    /// A present key could not be opened or enumerated.
    #[error("could not read registry key {path}")]
    Read {
        /// Registry path that failed.
        path: String,
        /// Underlying Windows registry error.
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{
        EaInstallSource, RegistryKey, discover_ea_install_roots, discover_gog_install_root,
        identify_ea_installations, parse_registry_export,
    };
    use crate::install::Product;
    use std::collections::BTreeMap;

    #[test]
    fn parses_ea_app_uninstall_and_origin_install_dir_layouts() {
        let keys =
            parse_registry_export(include_str!("../../tests/fixtures/ea_origin_registry.reg"))
                .expect("registry export");
        let roots = discover_ea_install_roots(&keys);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].source, EaInstallSource::EaAppUninstall);
        assert_eq!(
            roots[0].root.to_string_lossy(),
            "D:\\EA Games\\Medal of Honor Allied Assault"
        );
        assert_eq!(roots[1].source, EaInstallSource::OriginEaGames);
        assert_eq!(
            roots[1].root.to_string_lossy(),
            "E:\\Origin Games\\Medal of Honor Allied Assault"
        );
    }

    #[test]
    fn war_chest_name_does_not_infer_missing_expansions() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path().join("War Chest");
        fs::create_dir_all(root.join("main")).expect("base data directory");
        let export = format!(
            "Windows Registry Editor Version 5.00\n\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MOHAA]\n\"DisplayName\"=\"Medal of Honor Allied Assault War Chest\"\n\"Publisher\"=\"Electronic Arts\"\n\"InstallLocation\"=\"{}\"\n",
            root.display()
        );
        let keys = parse_registry_export(&export).expect("registry export");
        let candidates = discover_ea_install_roots(&keys);
        let discovery = identify_ea_installations(&candidates);

        assert_eq!(discovery.installations.len(), 1);
        assert_eq!(
            discovery.installations[0].installation.products,
            vec![Product::AlliedAssault]
        );
        assert!(discovery.skipped.is_empty());
    }

    #[test]
    fn extracts_only_the_engine_gog_product_key() {
        let keys = vec![RegistryKey {
            path: "HKEY_LOCAL_MACHINE\\SOFTWARE\\GOG.com\\Games\\1441704920".into(),
            values: BTreeMap::from([("PATH".into(), "C:\\GOG Games\\MOHAA\\".into())]),
        }];

        assert_eq!(
            discover_gog_install_root(&keys),
            Some(PathBuf::from("C:\\GOG Games\\MOHAA"))
        );
    }
}
