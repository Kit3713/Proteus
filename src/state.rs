// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands;
use crate::kill_switch::KillSwitchState;

/// Issue #204: state.json schema version. Bumped when a structural change
/// requires a migration (not a backwards-compatible additive field). The
/// load path runs the migration ladder before handing the state out so old
/// files keep working without operator intervention.
///
/// Ladder so far:
///   0 — implicit pre-versioning (anything missing this field is treated as 0)
///   1 — `state.originals.connections` and `state.managed.connections` keyed
///       by NM `connection.uuid` instead of display id (issue #124).
///       Migration drops legacy entries (already implemented in
///       `migrate_connection_keys_to_uuid`).
///   2 — Roadmap Milestone 3: `state.known_portal_ssids` is mirrored into
///       a new `state.per_ssid_seed` map (each entry stamped with
///       `portal_policy = "fresh-mac-per-visit"`) so the orchestrator can
///       pick up SSID-scoped policy without re-reading the legacy field.
///       The legacy `known_portal_ssids` array is **kept** for one cycle
///       so older `proteus portal list / mark / unmark` paths keep
///       working; deprecation lands in a follow-up.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Issue #204: schema version. Defaults to 0 for state files written
    /// before this field landed; the migration ladder in `migrate_state`
    /// upgrades them in place on load. Anything written by a current build
    /// stamps this with `CURRENT_SCHEMA_VERSION`.
    #[serde(default)]
    pub schema_version: u32,
    /// Burned-in (factory) MAC address per interface, captured the first time
    /// Proteus rotates that iface and never re-captured. The value MUST be
    /// the permanent driver-reported address — NOT whatever the kernel
    /// currently shows at `/sys/class/net/<iface>/address`, which after a
    /// prior rotation is the cloned value. See `mac::factory` for the
    /// resolution order: `phy80211/macaddress` (Wi-Fi), `ethtool -P`
    /// (ethernet), then live `address` only when `addr_assign_type` reports
    /// `NET_ADDR_PERM`. Used by `proteus revert` to restore originals — a
    /// wrong value here turns "revert" into "set to last cloned".
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
    /// Roadmap Milestone 3: state-side mirror of per-SSID policies the
    /// orchestrator carried over from earlier releases. Today this is
    /// only populated by the v1 → v2 migration (every legacy
    /// `known_portal_ssids` entry lands here with `portal_policy =
    /// "fresh-mac-per-visit"`). The runtime authority for per-SSID
    /// policy is `Config::per_ssid` in `/etc/proteus/config.toml`; this
    /// state map exists only as a migration breadcrumb so a fresh
    /// install never grows it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_ssid_seed: BTreeMap<String, PerSsidStateSeed>,
}

/// One entry in `state.per_ssid_seed`. Mirrors the public
/// `PerSsidPolicy` shape so the migration step stays a straight copy:
/// every field is `Option<String>` and missing values stay missing on
/// disk via `skip_serializing_if`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PerSsidStateSeed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggressiveness_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate_interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_policy: Option<String>,
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
    /// (802.1X anonymous-identity, DHCP settings). Keyed by connection id.
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
    /// Load state from disk.
    ///
    /// `Ok(None)` means the file does not exist (cold install).
    ///
    /// Issue #127: a malformed state.json must not brick read-only commands
    /// (`status`, `current`, `original`, `diff`, ...). When parsing fails we
    /// quarantine the bad file as `<path>.corrupt-<utc-stamp>` and return
    /// `Ok(None)`. The next mutating apply re-captures originals and writes a
    /// fresh state.json, while read-only callers see an empty state and keep
    /// working.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("reading state file {}", path.display()));
            }
        };
        match serde_json::from_slice::<State>(&bytes) {
            Ok(mut state) => {
                migrate_state(&mut state);
                Ok(Some(state))
            }
            Err(e) => {
                let quarantine = quarantine_path(path);
                tracing::warn!(
                    "state.json parse failed ({e}); quarantining {} -> {}",
                    path.display(),
                    quarantine.display()
                );
                // Best-effort rename; if it fails the next apply will overwrite
                // via write_atomic, so we still degrade to an empty state.
                let _ = fs::rename(path, &quarantine);
                Ok(None)
            }
        }
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        Ok(Self::load(path)?.unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        // Always stamp the current schema version on write so we know what
        // shape the file is in next time `migrate_state` runs.
        let mut to_write = self.clone();
        to_write.schema_version = CURRENT_SCHEMA_VERSION;
        let bytes = serde_json::to_vec_pretty(&to_write)?;
        commands::write_atomic(path, &bytes)
    }
}

