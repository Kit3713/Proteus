// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure data DTOs for the `state.json` family emitted by `proteus original
//! --json`, `proteus state info`, and friends.
//!
//! Roadmap 1.1.1: these are the leaf structs of the State tree. The
//! top-level [`State`](../../proteus/state/struct.State.html) struct itself
//! stays in the binary (`src/state.rs`) because its inherent `load`/`save`
//! impl is bound to filesystem helpers (`commands::write_atomic`,
//! `commands::now_iso8601`) and to `KillSwitchState` — none of which belong
//! in a pure-DTO crate. `State` references these moved types through the
//! binary's re-export shims, so the on-disk / `--json` shape is unchanged
//! from 1.0.x.
//!
//! Every `#[serde(...)]` attribute below is load-bearing: `serde(default)`
//! keeps old `state.json` files loading, and each `skip_serializing_if` keeps
//! a fresh install from growing the file with useless keys. Do not touch them
//! without treating it as a state-format change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One entry in `state.per_ssid_seed`. Mirrors the public
/// `PerSsidPolicy` shape so the migration step stays a straight copy:
/// every field is `Option<String>` and missing values stay missing on
/// disk via `skip_serializing_if`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct PortalCheckRecord {
    pub timestamp: String,
    pub classification: String,
    pub ssid: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct RfOriginals {
    /// TX power in mBm (milli-dBm; the unit `iw` reports natively).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_power_mbm: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct ManagedState {
    pub interfaces: BTreeMap<String, InterfaceRecord>,
    pub connections: BTreeMap<String, ConnectionRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Issue #294: optional audit string stamped at rotate time so an
    /// operator can later correlate a rotation with the trigger (a
    /// dispatcher event, an SSID join, a manual `--reason "lab test"`,
    /// etc.). The binary sanitizes this through `rotate::sanitize_reason`
    /// (strip control bytes, trim, cap at 256 bytes) before write.
    /// Optional + `skip_serializing_if` so old state.json files keep
    /// loading and rotations without `--reason` leave the on-disk
    /// shape unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
