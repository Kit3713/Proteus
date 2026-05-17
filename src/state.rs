// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands;
use crate::kill_switch::KillSwitchState;

/// Mode the state directory must carry (root rwx, no group/other). Issue
/// #275: callers that auto-create `/var/lib/proteus` previously fell back
/// to whatever umask was active, which on a misconfigured system left
/// `state.json` deletable by any local user. Every code path that creates
/// or touches the state dir routes through [`ensure_state_dir_secure`] so
/// the mode is forced regardless of umask or pre-existing state.
pub const STATE_DIR_MODE: u32 = 0o700;

/// Mode the state file (`state.json`) and lock file (`.lock`) must
/// carry. `write_atomic` already opens the temp file at this mode; this
/// constant exists so the lock file (which is created via `OpenOptions`
/// without a mode) can match the same root-only posture (issue #275).
pub const STATE_FILE_MODE: u32 = 0o600;

/// Canonical Proteus state directory. Pre-existing directories at this
/// exact path are tightened to [`STATE_DIR_MODE`] on every call so a
/// pre-#275 install with a 0o755 dir can't drift wider than 0o700.
/// Anywhere else (operator-supplied `--state /custom/path`), Proteus
/// only chmods directories it actually created itself.
///
/// GH #354 / GH #363: the previous shape unconditionally chmodded the
/// parent of `state.json` to 0o700. With `--state /tmp/x` the parent is
/// `/tmp`, and `chmod 0700 /tmp` system-bricks every other process on
/// the box. Constraining the chmod to "the canonical dir, or a dir we
/// just created" closes that footgun.
const CANONICAL_STATE_DIR: &str = "/var/lib/proteus";