/// Issue #204: run every applicable migration step, in order, so a state
/// file written by an older Proteus arrives at the current schema before
/// callers see it. Each step is idempotent — running the ladder against an
/// already-current state is a no-op.
fn migrate_state(state: &mut State) {
    if state.schema_version < 1 {
        // v0 → v1: drop legacy id-keyed connection entries. Issue #124.
        migrate_connection_keys_to_uuid(state);
        state.schema_version = 1;
    }
    if state.schema_version < 2 {
        // v1 → v2 (roadmap Milestone 3): mirror `known_portal_ssids` into
        // `per_ssid_seed` so the new orchestrator path can pick up SSID-
        // scoped policy without consulting the legacy array. The legacy
        // field is kept untouched for one cycle so older portal commands
        // keep working.
        migrate_known_portals_to_per_ssid(state);
        state.schema_version = 2;
    }
}

/// Roadmap Milestone 3: each entry in `state.known_portal_ssids` becomes
/// a `state.per_ssid_seed[<ssid>]` with `portal_policy =
/// "fresh-mac-per-visit"`. Idempotent: existing per_ssid_seed entries
/// are left untouched (only fields that are still `None` get filled in)
/// so running the ladder twice converges instead of stomping the
/// operator's later edits. The legacy `known_portal_ssids` array is
/// **not** drained — see `CURRENT_SCHEMA_VERSION` doc for the
/// deprecation plan.
fn migrate_known_portals_to_per_ssid(state: &mut State) {
    for ssid in &state.known_portal_ssids {
        let entry = state.per_ssid_seed.entry(ssid.clone()).or_default();
        if entry.portal_policy.is_none() {
            entry.portal_policy = Some("fresh-mac-per-visit".to_string());
        }
    }
}

/// Issue #124: `state.originals.connections` and `state.managed.connections`
/// now key by NM `connection.uuid` instead of `connection.id`. Old state
/// files still in the wild (from <= v0.2.6-alpha) used `id`. Drop those
/// entries on load: they cannot be safely remapped without contacting NM,
/// and the next `proteus apply` re-captures originals correctly. Alpha
/// state is not durable across this kind of structural change.
fn migrate_connection_keys_to_uuid(state: &mut State) {
    state.originals.connections.retain(|k, _| is_uuid_shape(k));
    state.managed.connections.retain(|k, _| is_uuid_shape(k));
}

