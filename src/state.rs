// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands;
use crate::kill_switch::KillSwitchState;

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
    /// Phase G — emergency kill switch state. `active = false` is the resting
    /// shape; `proteus kill` flips it on, `proteus resume` flips it off.
    /// Skip-serialised when inactive so a cold install does not grow the
    /// state file with a useless object.
    #[serde(skip_serializing_if = "kill_switch_inactive")]
    pub kill_switch: KillSwitchState,
    // Phase C: captive portal state.
    pub known_portal_ssids: Vec<String>,
    pub last_portal_check: Option<PortalCheckRecord>,
}

fn kill_switch_inactive(k: &KillSwitchState) -> bool {
    !k.active && k.interfaces.is_empty() && k.activated_at.is_none()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PortalCheckRecord {
    pub timestamp: String,
    pub classification: String,
    pub ssid: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Originals {
    pub bluetooth_aliases: BTreeMap<String, String>,
    /// First-apply snapshot of all three hostnamed-tracked fields. `None`
    /// means hostname has never been applied on this system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<HostnameOriginals>,
    /// First-apply snapshot of per-iface IPv6 sysctl values. Keyed by
    /// interface name.
    pub ipv6: BTreeMap<String, Ipv6Originals>,
    /// First-apply snapshot of per-NM-connection settings Proteus mutates
    /// (802.1X anonymous-identity, DHCP settings). Keyed by NM `connection.uuid`
    /// (issue #124 — `id` isn't unique, two profiles can share a name).
    pub connections: BTreeMap<String, ConnectionOriginals>,
    /// Cached sysctl values keyed by full sysctl name (e.g.
    /// `net.ipv4.tcp_timestamps`). Populated on `proteus stack apply` before
    /// any write, never overwritten on subsequent applies. Empty string means
    /// "key did not exist on this kernel".
    pub sysctls: BTreeMap<String, String>,
    /// First-apply snapshot of per-Wi-Fi-iface TX power. Keyed by interface
    /// name. Captured the first time `proteus rf apply` writes a new TX
    /// power and used by `proteus rf revert` to restore the original. Empty
    /// when no RF apply has run; skip-serialized to keep state.json compact.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub rf: BTreeMap<String, RfOriginals>,
}

/// Cached pre-Proteus values for the per-connection settings Proteus can
/// rewrite (802.1X anonymous-identity, DHCP options). Captured on first
/// touch, never re-captured.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionOriginals {
    /// Original value of `802-1x.anonymous-identity`. `None` means the key
    /// was unset before Proteus's first enable on this connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anonymous_identity: Option<String>,
    /// Original DHCP settings before Proteus's first apply on this connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dhcp_settings: Option<DhcpSettingsSnapshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DhcpSettingsSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_dhcp_send_hostname: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_dhcp_hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_dhcp_fqdn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_dhcp_vendor_class_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_dhcp_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6_dhcp_duid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6_dhcp_iaid: Option<String>,
}

/// Cached pre-Proteus values for the IPv6 sysctls Proteus manages on a
/// given interface. Captured on the first apply and never re-captured;
/// `revert` writes these back. All fields are stored as the raw integer
/// strings the kernel uses for the corresponding `/proc/sys/net/ipv6/conf/*`
/// node so the on-disk format is forward-compatible if the kernel grows
/// new modes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ipv6Originals {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_tempaddr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr_gen_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_valid_lft: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_prefered_lft: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HostnameOriginals {
    pub kernel: Option<String>,
    pub pretty: Option<String>,
    pub transient: Option<String>,
}

/// Cached pre-Proteus TX power for one Wi-Fi interface. `None` means the
/// `iw` lookup did not return a value at first-apply time (driver doesn't
/// expose it, link was down, etc.); revert in that case is a no-op for
/// the iface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RfOriginals {
    /// TX power in mBm (milli-dBm; the unit `iw` reports natively).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_power_mbm: Option<i32>,
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
        let mut state: State = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing state file {}", path.display()))?;
        state.migrate_connection_keys();
        Ok(Some(state))
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        Ok(Self::load(path)?.unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        commands::write_atomic(path, &bytes)
    }

    /// Issue #124 migration: drop `originals.connections` and
    /// `managed.connections` entries that aren't keyed by an NM uuid.
    /// Pre-fix versions of Proteus keyed these maps by `connection.id`, which
    /// isn't unique. We can't translate id -> uuid offline (it's an NM
    /// runtime mapping), so we drop the offenders and re-capture on the next
    /// apply. The "originals are sacred" rule means losing data is bad — we
    /// log loudly so the operator notices.
    fn migrate_connection_keys(&mut self) {
        drop_non_uuid_keys(
            &mut self.originals.connections,
            "originals.connections",
            "re-capture happens on next apply",
        );
        drop_non_uuid_keys(
            &mut self.managed.connections,
            "managed.connections",
            "pin/rotation history will be re-built on next rotate",
        );
    }
}

