// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub original_macs: BTreeMap<String, String>,
    pub original_hostname: Option<String>,
    pub captured_by_version: Option<String>,
    pub captured_at: Option<String>,
    // Phase B+ fields. `#[serde(default)]` keeps older state.json files loading.
    pub managed: ManagedState,
    pub originals: Originals,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Originals {
    pub bluetooth_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ManagedState {
    pub interfaces: BTreeMap<String, InterfaceRecord>,
    pub connections: BTreeMap<String, ConnectionRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceRecord {
    pub current_mac: Option<String>,
    pub pinned: Option<String>,
    pub last_rotated: Option<String>,
    pub rotation_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionRecord {
    pub current_mac: Option<String>,
    pub pinned: Option<String>,
    pub last_rotated: Option<String>,
    pub rotation_count: u64,
}

impl State {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("reading state file {}", path.display()));
            }
        };
        let state: State = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing state file {}", path.display()))?;
        Ok(Some(state))
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        Ok(Self::load(path)?.unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        commands::write_atomic(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_managed_section() {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".to_string(),
            InterfaceRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                pinned: None,
                last_rotated: Some("2026-05-06T00:00:00Z".to_string()),
                rotation_count: 3,
            },
        );
        let bytes = serde_json::to_vec(&s).unwrap();
        let back: State = serde_json::from_slice(&bytes).unwrap();
        let rec = back.managed.interfaces.get("wlan0").unwrap();
        assert_eq!(rec.current_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(rec.rotation_count, 3);
    }

    #[test]
    fn old_state_files_load() {
        // No `managed` field at all — must still parse.
        let json = r#"{"original_macs":{"wlan0":"aa:bb:cc:dd:ee:ff"}}"#;
        let s: State = serde_json::from_str(json).unwrap();
        assert_eq!(
            s.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff")
        );
        assert!(s.managed.interfaces.is_empty());
    }
}