/// NM uuids are RFC-4122 — 36 chars: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
/// We don't need full RFC validation; shape-matching is enough to filter
/// legacy id-keyed entries from a v0.2.6 state.json.
fn is_uuid_shape(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        let want_dash = matches!(i, 8 | 13 | 18 | 23);
        if want_dash {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// `<path>.corrupt-<UTC-iso-with-colons-replaced>` so the bad bytes are
/// preserved for a postmortem and don't collide on a rapid retry. Colons are
/// stripped from the timestamp because some shells and recovery tools treat
/// them awkwardly in filenames.
fn quarantine_path(path: &Path) -> std::path::PathBuf {
    let stamp = commands::now_iso8601().replace(':', "-");
    let mut name = path
        .file_name()
        .map(|f| f.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("state.json"));
    name.push(format!(".corrupt-{stamp}"));
    path.with_file_name(name)
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

    /// Issue #204: a state file written before `schema_version` existed has
    /// `schema_version = 0` after parse. The migration ladder advances it to
    /// `CURRENT_SCHEMA_VERSION` and replays every applicable migration step.
    #[test]
    fn migration_ladder_advances_unversioned_state() {
        let json = r#"{"original_macs":{"wlan0":"aa:bb:cc:dd:ee:ff"}}"#;
        let mut s: State = serde_json::from_str(json).unwrap();
        assert_eq!(s.schema_version, 0, "unversioned files default to 0");
        migrate_state(&mut s);
        assert_eq!(s.schema_version, CURRENT_SCHEMA_VERSION);
    }

    /// Save round-trip stamps the current schema version onto the file
    /// even when the in-memory state has stale-version 0 (which can happen
    /// if a caller built `State::default()` and then mutated fields without
    /// going through `load`).
    #[test]
    fn save_stamps_current_schema_version() {
        let dir =
            std::env::temp_dir().join(format!("proteus-state-stamp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let s = State::default();
        assert_eq!(s.schema_version, 0);
        s.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(
            json.contains(&format!("\"schema_version\": {CURRENT_SCHEMA_VERSION}")),
            "save did not stamp schema_version: {json}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_quarantines_corrupt_state_file() {
        // Issue #127: a corrupt state.json (e.g. half-written from a crash)
        // must not brick read-only commands. `load` quarantines the file and
        // returns Ok(None) so callers can proceed with an empty state.
        let dir =
            std::env::temp_dir().join(format!("proteus-state-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, b"{\"original_macs\": this is not json").unwrap();

        let result = State::load(&path).expect("load returns Ok even on corrupt input");
        assert!(result.is_none(), "corrupt state must yield Ok(None)");
        assert!(!path.exists(), "corrupt file should be renamed away");

        let quarantines: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(
            quarantines.len(),
            1,
            "expected exactly one quarantined file, got {quarantines:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_none_for_missing_file() {
        let path = std::env::temp_dir().join("proteus-state-does-not-exist.json");
        let _ = fs::remove_file(&path);
        let result = State::load(&path).expect("missing path is Ok(None)");
        assert!(result.is_none());
    }

    /// Roadmap Milestone 3: a v1 state file with `known_portal_ssids`
    /// migrates the entries into `per_ssid_seed` with the fresh-MAC
    /// policy stamped in. The legacy array is kept (cycle's grace
    /// period).
    #[test]
    fn migration_v1_to_v2_seeds_per_ssid_from_known_portals() {
        let json = r#"{
            "schema_version": 1,
            "known_portal_ssids": ["foo", "bar"]
        }"#;
        let mut s: State = serde_json::from_str(json).unwrap();
        migrate_state(&mut s);
        assert_eq!(s.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(
            s.known_portal_ssids.iter().any(|x| x == "foo"),
            "legacy array must not be drained on migration"
        );
        let foo = s.per_ssid_seed.get("foo").expect("foo seeded");
        assert_eq!(foo.portal_policy.as_deref(), Some("fresh-mac-per-visit"));
        let bar = s.per_ssid_seed.get("bar").expect("bar seeded");
        assert_eq!(bar.portal_policy.as_deref(), Some("fresh-mac-per-visit"));
    }

    /// Migration is idempotent: a second run is a no-op (schema stays at
    /// `CURRENT_SCHEMA_VERSION` and existing seeds are not stomped).
    #[test]
    fn migration_v1_to_v2_is_idempotent() {
        let json = r#"{
            "schema_version": 1,
            "known_portal_ssids": ["foo"]
        }"#;
        let mut s: State = serde_json::from_str(json).unwrap();
        migrate_state(&mut s);
        let after_first = s.clone();
        migrate_state(&mut s);
        assert_eq!(s.schema_version, after_first.schema_version);
        assert_eq!(s.per_ssid_seed, after_first.per_ssid_seed);
    }

    /// Migration must not stomp an existing per_ssid_seed entry the
    /// operator may have authored manually with a non-default
    /// portal_policy. The migration only fills in fields that are still
    /// `None`.
    #[test]
    fn migration_preserves_existing_per_ssid_seed_entries() {
        let json = r#"{
            "schema_version": 1,
            "known_portal_ssids": ["foo"],
            "per_ssid_seed": {
                "foo": { "portal_policy": "rotate-before-auth" }
            }
        }"#;
        let mut s: State = serde_json::from_str(json).unwrap();
        migrate_state(&mut s);
        let foo = s.per_ssid_seed.get("foo").unwrap();
        assert_eq!(
            foo.portal_policy.as_deref(),
            Some("rotate-before-auth"),
            "existing entry must not be overwritten"
        );
    }

    /// State file written by the test in the task description loads
    /// cleanly through `State::load` and surfaces `foo` / `bar` in
    /// `per_ssid_seed`.
    #[test]
    fn load_runs_v1_to_v2_migration_end_to_end() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-v1v2-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(
            &path,
            br#"{"schema_version": 1, "known_portal_ssids": ["foo", "bar"]}"#,
        )
        .unwrap();

        let s = State::load(&path).expect("load ok").expect("present");
        assert_eq!(s.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(s.per_ssid_seed.contains_key("foo"));
        assert!(s.per_ssid_seed.contains_key("bar"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_default_yields_empty_on_corrupt_file() {
        // The mutating-command path goes through load_or_default; verify the
        // resilience hook reaches it so apply/rotate keep working even after
        // a state.json corruption.
        let dir =
            std::env::temp_dir().join(format!("proteus-state-default-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, b"\x00\x00not-json\x00").unwrap();

        let s = State::load_or_default(&path).expect("load_or_default never errors on corruption");
        assert!(s.original_macs.is_empty());
        assert!(s.managed.interfaces.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
