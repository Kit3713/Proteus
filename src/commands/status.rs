// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::exit;
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
    state: &'static str,
    note: &'static str,
}

pub fn run(json: bool) -> Result<u8> {
    let report = build_report();
    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn build_report() -> StatusReport {
    StatusReport {
        proteus_version: version::VERSION,
        phase: version::PHASE,
        system: detect_system(),
        interfaces: enumerate_interfaces(),
        features: feature_table(),
    }
}

fn detect_system() -> SystemInfo {
    SystemInfo {
        // /run/systemd/system is created by PID 1 systemd; cheap and reliable.
        systemd: Path::new("/run/systemd/system").is_dir(),
        // No DBus in phase A — rely on file presence under /run.
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
    // /sys/class/net/<iface> is a symlink. If it resolves under /sys/devices/virtual, treat as virtual.
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

fn feature_table() -> Vec<FeatureStatus> {
    // Surface the future feature set so users can see what's coming.
    vec![
        FeatureStatus {
            name: "mac-rotation",
            state: "not implemented",
            note: "phase B",
        },
        FeatureStatus {
            name: "bluetooth",
            state: "not implemented",
            note: "phase B",
        },
        FeatureStatus {
            name: "probes",
            state: "not implemented",
            note: "phase C",
        },
        FeatureStatus {
            name: "captive-portals",
            state: "not implemented",
            note: "phase C",
        },
        FeatureStatus {
            name: "dhcp-options",
            state: "not implemented",
            note: "phase D",
        },
        FeatureStatus {
            name: "ipv6-privacy",
            state: "not implemented",
            note: "phase D",
        },
        FeatureStatus {
            name: "hostname",
            state: "not implemented",
            note: "phase D",
        },
        FeatureStatus {
            name: "enterprise-wifi",
            state: "not implemented",
            note: "phase D",
        },
        FeatureStatus {
            name: "dns-ecs-strip",
            state: "not implemented",
            note: "phase D",
        },
        FeatureStatus {
            name: "discovery-silence",
            state: "not implemented",
            note: "phase E",
        },
        FeatureStatus {
            name: "stack-fingerprint",
            state: "not implemented",
            note: "phase E",
        },
        FeatureStatus {
            name: "rf-tx-power",
            state: "not implemented",
            note: "phase E",
        },
    ]
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
