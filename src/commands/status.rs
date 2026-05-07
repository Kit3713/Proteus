// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    proteus_version: &'static str,
    phase: char,
    system: SystemInfo,
    interfaces: Vec<Iface>,
    features: Vec<FeatureStatus>,
}

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub systemd: bool,
    pub network_manager: bool,
    pub bluez: bool,
    pub systemd_resolved: bool,
}

#[derive(Debug, Serialize)]
pub struct Iface {
    pub name: String,
    pub mac: Option<String>,
    pub kind: String,
    pub wireless: bool,
}

#[derive(Debug, Serialize)]
struct FeatureStatus {
    name: &'static str,
    state: String,
    note: String,
}

pub fn run(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let report = build_report(state_path, config_path);
    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn build_report(state_path: Option<&Path>, config_path: Option<&Path>) -> StatusReport {
    let state = load_state(state_path);
    let config = load_config(config_path);
    let system = detect_system();
    let features = feature_table(state.as_ref(), &config, &system);
    StatusReport {
        proteus_version: version::VERSION,
        phase: version::PHASE,
        system,
        interfaces: enumerate_interfaces(),
        features,
    }
}

fn load_state(path: Option<&Path>) -> Option<State> {
    let path = super::state_path(path);
    State::load(&path).ok().flatten()
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    crate::config::Config::default_or_loaded(&path).unwrap_or_default()
}

pub fn detect_system() -> SystemInfo {
    SystemInfo {
        systemd: Path::new("/run/systemd/system").is_dir(),
        network_manager: Path::new("/run/NetworkManager").exists()
            || Path::new("/var/run/NetworkManager").exists(),
        bluez: crate::bluetooth::detect_runtime(),
        systemd_resolved: Path::new("/run/systemd/resolve").exists(),
    }
}

pub fn enumerate_interfaces() -> Vec<Iface> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("skip /sys/class/net: {e}");
            return out;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        let base = entry.path();
        let mac = read_trim(&base.join("address")).filter(|m| m != "00:00:00:00:00:00");
        let kind = classify_kind(&base);
        let wireless = base.join("wireless").exists() || base.join("phy80211").exists();
        if kind == "virtual" {
            continue;
        }
        out.push(Iface {
            name,
            mac,
            kind,
            wireless,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn classify_kind(base: &Path) -> String {
    if let Ok(target) = std::fs::read_link(base)
        && target.to_string_lossy().contains("devices/virtual")
    {
        return "virtual".into();
    }
    if base.join("wireless").exists() || base.join("phy80211").exists() {
        return "wifi".into();
    }
    if base.join("device").exists() {
        return "ethernet".into();
    }
    "other".into()
}

fn read_trim(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s.trim().to_owned()),
        Err(e) => {
            tracing::debug!("skip {}: {e}", path.display());
            None
        }
    }
}

fn feature_table(
    state: Option<&State>,
    config: &Config,
    system: &SystemInfo,
) -> Vec<FeatureStatus> {
    let mac_state = mac_rotation_state(state, config);
    let bt_state = bluetooth_state(state, config, system);
    let host_state = hostname_state(state, config);
    let v6_state = ipv6_state(state, config);
    let ew_state = enterprise_wifi_state(state, config);
    let stack_state = stack_state(state, config);
    let dns_state = dns_ecs_state(config);
    let dhcp_state = dhcp_state(state, config);
    let portal_state = captive_portal_state(state, config);
    vec![
        FeatureStatus {
            name: "mac-rotation",
            state: mac_state.0,
            note: mac_state.1,
        },
        FeatureStatus {
            name: "bluetooth",
            state: bt_state.0,
            note: bt_state.1,
        },
        FeatureStatus {
            name: "probes",
            state: "not implemented".into(),
            note: "phase C".into(),
        },
        FeatureStatus {
            name: "captive-portals",
            state: portal_state.0,
            note: portal_state.1,
        },
        FeatureStatus {
            name: "dhcp-options",
            state: dhcp_state.0,
            note: dhcp_state.1,
        },
        FeatureStatus {
            name: "ipv6-privacy",
            state: v6_state.0,
            note: v6_state.1,
        },
        FeatureStatus {
            name: "hostname",
            state: host_state.0,
            note: host_state.1,
        },
        FeatureStatus {
            name: "enterprise-wifi",
            state: ew_state.0,
            note: ew_state.1,
        },
        FeatureStatus {
            name: "dns-ecs-strip",
            state: dns_state.0,
            note: dns_state.1,
        },
        FeatureStatus {
            name: "discovery-silence",
            state: "not implemented".into(),
            note: "phase E".into(),
        },
        FeatureStatus {
            name: "stack-fingerprint",
            state: stack_state.0,
            note: stack_state.1,
        },
        FeatureStatus {
            name: "rf-tx-power",
            state: "not implemented".into(),
            note: "phase E".into(),
        },
    ]
}

fn stack_state(state: Option<&State>, config: &Config) -> (String, String) {
    let any_knob = config.stack.tcp_timestamps_off
        || config.stack.icmpv6_hardening
        || config.stack.suppress_gratuitous_arp;
    if !any_knob {
        return ("idle".to_string(), "every [stack] knob is off".to_string());
    }
    let dropin_present = Path::new(crate::stack::DROPIN_PATH).exists();
    let captured = state
        .map(|s| !s.originals.sysctls.is_empty())
        .unwrap_or(false);
    if dropin_present && captured {
        return (
            "applied".to_string(),
            format!("drop-in at {}", crate::stack::DROPIN_PATH),
        );
    }
    if dropin_present {
        return (
            "applied".to_string(),
            "drop-in present (no captured originals — apply to refresh)".to_string(),
        );
    }
    (
        "idle".to_string(),
        "configured; run `proteus stack apply` to write the drop-in".to_string(),
    )
}

fn bluetooth_state(
    state: Option<&State>,
    config: &Config,
    system: &SystemInfo,
) -> (String, String) {
    if !config.bluetooth.enabled {
        return (
            "idle".to_string(),
            "disabled in config (bluetooth.enabled = false)".to_string(),
        );
    }
    if !system.bluez {
        return ("skipped".to_string(), "no BlueZ detected".to_string());
    }
    let cached = state
        .map(|s| !s.originals.bluetooth_aliases.is_empty())
        .unwrap_or(false);
    if cached {
        let n = state
            .map(|s| s.originals.bluetooth_aliases.len())
            .unwrap_or(0);
        return (
            "applied".to_string(),
            format!("{n} adapter(s) cached; see `proteus bluetooth status`"),
        );
    }
    (
        "idle".to_string(),
        "BlueZ present; run `proteus bluetooth apply` to manage".to_string(),
    )
}

fn ipv6_state(state: Option<&State>, config: &Config) -> (String, String) {
    if !config.ipv6.enabled {
        return (
            "idle".to_string(),
            "disabled in config (ipv6.enabled = false)".to_string(),
        );
    }
    let cached = state.map(|s| !s.originals.ipv6.is_empty()).unwrap_or(false);
    if cached {
        let n = state.map(|s| s.originals.ipv6.len()).unwrap_or(0);
        return (
            "applied".to_string(),
            format!("{n} interface(s) hardened; see `proteus ipv6 status`"),
        );
    }
    (
        "idle".to_string(),
        "configured; run `proteus ipv6 apply` to harden".to_string(),
    )
}

fn dhcp_state(state: Option<&State>, config: &Config) -> (String, String) {
    if !config.dhcp.enabled {
        return (
            "idle".to_string(),
            "disabled in config (dhcp.enabled = false)".to_string(),
        );
    }
    let any_cached = state
        .map(|s| {
            s.originals
                .connections
                .values()
                .any(|c| c.dhcp_settings.is_some())
        })
        .unwrap_or(false);
    if any_cached {
        let n = state
            .map(|s| {
                s.originals
                    .connections
                    .values()
                    .filter(|c| c.dhcp_settings.is_some())
                    .count()
            })
            .unwrap_or(0);
        return (
            "applied".to_string(),
            format!("{n} connection(s) tracked; see `proteus dhcp status`"),
        );
    }
    (
        "idle".to_string(),
        "run `proteus dhcp apply` to suppress 12/60/61/81 + DUID".to_string(),
    )
}

fn dns_ecs_state(config: &Config) -> (String, String) {
    if !config.dns.strip_edns_client_subnet {
        return (
            "idle".to_string(),
            "disabled in config (dns.strip_edns_client_subnet = false)".to_string(),
        );
    }
    let paths = crate::dns::Paths::system_default();
    if let Some(reason) = crate::dns::detect_defer_system(&paths) {
        return (
            "skipped".to_string(),
            format!("deferred to {}", reason.tool_name()),
        );
    }
    if crate::dns::apply::dropin_present(&paths) {
        return (
            "applied".to_string(),
            "drop-in present at /etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf".to_string(),
        );
    }
    (
        "idle".to_string(),
        "configured; run `proteus dns apply` to install".to_string(),
    )
}

fn enterprise_wifi_state(state: Option<&State>, config: &Config) -> (String, String) {
    let managed = state.map(|s| s.originals.connections.len()).unwrap_or(0);
    if managed > 0 {
        return (
            "applied".to_string(),
            format!("{managed} connection(s) tagged; see `proteus enterprise-wifi status`"),
        );
    }
    if config.enterprise_wifi.anonymous_outer_identity {
        return (
            "idle".to_string(),
            "master switch on; run `proteus enterprise-wifi enable --connection <id>`".to_string(),
        );
    }
    (
        "idle".to_string(),
        "opt-in; default off (some auth servers reject mismatched outer ids)".to_string(),
    )
}

fn hostname_state(state: Option<&State>, config: &Config) -> (String, String) {
    if !config.hostname.enabled {
        return (
            "idle".to_string(),
            "disabled in config (hostname.enabled = false)".to_string(),
        );
    }
    let cached = state.and_then(|s| s.originals.hostname.as_ref()).is_some();
    if cached {
        return (
            "applied".to_string(),
            format!(
                "mode={}; see `proteus hostname status`",
                config.hostname.mode
            ),
        );
    }
    (
        "idle".to_string(),
        format!(
            "mode={}; run `proteus hostname rotate` to apply",
            config.hostname.mode
        ),
    )
}

fn captive_portal_state(state: Option<&State>, config: &Config) -> (String, String) {
    if !config.captive_portal.enabled {
        return (
            "idle".to_string(),
            "disabled in config (captive_portal.enabled = false)".to_string(),
        );
    }
    let known = state.map(|s| s.known_portal_ssids.len()).unwrap_or(0);
    let last = state.and_then(|s| s.last_portal_check.as_ref());
    match last {
        Some(rec) => (
            "applied".to_string(),
            format!(
                "last check: {} at {}; {known} known portal SSID(s)",
                rec.classification, rec.timestamp
            ),
        ),
        None => (
            "idle".to_string(),
            format!("detector ready; {known} known portal SSID(s); run `proteus portal status`"),
        ),
    }
}

fn mac_rotation_state(state: Option<&State>, config: &Config) -> (String, String) {
    let any_managed = state
        .map(|s| !s.managed.interfaces.is_empty() || !s.managed.connections.is_empty())
        .unwrap_or(false);
    if any_managed {
        let n = state.map(|s| s.managed.interfaces.len()).unwrap_or(0);
        // Issue #208: surface interfaces that have been rotated but have no
        // factory MAC captured. Without a captured original, `proteus revert`
        // is a no-op for that interface — the operator should know.
        let missing: Vec<&str> = state
            .map(|s| {
                s.managed
                    .interfaces
                    .keys()
                    .filter(|iface| !s.original_macs.contains_key(*iface))
                    .map(String::as_str)
                    .collect()
            })
            .unwrap_or_default();
        if !missing.is_empty() {
            return (
                "applied".to_string(),
                format!(
                    "{n} interface(s) tracked; no factory MAC captured for {} (revert will be a no-op there); see `proteus current`",
                    missing.join(", ")
                ),
            );
        }
        return (
            "applied".to_string(),
            format!("{n} interface(s) tracked; see `proteus current`"),
        );
    }
    if config.mac.enabled {
        return (
            "idle".to_string(),
            "configured but no rotations recorded yet; run `proteus rotate`".to_string(),
        );
    }
    (
        "idle".to_string(),
        "rotation core implemented; run `proteus rotate` to start".to_string(),
    )
}

fn print_human(r: &StatusReport) {
    println!("proteus {} (phase {})", r.proteus_version, r.phase);
    println!();
    println!("system:");
    println!("  systemd:           {}", yesno(r.system.systemd));
    println!("  NetworkManager:    {}", yesno(r.system.network_manager));
    println!("  BlueZ:             {}", yesno(r.system.bluez));
    println!("  systemd-resolved:  {}", yesno(r.system.systemd_resolved));
    println!();
    println!("interfaces:");
    if r.interfaces.is_empty() {
        println!("  (none detected)");
    } else {
        for i in &r.interfaces {
            let mac = i.mac.as_deref().unwrap_or("?");
            println!("  {:<12} {:<8} {}", i.name, i.kind, mac);
        }
    }
    println!();
    println!("features:");
    for f in &r.features {
        println!("  {:<22} {:<18} ({})", f.name, f.state, f.note);
    }
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
