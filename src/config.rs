// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// Provisional schema. Phases B-E will refine. `#[serde(default)]` everywhere
// so future fields don't break older configs and vice versa.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mac: MacConfig,
    pub bluetooth: BluetoothConfig,
    pub hostname: HostnameConfig,
    pub dns: DnsConfig,
    pub discovery: DiscoveryConfig,
    pub probes: ProbesConfig,
    pub ipv6: Ipv6Config,
    pub enterprise_wifi: EnterpriseWifiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacConfig {
    pub enabled: bool,
    pub rotation_interval: String,
    pub oui_pool: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BluetoothConfig {
    pub enabled: bool,
    pub generic_alias: bool,
    pub alias_source: String,
    pub pinned_alias: Option<String>,
    pub discoverable: bool,
    pub ble_rpa: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostnameConfig {
    pub enabled: bool,
    pub mode: String,
    pub pinned_value: Option<String>,
    /// Rotate hostname every time MAC rotates. Default off — see wiki.
    pub rotate_with_mac: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsConfig {
    pub strip_edns_client_subnet: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    // SSDP/WSD off by default — they break KDE Connect and WS-Discovery printers.
    pub mdns_silence: bool,
    pub llmnr_silence: bool,
    pub ssdp_block: bool,
    pub wsd_block: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbesConfig {
    pub quorum_n: u8,
    pub quorum_total: u8,
    pub interval: String,
    pub cooldown: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ipv6Config {
    pub enabled: bool,
    pub use_temp_addresses: bool,
    pub addr_gen_mode: String,
    pub ndp_hardening: bool,
}

/// 802.1X anonymous outer identity for enterprise Wi-Fi (eduroam, corporate).
/// Opt-in, default off — some auth servers reject mismatched outer/inner
/// identities. See `proteus wiki enterprise-wifi`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnterpriseWifiConfig {
    /// Master switch. When false the feature does nothing globally; per-
    /// connection overrides can still opt specific connections in via the
    /// `enable` subcommand.
    pub anonymous_outer_identity: bool,
    /// `auto` extracts the realm from `802-1x.identity` (the part after `@`).
    /// `manual` uses `anonymous_realm` verbatim.
    pub realm_strip_strategy: String,
    /// Used when `realm_strip_strategy = "manual"`. Empty otherwise.
    pub anonymous_realm: String,
}

impl Default for EnterpriseWifiConfig {
    fn default() -> Self {
        Self {
            anonymous_outer_identity: false,
            realm_strip_strategy: "auto".into(),
            anonymous_realm: String::new(),
        }
    }
}

impl Default for Ipv6Config {
    fn default() -> Self {
        Self {
            enabled: true,
            use_temp_addresses: true,
            addr_gen_mode: "stable-privacy".into(),
            ndp_hardening: true,
        }
    }
}

impl Default for MacConfig {
    fn default() -> Self {
        Self {
            // Default off in phase A: rotation isn't implemented yet.
            enabled: false,
            rotation_interval: "2h".into(),
            oui_pool: vec![
                "apple".into(),
                "intel".into(),
                "samsung".into(),
                "dell".into(),
                "random-locally-administered".into(),
            ],
        }
    }
}

impl Default for BluetoothConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            generic_alias: true,
            alias_source: "generic".into(),
            pinned_alias: None,
            discoverable: false,
            ble_rpa: true,
        }
    }
}

impl Default for HostnameConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "wordlist".into(),
            pinned_value: None,
            rotate_with_mac: false,
        }
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            strip_edns_client_subnet: true,
        }
    }
}

impl Config {
    pub fn default_or_loaded(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

impl Default for ProbesConfig {
    fn default() -> Self {
        Self {
            quorum_n: 3,
            quorum_total: 4,
            interval: "5m".into(),
            cooldown: "60s".into(),
            endpoints: vec![
                "1.1.1.1:443".into(),
                "8.8.8.8:443".into(),
                "9.9.9.9:443".into(),
                "142.250.190.78:443".into(),
            ],
        }
    }
}