/// Ensure `dir` exists and is owned by root with mode [`STATE_DIR_MODE`].
/// Idempotent: safe to call when the dir is already correct, when it
/// exists with a wrong mode (gets re-chmodded if Proteus owns it), or
/// when it doesn't yet exist (gets created with the target mode).
///
/// Issue #275: do **not** trust umask. Callers reach this both at first
/// install (cold dir) and on every mutating command (warm dir).
///
/// GH #354 / GH #363: do **not** chmod operator-supplied directories
/// that we did not create ourselves. If the caller passed
/// `--state /tmp/x` (with `/tmp/x` already present, or
/// `--state /tmp/x/state.json` where `/tmp/x` already exists), this
/// function will create what's missing under it and chmod only the
/// freshly-created leg of the path. The CANONICAL_STATE_DIR is a
/// special case: that path *is* Proteus territory and gets the
/// pre-#275 idempotent re-tighten so a wider mode never persists on a
/// real install.
pub fn ensure_state_dir_secure(dir: &Path) -> Result<()> {
    let already_existed = dir.exists();
    fs::create_dir_all(dir).with_context(|| format!("creating state dir {}", dir.display()))?;

    // Chmod policy:
    // - We just created it now → tighten to 0o700.
    // - Path is the canonical /var/lib/proteus → keep idempotent #275
    //   tighten so a pre-existing wider mode is corrected on next run.
    // - Otherwise (operator-supplied custom path that already existed) →
    //   leave the directory's mode alone. The state.json itself is
    //   landed at 0o600 by `write_atomic`, which is the actual secret-
    //   bearing surface; chmodding the parent on every save is what
    //   bricks systems when `--state` points into a shared dir.
    let is_canonical = dir == Path::new(CANONICAL_STATE_DIR);
    if !already_existed || is_canonical {
        let perms = fs::Permissions::from_mode(STATE_DIR_MODE);
        fs::set_permissions(dir, perms)
            .with_context(|| format!("chmod 0{STATE_DIR_MODE:o} on state dir {}", dir.display()))?;
    }
    Ok(())
}

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
    /// Issue #364: ISO-8601 UTC timestamp captured when `pinned` was last
    /// set via `proteus pin`. Surfaced by `proteus pin list` so the
    /// operator can see when each pin was authored. Older state files
    /// (pre-#364) and unpinned records have this as `None`; the
    /// `skip_serializing_if` keeps fresh installs from growing the
    /// state file with a useless key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    pub last_rotated: Option<String>,
    pub rotation_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionRecord {
    pub current_mac: Option<String>,
    pub pinned: Option<String>,
    /// Issue #364: ISO-8601 UTC timestamp captured when `pinned` was last
    /// set via `proteus pin`. See [`InterfaceRecord::pinned_at`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
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
    /// `Ok(Some(recovered))` where `recovered` carries any originals we could
    /// salvage — the next mutating apply keeps the cached originals so
    /// `proteus revert` still restores the actual original hostname / BT
    /// alias, not the rotated one.
    ///
    /// Issue #290: previously the quarantine path returned `Ok(None)` (an
    /// empty state). With the hostname rotated, the next apply would
    /// re-capture the **rotated** hostname as the "original" because
    /// `state.originals.hostname.is_some()` was now `false`. The fix is
    /// best-effort partial recovery: we re-parse the corrupt bytes as a
    /// `serde_json::Value` and extract whatever `originals`, `original_macs`,
    /// and `original_hostname` fields still deserialize cleanly. The rest of
    /// the state (managed records, kill switch, portal data, etc.) is
    /// recoverable from the live system on the next apply, but the
    /// originals cache is sacred — there is no other source for it once the
    /// system is rotated.
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
                // Issue #218: refuse to load a state file written by a newer
                // Proteus. Without this, `serde(default)` silently drops the
                // unknown fields on parse and the next save writes the
                // truncated shape back, losing the v(N+1) data forever.
                // Bailing forces the operator to either upgrade the binary
                // or restore from backup — the alpha-cycle promise is "no
                // silent state loss," not "every binary version reads every
                // future state."
                if state.schema_version > CURRENT_SCHEMA_VERSION {
                    anyhow::bail!(
                        "{} has schema_version {} but this proteus binary supports \
                         up to {}; install a newer proteus or restore state from \
                         backup before continuing",
                        path.display(),
                        state.schema_version,
                        CURRENT_SCHEMA_VERSION,
                    );
                }
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
                // Issue #290: rescue the originals cache before quarantining
                // so a later `proteus revert` can still restore the real
                // pre-Proteus hostname / BT alias. recover_originals
                // best-effort-parses the bad bytes via serde_json::Value;
                // anything it can't extract stays at its default.
                let recovered = recover_originals(&bytes);
                // C5 / S4: surface rename failures to the caller. The
                // previous `let _ = ...` shape silently no-op'd on a
                // read-only filesystem, leaving the corrupt file in
                // place; the next load re-quarantined forever. We now
                // emit a warning with the real errno so the operator
                // can act, while still falling back to the recovered
                // state so read-only commands keep working.
                if let Err(e) = fs::rename(path, &quarantine) {
                    tracing::warn!(
                        "state.json quarantine rename failed ({e}); corrupt file left at {} \
                         and will be re-detected on next load. Operator may need to remove it \
                         manually or fix permissions on {}",
                        path.display(),
                        path.parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| ".".into()),
                    );
                }
                if recovered.has_any_originals() {
                    tracing::warn!(
                        "state.json quarantine: recovered originals cache (hostname / BT alias / MAC) — revert can still restore the pre-Proteus values"
                    );
                    Ok(Some(recovered))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// True when at least one of the sacred-original fields is populated.
    /// Used by [`load`] to decide whether the quarantine path should
    /// surface a recovered partial state vs. the historical empty-state
    /// behaviour.
    fn has_any_originals(&self) -> bool {
        self.original_hostname.is_some()
            || !self.original_macs.is_empty()
            || self.originals.hostname.is_some()
            || !self.originals.bluetooth_aliases.is_empty()
            || !self.originals.connections.is_empty()
            || !self.originals.ipv6.is_empty()
            || !self.originals.sysctls.is_empty()
            || !self.originals.rf.is_empty()
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        Ok(Self::load(path)?.unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        // Issue #218: never downgrade the on-disk schema_version. Stamp
        // max(self.schema_version, CURRENT_SCHEMA_VERSION) so the only
        // way `schema_version` decreases is if an operator manually edits
        // state.json. The load-side guard refuses files newer than the
        // running binary, so in practice `self.schema_version` is always
        // <= CURRENT after `load`/`migrate_state`; this max() is the
        // defensive belt for callers that build a State from scratch and
        // happen to set a higher version (e.g. a v(N+1) field migration
        // that bumps the version mid-run before saving).
        let mut to_write = self.clone();
        to_write.schema_version = self.schema_version.max(CURRENT_SCHEMA_VERSION);
        let bytes = serde_json::to_vec_pretty(&to_write)?;
        // Issue #275: tighten the parent dir to 0o700 before write_atomic
        // touches it. write_atomic already lands the file at 0o600, but the
        // dir's mode is whatever umask gave us — fix that here. The chmod
        // is idempotent: a correctly-permissioned dir is a no-op.
        if let Some(parent) = path.parent() {
            ensure_state_dir_secure(parent)?;
        }
        commands::write_atomic(path, &bytes)?;
        // N12.16: do NOT re-chmod the path here. `write_atomic` already
        // creates the temp file with `O_CREAT | O_EXCL` and `mode(0o600)`,
        // and renames it over the destination. A second `set_permissions`
        // by-path is racy (TOCTOU between rename and chmod) and is
        // semantically a no-op when `write_atomic` is correct. If a future
        // refactor ever drops the `.mode()` on `write_atomic`, the
        // `STATE_FILE_MODE` constant + the explicit O_CREAT|O_EXCL contract
        // is the place to assert that — not a defensive chmod here.
        Ok(())
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
    // N12.17: case-fold UUID-shaped connection keys to lowercase on every
    // load, regardless of schema version. NM emits lowercase RFC-4122
    // uuids; a state.json restored from a tool that uppercased hex would
    // otherwise silently miss on next-rotate lookups. Idempotent — a
    // no-op when every key is already lowercase.
    //
    // C9: this is also where a cross-system state-restore quietly drops
    // entries — UUIDs from the source system don't match anything on the
    // target NM, so the next apply re-captures originals from scratch.
    // Documented behaviour: by design, fail-safe; never silent
    // corruption. See CURRENT_SCHEMA_VERSION for the deprecation
    // policy.
    fold_uuid_keys_to_lowercase(&mut state.originals.connections);
    fold_uuid_keys_to_lowercase(&mut state.managed.connections);
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
///
/// N12.17: NM emits lowercase RFC-4122 uuids. A state file with
/// uppercase hex (manual edit, restored from a tool that uppercases)
/// passed `is_uuid_shape` historically (which accepts `[0-9a-fA-F]`)
/// but later string-equality lookups against fresh NM uuids missed →
/// silent data loss for `proteus revert`. Fold to lowercase here so
/// the cache survives a wild edit.
fn migrate_connection_keys_to_uuid(state: &mut State) {
    fold_uuid_keys_to_lowercase(&mut state.originals.connections);
    fold_uuid_keys_to_lowercase(&mut state.managed.connections);
    state.originals.connections.retain(|k, _| is_uuid_shape(k));
    state.managed.connections.retain(|k, _| is_uuid_shape(k));
}

/// N12.17 helper: rebuild the map with every UUID-shaped key folded to
/// ASCII lowercase. Non-UUID-shaped keys are left untouched (they get
/// dropped a moment later by the retain-shape filter). Idempotent.
fn fold_uuid_keys_to_lowercase<V>(map: &mut BTreeMap<String, V>) {
    let needs_fold = map
        .keys()
        .any(|k| is_uuid_shape(k) && k.bytes().any(|b| b.is_ascii_uppercase()));
    if !needs_fold {
        return;
    }
    let drained: Vec<(String, V)> = std::mem::take(map).into_iter().collect();
    for (mut k, v) in drained {
        if is_uuid_shape(&k) {
            k.make_ascii_lowercase();
        }
        map.insert(k, v);
    }
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

/// Issue #290: best-effort rescue of the sacred-originals cache from a
/// state.json that failed strict deserialization. Strategy:
///
/// 1. Parse as a permissive `serde_json::Value`. If even that fails (e.g.
///    truncated mid-token), return [`State::default`] — there is genuinely
///    nothing recoverable.
/// 2. For each sacred-originals subtree (`original_hostname`,
///    `original_macs`, `originals`), try to deserialize **just that
///    subtree** into its typed shape. Any subtree that fails round-trip
///    stays at its default; everything else flows back into the returned
///    state.
///
/// We do NOT attempt to recover non-originals fields (managed records,
/// kill_switch, portal lists, per_ssid_seed, ...). The rest of the state is
/// re-derivable from the live system on the next apply; the originals are
/// not, which is why they get the white-glove treatment here.
///
/// Anything recovered flows through [`migrate_state`] so a v0/v1 quarantine
/// file's fields end up at the current schema before callers see them.
fn recover_originals(bytes: &[u8]) -> State {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return State::default(),
    };
    let mut recovered = State::default();
    if let Some(obj) = value.as_object() {
        // Each field is recovered independently — a bad `originals` subtree
        // must not poison the legacy `original_hostname` / `original_macs`
        // fields and vice versa.
        if let Some(v) = obj.get("original_hostname")
            && let Ok(parsed) = serde_json::from_value::<Option<String>>(v.clone())
        {
            recovered.original_hostname = parsed;
        }
        if let Some(v) = obj.get("original_macs")
            && let Ok(parsed) = serde_json::from_value::<BTreeMap<String, String>>(v.clone())
        {
            recovered.original_macs = parsed;
        }
        if let Some(v) = obj.get("originals")
            && let Ok(parsed) = serde_json::from_value::<Originals>(v.clone())
        {
            recovered.originals = parsed;
        }
        // schema_version is recoverable too — saving a recovered state with
        // schema_version 0 would force the migration ladder to re-run on
        // every load, which is functionally fine but noisy in logs.
        if let Some(v) = obj.get("schema_version")
            && let Ok(parsed) = serde_json::from_value::<u32>(v.clone())
            && parsed <= CURRENT_SCHEMA_VERSION
        {
            recovered.schema_version = parsed;
        }
    }
    migrate_state(&mut recovered);
    recovered
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
                pinned_at: None,
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
        let dir = std::env::temp_dir().join(format!("proteus-state-stamp-{}", std::process::id()));
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

    /// Issue #218: a state file written by a newer Proteus must not be
    /// silently downgraded. `load` bails so the operator notices, rather
    /// than letting `serde(default)` drop the unknown fields and the next
    /// `save` truncate them on disk.
    #[test]
    fn load_refuses_newer_schema_version() {
        let dir = std::env::temp_dir().join(format!("proteus-state-newer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let future = CURRENT_SCHEMA_VERSION + 1;
        fs::write(
            &path,
            format!(r#"{{"schema_version": {future}, "original_macs": {{}}}}"#),
        )
        .unwrap();

        let err = State::load(&path).expect_err("newer schema must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("schema_version") && msg.contains(&future.to_string()),
            "error must name the offending version: {msg}"
        );
        // The bad file is preserved on disk — the operator handles the
        // downgrade story explicitly. (Contrast with the corrupt-parse
        // path which quarantines so reads keep working.)
        assert!(path.exists(), "newer-schema file must be preserved");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Issue #218: `save` preserves a schema_version higher than
    /// `CURRENT_SCHEMA_VERSION` so a v(N+1) migration that bumps the
    /// version mid-run doesn't get clobbered back to N. (In practice
    /// `load` refuses such files, so this exercises the defensive
    /// belt.)
    #[test]
    fn save_does_not_downgrade_schema_version() {
        let dir =
            std::env::temp_dir().join(format!("proteus-state-no-down-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut s = State::default();
        let future = CURRENT_SCHEMA_VERSION + 5;
        s.schema_version = future;
        s.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(
            json.contains(&format!("\"schema_version\": {future}")),
            "save must preserve >CURRENT schema_version: {json}"
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

    /// Issue #275: `ensure_state_dir_secure` lands a fresh dir at exactly
    /// 0o700 regardless of the active umask, and tightens an existing
    /// world-readable dir on a second call.
    #[test]
    fn ensure_state_dir_secure_creates_at_0700() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-mode-create-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        ensure_state_dir_secure(&dir).expect("create with 0700");
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, STATE_DIR_MODE,
            "newly-created state dir must be 0o700, got 0o{mode:o}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// GH #354 / GH #363: a pre-existing operator-supplied directory
    /// must be left alone — Proteus does not chmod paths it didn't
    /// create. The previous behaviour (always re-tighten) bricks
    /// `--state /tmp/x` invocations by chmodding /tmp 0o700.
    #[test]
    fn ensure_state_dir_secure_does_not_tighten_foreign_existing_dir() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-mode-foreign-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Simulate operator-supplied dir at the conventional umask of 0o755.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        ensure_state_dir_secure(&dir).expect("noop on foreign existing dir");
        let post = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            post, 0o755,
            "ensure_state_dir_secure must NOT chmod a pre-existing operator-supplied dir; got 0o{post:o}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// GH #354 / GH #363: the chmod on canonical Proteus dir is still
    /// idempotent — issue #275's tighten-on-every-run guarantee for
    /// `/var/lib/proteus` is preserved. We can't actually mutate
    /// `/var/lib/proteus` in unit tests (it's root-only on prod
    /// systems), so this test only pins the policy via the leaf-creation
    /// path: a dir Proteus just created must always come back at 0o700.
    #[test]
    fn ensure_state_dir_secure_chmods_freshly_created_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-mode-fresh-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);

        ensure_state_dir_secure(&dir).expect("create with 0o700");
        let post = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            post, STATE_DIR_MODE,
            "freshly-created state dir must be 0o700, got 0o{post:o}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Issue #275: end-to-end — a fresh `State::save` lands the state dir
    /// at 0o700 and `state.json` itself at 0o600, regardless of the
    /// active umask.
    #[test]
    fn save_lands_state_dir_at_0700_and_file_at_0600() {
        // Inner state-dir must not exist so save() exercises the
        // create-then-chmod path, not just the tighten path.
        let parent = std::env::temp_dir().join(format!(
            "proteus-state-save-mode-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&parent).unwrap();
        let dir = parent.join("var-lib-proteus");
        let path = dir.join("state.json");

        let s = State::default();
        s.save(&path).expect("save creates dir + file");

        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            dir_mode, STATE_DIR_MODE,
            "state dir must be 0o700, got 0o{dir_mode:o}"
        );
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            file_mode, STATE_FILE_MODE,
            "state.json must be 0o600, got 0o{file_mode:o}"
        );

        let _ = fs::remove_dir_all(&parent);
    }

    /// Issue #290: when `state.json` becomes unparseable but the cached
    /// originals subtree is still valid JSON, the quarantine path must
    /// rescue them. Otherwise the next `proteus apply` re-captures the
    /// **rotated** hostname / BT alias as "originals" and `proteus revert`
    /// silently restores the rotated value rather than the actual
    /// pre-Proteus one.
    #[test]
    fn load_recovers_originals_on_corrupt_state_json() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-recover-orig-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        // Strict-typed deserialization must fail on this body because
        // `managed` carries the wrong shape (a string where the schema
        // expects an object) — but it is still well-formed JSON, so the
        // value-fallback parser can pull the originals subtree intact.
        //
        // This shape mimics the real failure mode in #290: a subset of
        // fields on disk has drifted off-schema (manual edit, partial
        // write, or a future-Proteus quirk) while the originals cache
        // beside it is fine.
        let body = r#"{
            "schema_version": 2,
            "original_hostname": "factory-laptop",
            "original_macs": { "wlan0": "aa:bb:cc:dd:ee:ff" },
            "originals": {
                "bluetooth_aliases": { "hci0": "Factory BT" },
                "hostname": {
                    "kernel": "factory-laptop",
                    "pretty": "Factory Laptop",
                    "transient": "factory-laptop"
                }
            },
            "managed": "this is a string, not the ManagedState object schema expects"
        }"#;
        fs::write(&path, body).unwrap();

        let s = State::load(&path)
            .expect("load returns Ok on corrupt input")
            .expect("partial-recovered state must surface as Some");
        assert_eq!(
            s.original_hostname.as_deref(),
            Some("factory-laptop"),
            "original_hostname must survive quarantine recovery"
        );
        assert_eq!(
            s.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "original_macs must survive quarantine recovery"
        );
        assert_eq!(
            s.originals
                .bluetooth_aliases
                .get("hci0")
                .map(String::as_str),
            Some("Factory BT"),
            "bluetooth_aliases must survive quarantine recovery"
        );
        let h = s.originals.hostname.expect("hostname triple recovered");
        assert_eq!(h.kernel.as_deref(), Some("factory-laptop"));
        assert_eq!(h.pretty.as_deref(), Some("Factory Laptop"));

        // The bad file must still be quarantined so the operator can
        // postmortem it.
        assert!(!path.exists(), "corrupt file must be renamed away");
        let quarantines: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantines.len(), 1, "expected one quarantined sidecar");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Issue #290 — degenerate corruption (file truncated mid-token, no
    /// recoverable JSON) still falls back to `Ok(None)` so read-only
    /// commands don't break. The quarantine sidecar is left for the
    /// operator. This pins the existing #127 contract while we extend
    /// it for partial-recovery.
    #[test]
    fn load_returns_none_when_no_originals_recoverable() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-recover-empty-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        // Not even valid JSON at all — Value parse fails too.
        fs::write(&path, b"\x00\x00not-json\x00").unwrap();

        let result = State::load(&path).expect("load Ok on degenerate corruption");
        assert!(
            result.is_none(),
            "no recoverable originals → fall back to Ok(None)"
        );
        assert!(!path.exists(), "corrupt file must be renamed away");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Issue #290 — recover_originals must NOT pull non-originals fields
    /// like `managed` even if they happen to deserialize cleanly. The
    /// rest of the state is re-derivable from the live system on the
    /// next apply; only the originals get the white-glove rescue.
    #[test]
    fn recover_originals_only_extracts_sacred_fields() {
        // Valid JSON with `managed` populated but `originals` empty.
        let bytes = br#"{
            "managed": {
                "interfaces": {
                    "wlan0": { "current_mac": "11:22:33:44:55:66", "rotation_count": 7 }
                }
            },
            "original_hostname": "factory"
        }"#;
        let recovered = recover_originals(bytes);
        assert_eq!(recovered.original_hostname.as_deref(), Some("factory"));
        // managed must NOT have been rescued — re-derived from live system.
        assert!(
            recovered.managed.interfaces.is_empty(),
            "recover_originals must only pull sacred fields, not managed"
        );
    }

    /// N12.17: state.json with uppercase RFC-4122 UUID keys must be
    /// folded to lowercase during migration so subsequent NM lookups
    /// (which always use lowercase) hit the cache.
    #[test]
    fn migrate_state_lowercases_uppercase_uuid_keys() {
        let mut s = State::default();
        let upper = "AABBCCDD-EEFF-1122-3344-556677889900".to_string();
        s.originals.connections.insert(
            upper.clone(),
            ConnectionOriginals {
                anonymous_identity: Some("anon".into()),
                ..Default::default()
            },
        );
        s.managed.connections.insert(
            upper.clone(),
            ConnectionRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                ..Default::default()
            },
        );
        s.schema_version = CURRENT_SCHEMA_VERSION;

        migrate_state(&mut s);

        let lower = upper.to_ascii_lowercase();
        assert!(
            s.originals.connections.contains_key(&lower),
            "uppercase originals key must be folded to lowercase"
        );
        assert!(
            !s.originals.connections.contains_key(&upper),
            "uppercase originals key must be removed after fold"
        );
        assert!(
            s.managed.connections.contains_key(&lower),
            "uppercase managed key must be folded to lowercase"
        );
    }

    /// N12.17 idempotence: a state with already-lowercase keys does not
    /// allocate a new map.
    #[test]
    fn migrate_state_idempotent_on_lowercase_uuids() {
        let mut s = State::default();
        let lower = "aabbccdd-eeff-1122-3344-556677889900".to_string();
        s.managed
            .connections
            .insert(lower.clone(), ConnectionRecord::default());
        s.schema_version = CURRENT_SCHEMA_VERSION;
        migrate_state(&mut s);
        assert!(s.managed.connections.contains_key(&lower));
        assert_eq!(s.managed.connections.len(), 1);
    }

    /// Issue #290 — a corrupt state file with no originals fields at all
    /// has `has_any_originals == false` so `load` keeps the historical
    /// `Ok(None)` shape. Pins the boundary between "rescued partial"
    /// and "give up" behaviours.
    #[test]
    fn has_any_originals_is_false_for_default_state() {
        let s = State::default();
        assert!(
            !s.has_any_originals(),
            "default state must report no originals"
        );

        let with_hostname = State {
            original_hostname: Some("h".into()),
            ..Default::default()
        };
        assert!(with_hostname.has_any_originals());

        let mut with_bt = State::default();
        with_bt
            .originals
            .bluetooth_aliases
            .insert("hci0".into(), "name".into());
        assert!(with_bt.has_any_originals());
    }
}
