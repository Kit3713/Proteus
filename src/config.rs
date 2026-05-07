// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

// Provisional schema. Phases B-E will refine. `#[serde(default)]` everywhere
// so future fields don't break older configs and vice versa.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mac: MacConfig,
    pub hostname: HostnameConfig,
    pub dns: DnsConfig,
    pub discovery: DiscoveryConfig,
    pub probes: ProbesConfig,
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
pub struct HostnameConfig {
    pub enabled: bool,
    pub mode: String,
    pub pinned_value: Option<String>,
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

impl Default for HostnameConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "wordlist".into(),
            pinned_value: None,
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

impl Default for ProbesConfig {
    fn default() -> Self {
        Self {
            quorum_n: 3,
            quorum_total: 4,
            interval: "5m".into(),
            cooldown: "60s".into(),
        }
    }
}
