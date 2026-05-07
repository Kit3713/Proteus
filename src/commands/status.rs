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
struct SystemInfo {
    systemd: bool,
    network_manager: bool,
    bluez: bool,
    systemd_resolved: bool,
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
    StatusReport {
        proteus_version: version::VERSION,
        phase: version::PHASE,
        system: detect_system(),
        interfaces: enumerate_interfaces(),
        features: feature_table(state.as_ref(), &config),
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

fn detect_system() -> SystemInfo {
    SystemInfo {
        systemd: Path::new("/run/systemd/system").is_dir(),
        network_manager: Path::new("/run/NetworkManager").exists()
            || Path::new("/var/run/NetworkManager").exists(),
        bluez: Path::new("/run/bluetooth").exists() || Path::new("/var/run/bluetooth").exists(),
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

fn feature_table(state: Option<&State>, config: &Config) -> Vec<FeatureStatus> {
    let mac_state = mac_rotation_state(state, config);
    vec![
        FeatureStatus {
            name: "mac-rotation",
            state: mac_state.0,
            note: mac_state.1,
        },
        FeatureStatus {
            name: "bluetooth",
            state: "not implemented".into(),
            note: "phase B (parallel PR)".into(),
        },
        FeatureStatus {
            name: "probes",
            state: "not implemented".into(),
            note: "phase C".into(),
        },
        FeatureStatus {
            name: "captive-portals",
            state: "not implemented".into(),
            note: "phase C".into(),
        },
        FeatureStatus {
            name: "dhcp-options",
            state: "not implemented".into(),
            note: "phase D".into(),
        },
        FeatureStatus {
            name: "ipv6-privacy",
            state: "not implemented".into(),
            note: "phase D".into(),
        },
        FeatureStatus {
            name: "hostname",
            state: "not implemented".into(),
            note: "phase D".into(),
        },
        FeatureStatus {
            name: "enterprise-wifi",
            state: "not implemented".into(),
            note: "phase D".into(),
        },
        FeatureStatus {
            name: "dns-ecs-strip",
            state: "not implemented".into(),
            note: "phase D".into(),
        },
        FeatureStatus {
            name: "discovery-silence",
            state: "not implemented".into(),
            note: "phase E".into(),
        },
        FeatureStatus {
            name: "stack-fingerprint",
            state: "not implemented".into(),
            note: "phase E".into(),
        },
        FeatureStatus {
            name: "rf-tx-power",
            state: "not implemented".into(),
            note: "phase E".into(),
        },
    ]
}

fn mac_rotation_state(state: Option<&State>, config: &Config) -> (String, String) {
    let any_managed = state
        .map(|s| !s.managed.interfaces.is_empty() || !s.managed.connections.is_empty())
        .unwrap_or(false);
    if any_managed {
        let n = state.map(|s| s.managed.interfaces.len()).unwrap_or(0);
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
