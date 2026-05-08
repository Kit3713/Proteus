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
    /// Roadmap Milestone 4b: surface driver/chip/firmware in one line.
    /// `None` for non-wifi or when sysfs doesn't expose the data — the
    /// renderer skips the field rather than printing "(unknown)/(unknown)".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chipset: Option<IfaceChipset>,
}

/// Compact chipset summary for the `proteus status` interfaces table.
/// Mirrors the JSON fields from `rf::ChipInfoExtended` but trimmed to
/// what's worth showing inline — operators wanting the full inventory
/// run `proteus rf chipset`.
#[derive(Debug, Serialize)]
pub struct IfaceChipset {
    pub driver: Option<String>,
    pub chip: Option<String>,
    pub firmware: Option<String>,
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
        // NCMD2.5: sysfs entries with non-UTF-8 names previously slipped
        // through `to_string_lossy` with U+FFFD substitutions, which then
        // landed in the JSON output as bogus `name = "\u{FFFD}..."`
        // values. Skip those entries entirely with a `debug!` line so
        // log-level=debug operators still see the drop. UTF-8 names go
        // through unchanged.
        let raw_name = entry.file_name();
        let Some(name) = raw_name.to_str().map(str::to_string) else {
            tracing::debug!(
                "skipping non-UTF-8 sysfs entry under /sys/class/net: {:?}",
                raw_name
            );
            continue;
        };
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
        // Roadmap Milestone 4b: best-effort chipset inline. We only
        // populate it for wifi (chipset for ethernet is a follow-up;
        // the data is in sysfs but `iw`-driven `chip_info_extended`
        // is wifi-only by design). Keep the read read-only and
        // never let it block the rest of the status pass.
        let chipset = if wireless {
            let info = crate::rf::chip_info_extended(&name);
            let chip = match (info.vendor_id.as_deref(), info.device_id.as_deref()) {
                (Some(v), Some(d)) => Some(format!("{v}:{d}")),
                _ => None,
            };
            Some(IfaceChipset {
                driver: info.driver,
                chip,
                firmware: info.firmware,
            })
        } else {
            None
        };
        out.push(Iface {
            name,
            mac,
            kind,
            wireless,
            chipset,
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
    let probes_state = probes_state(config);
    let discovery_state = discovery_silence_state(config);
    let rf_state = rf_tx_power_state(state, config);
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
            state: probes_state.0,
            note: probes_state.1,
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
            state: discovery_state.0,
            note: discovery_state.1,
        },
        FeatureStatus {
            name: "stack-fingerprint",
            state: stack_state.0,
            note: stack_state.1,
        },
        FeatureStatus {
            name: "rf-tx-power",
            state: rf_state.0,
            note: rf_state.1,
        },
    ]
}

/// Quorum-probe state. The probe runner is always available via
/// `proteus probe`; the only knobs the operator tunes are `[probes]`
/// quorum + endpoint pool, so "configured" reflects whether the
/// endpoint pool is non-empty.
fn probes_state(config: &Config) -> (String, String) {
    if config.probes.endpoints.is_empty() {
        return (
            "idle".to_string(),
            "no probe endpoints configured ([probes] endpoints is empty)".to_string(),
        );
    }
    (
        "configured".to_string(),
        format!(
            "{n} endpoint(s); quorum {q}/{t}; run `proteus probe` to test",
            n = config.probes.endpoints.len(),
            q = config.probes.quorum_n,
            t = config.probes.quorum_total,
        ),
    )
}

/// Discovery-silence feature: shipped via the resolved drop-in
/// (`MulticastDNS=no` / `LLMNR=no`) and the nft `discovery_drops` chain
/// (`ssdp_block` / `wsd_block`). All four knobs default off; reports
/// `idle` when none are on, `applied` when the resolved drop-in is on
/// disk, `configured` otherwise (knobs on but apply hasn't run yet).
fn discovery_silence_state(config: &Config) -> (String, String) {
    let mut on: Vec<&'static str> = Vec::new();
    if config.resolved.mdns_off {
        on.push("mdns");
    }
    if config.resolved.llmnr_off {
        on.push("llmnr");
    }
    if config.discovery.ssdp_block {
        on.push("ssdp");
    }
    if config.discovery.wsd_block {
        on.push("wsd");
    }
    if on.is_empty() {
        return (
            "idle".to_string(),
            "every [discovery]/[resolved] silence knob is off".to_string(),
        );
    }
    let resolved_dropin =
        crate::dns::resolved::dropin_present(&crate::dns::Paths::system_default());
    if resolved_dropin {
        return (
            "applied".to_string(),
            format!("{} silenced; see `proteus resolved status`", on.join(",")),
        );
    }
    (
        "configured".to_string(),
        format!(
            "{}; run `proteus apply` (resolved + nft) to install",
            on.join(",")
        ),
    )
}

/// RF TX-power-reduce feature: shipped via `proteus rf apply`. `applied`
/// when at least one originals row has been captured (i.e. apply has run
/// and revert hasn't cleaned up yet); `idle` until then.
fn rf_tx_power_state(state: Option<&State>, config: &Config) -> (String, String) {
    if !config.rf.tx_power_reduce {
        return (
            "idle".to_string(),
            "disabled in config (rf.tx_power_reduce = false)".to_string(),
        );
    }
    let captured = state.map(|s| !s.originals.rf.is_empty()).unwrap_or(false);
    if captured {
        return (
            "applied".to_string(),
            format!(
                "TX power reduced by {db} dB; see `proteus rf status`",
                db = config.rf.tx_power_reduction_db,
            ),
        );
    }
    (
        "idle".to_string(),
        format!(
            "configured ({db} dB reduction); run `proteus rf apply` to install",
            db = config.rf.tx_power_reduction_db,
        ),
    )
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
            // Roadmap Milestone 4b: per-iface chipset line. Skip if no
            // info — we don't want a "(unknown)/(unknown)/(unknown)"
            // row cluttering the table on systems without iw-tools.
            if let Some(c) = &i.chipset
                && (c.driver.is_some() || c.chip.is_some() || c.firmware.is_some())
            {
                let driver = c.driver.as_deref().unwrap_or("?");
                let chip = c.chip.as_deref().unwrap_or("?");
                let fw = c.firmware.as_deref().unwrap_or("?");
                println!("    chipset: driver={driver} chip={chip} firmware={fw}");
            }
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