fn drop_non_uuid_keys<V>(map: &mut BTreeMap<String, V>, label: &str, recovery_hint: &str) {
    let bad: Vec<String> = map
        .keys()
        .filter(|k| !looks_like_uuid(k))
        .cloned()
        .collect();
    if bad.is_empty() {
        return;
    }
    tracing::warn!(
        target: "proteus::state",
        "state migration (issue #124): dropping {} {label} entr{} keyed by id instead of uuid; \
         {recovery_hint}: {:?}",
        bad.len(),
        if bad.len() == 1 { "y" } else { "ies" },
        bad,
    );
    for k in &bad {
        map.remove(k);
    }
}

/// Loose check for the canonical NM uuid shape (8-4-4-4-12 hex with hyphens).
/// We accept any case so manual edits round-trip. Non-uuid values flag id-keyed
/// state entries from before the issue #124 migration. Shared with command
/// modules that accept either uuid or id as a CLI target.
pub fn looks_like_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if *b != b'-' {
                    return false;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
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

    #[test]
    fn looks_like_uuid_accepts_canonical_form() {
        assert!(looks_like_uuid("12345678-1234-1234-1234-123456789abc"));
        // Mixed case is fine.
        assert!(looks_like_uuid("12345678-1234-1234-1234-123456789ABC"));
    }

    #[test]
    fn looks_like_uuid_rejects_short_or_non_hex() {
        // Connection-id-shaped strings with no hyphens.
        assert!(!looks_like_uuid("MyHomeWiFi"));
        assert!(!looks_like_uuid("Office Wi-Fi"));
        // Right length but missing hyphens.
        assert!(!looks_like_uuid("123456781234123412341234567890ab"));
        // Non-hex character in a hex slot.
        assert!(!looks_like_uuid("zzzzzzzz-1234-1234-1234-123456789abc"));
        // Empty.
        assert!(!looks_like_uuid(""));
    }

    #[test]
    fn migrate_drops_id_keyed_originals_connections() {
        // Pre-fix state used connection.id (human name) as the key. The fix
        // is uuid-only — old entries are dropped on load with a loud warning
        // (issue #124). State migration must not silently lose data: the
        // entry must be gone from the live map afterwards.
        let mut s = State::default();
        s.originals.connections.insert(
            "MyHomeWiFi".to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("anonymous@example.edu".to_string()),
                dhcp_settings: None,
            },
        );
        // Sanity-check: the entry's there before migration runs.
        assert!(s.originals.connections.contains_key("MyHomeWiFi"));
        s.migrate_connection_keys();
        assert!(
            !s.originals.connections.contains_key("MyHomeWiFi"),
            "migration must drop id-keyed entries"
        );
    }

    #[test]
    fn migrate_keeps_uuid_keyed_originals_connections() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut s = State::default();
        s.originals.connections.insert(
            uuid.to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("anonymous@example.edu".to_string()),
                dhcp_settings: None,
            },
        );
        s.migrate_connection_keys();
        assert!(
            s.originals.connections.contains_key(uuid),
            "uuid-keyed entries must survive migration"
        );
    }

    #[test]
    fn two_uuids_with_same_id_stay_under_separate_keys() {
        // Issue #124 cornerstone: two NM connection profiles can share a
        // connection.id (e.g. cloned profile, two SSIDs hand-named the same
        // way) but ALWAYS have distinct uuids. Verify our keying lets both
        // coexist; the pre-fix id-keyed code collapsed them into one.
        let uuid_a = "11111111-1111-1111-1111-111111111111";
        let uuid_b = "22222222-2222-2222-2222-222222222222";
        let mut s = State::default();
        s.originals.connections.insert(
            uuid_a.to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("user-a@a.example".to_string()),
                dhcp_settings: None,
            },
        );
        s.originals.connections.insert(
            uuid_b.to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("user-b@b.example".to_string()),
                dhcp_settings: None,
            },
        );
        s.migrate_connection_keys();
        assert_eq!(s.originals.connections.len(), 2, "both uuids must persist");
        assert_eq!(
            s.originals.connections[uuid_a]
                .anonymous_identity
                .as_deref(),
            Some("user-a@a.example")
        );
        assert_eq!(
            s.originals.connections[uuid_b]
                .anonymous_identity
                .as_deref(),
            Some("user-b@b.example")
        );
    }

    #[test]
    fn migrate_drops_id_keyed_managed_connections() {
        let mut s = State::default();
        s.managed.connections.insert(
            "OfficeWiFi".to_string(),
            ConnectionRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                ..Default::default()
            },
        );
        s.migrate_connection_keys();
        assert!(s.managed.connections.is_empty());
    }
}
