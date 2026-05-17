// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus state info` — read-only summary of `state.json` (#300).
//!
//! Roadmap v0.4.x no-brainer: surfaces the schema version, file path,
//! size in bytes, count of managed interfaces / connections / pinned
//! entries / cached originals, and last-rotated timestamps per iface.
//! Designed as the support-desk diagnostic adjacent to `proteus status`
//! — purely read-only, never mutates the state file. Future subcommands
//! (`state migrate`, `state dump`, ...) hang off the same top-level
//! `proteus state` namespace.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::display::display_safe;
use crate::exit;
use crate::state::{CURRENT_SCHEMA_VERSION, State};

#[derive(Debug, Serialize)]
struct IfaceLastRotated {
    iface: String,
    last_rotated: Option<String>,
    rotation_count: u64,
}

#[derive(Debug, Serialize)]
struct Counts {
    managed_interfaces: usize,
    managed_connections: usize,
    pinned_interfaces: usize,
    pinned_connections: usize,
    original_macs: usize,
    bluetooth_aliases: usize,
    connection_originals: usize,
    ipv6_originals: usize,
    sysctl_originals: usize,
    rf_originals: usize,
    known_portal_ssids: usize,
    per_ssid_seed: usize,
}

#[derive(Debug, Serialize)]
struct StateInfoReport {
    /// `true` when `state.json` did not exist at `path`. The remaining
    /// fields still surface so wrappers can read a stable shape (e.g.
    /// `size_bytes: 0`, empty counts) without branching on presence.
    cold: bool,
    /// Schema version on disk after the migration ladder ran. Equal to
    /// `CURRENT_SCHEMA_VERSION` for a state file written by this binary.
    schema_version: u32,
    /// Version this Proteus binary understands. Compare to
    /// `schema_version` to spot a state migrated by a newer Proteus.
    current_schema_version: u32,
    path: String,
    size_bytes: u64,
    counts: Counts,
    captured_by_version: Option<String>,
    captured_at: Option<String>,
    original_hostname: Option<String>,
    /// `last_rotated` timestamp + rotation_count per managed iface. One
    /// entry per `state.managed.interfaces` row; ordered by iface name
    /// (BTreeMap iteration).
    interfaces: Vec<IfaceLastRotated>,
}

/// Render a read-only summary of `state.json` to stdout.
///
/// `state_path` honours the global `--state <path>` override; a cold
/// install with no state file yet returns a `cold = true` report rather
/// than an error so the support diagnostic still works on a fresh
/// system. Parse failures bubble out as `Err` and the caller exits with
/// `GENERIC_ERROR`.
pub fn run_info(json: bool, state_path: Option<&Path>) -> Result<u8> {
    let path = super::state_path(state_path);
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let loaded = State::load(&path)?;
    let cold = loaded.is_none();
    let state = loaded.unwrap_or_default();

    let report = build_report(&state, &path, size_bytes, cold);

    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn build_report(state: &State, path: &Path, size_bytes: u64, cold: bool) -> StateInfoReport {
    let pinned_interfaces = state
        .managed
        .interfaces
        .values()
        .filter(|r| r.pinned.is_some())
        .count();
    let pinned_connections = state
        .managed
        .connections
        .values()
        .filter(|r| r.pinned.is_some())
        .count();

    let interfaces: Vec<IfaceLastRotated> = state
        .managed
        .interfaces
        .iter()
        .map(|(iface, rec)| IfaceLastRotated {
            iface: iface.clone(),
            last_rotated: rec.last_rotated.clone(),
            rotation_count: rec.rotation_count,
        })
        .collect();

    StateInfoReport {
        cold,
        schema_version: state.schema_version,
        current_schema_version: CURRENT_SCHEMA_VERSION,
        path: path.display().to_string(),
        size_bytes,
        counts: Counts {
            managed_interfaces: state.managed.interfaces.len(),
            managed_connections: state.managed.connections.len(),
            pinned_interfaces,
            pinned_connections,
            original_macs: state.original_macs.len(),
            bluetooth_aliases: state.originals.bluetooth_aliases.len(),
            connection_originals: state.originals.connections.len(),
            ipv6_originals: state.originals.ipv6.len(),
            sysctl_originals: state.originals.sysctls.len(),
            rf_originals: state.originals.rf.len(),
            known_portal_ssids: state.known_portal_ssids.len(),
            per_ssid_seed: state.per_ssid_seed.len(),
        },
        captured_by_version: state.captured_by_version.clone(),
        captured_at: state.captured_at.clone(),
        original_hostname: state.original_hostname.clone(),
        interfaces,
    }
}

fn print_human(r: &StateInfoReport) {
    if r.cold {
        println!("(no state file yet — proteus has not been applied on this system)");
    }
    println!("path:                  {}", r.path);
    println!("size:                  {} bytes", r.size_bytes);
    if r.schema_version == r.current_schema_version {
        println!("schema_version:        {}", r.schema_version);
    } else {
        println!(
            "schema_version:        {} (binary supports up to {})",
            r.schema_version, r.current_schema_version
        );
    }
    println!(
        "captured_by_version:   {}",
        r.captured_by_version.as_deref().unwrap_or("(none)")
    );
    println!(
        "captured_at:           {}",
        r.captured_at.as_deref().unwrap_or("(none)")
    );
    // original_hostname is operator-controlled (DHCP host the laptop
    // last saw, kernel hostname) — sanitize via display_safe.
    match r.original_hostname.as_deref() {
        Some(h) => println!("original_hostname:     {}", display_safe(h)),
        None => println!("original_hostname:     (none)"),
    }
    println!("counts:");
    println!("  managed_interfaces:  {}", r.counts.managed_interfaces);
    println!("  managed_connections: {}", r.counts.managed_connections);
    println!("  pinned_interfaces:   {}", r.counts.pinned_interfaces);
    println!("  pinned_connections:  {}", r.counts.pinned_connections);
    println!("  original_macs:       {}", r.counts.original_macs);
    println!("  bluetooth_aliases:   {}", r.counts.bluetooth_aliases);
    println!("  connection_originals: {}", r.counts.connection_originals);
    println!("  ipv6_originals:      {}", r.counts.ipv6_originals);
    println!("  sysctl_originals:    {}", r.counts.sysctl_originals);
    println!("  rf_originals:        {}", r.counts.rf_originals);
    println!("  known_portal_ssids:  {}", r.counts.known_portal_ssids);
    println!("  per_ssid_seed:       {}", r.counts.per_ssid_seed);

    if r.interfaces.is_empty() {
        println!("last_rotated:          (no managed interfaces)");
    } else {
        println!("last_rotated:");
        for entry in &r.interfaces {
            // Iface name traces back to NM / sysfs — sanitize.
            let iface = display_safe(&entry.iface);
            let stamp = entry.last_rotated.as_deref().unwrap_or("(never)");
            println!(
                "  {iface:<12} {stamp}  [rotations={}]",
                entry.rotation_count
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ConnectionRecord, InterfaceRecord};

    fn populated_state() -> State {
        let mut s = State {
            schema_version: CURRENT_SCHEMA_VERSION,
            captured_by_version: Some("0.4.4-beta".into()),
            captured_at: Some("2026-05-17T00:00:00Z".into()),
            original_hostname: Some("factory-laptop".into()),
            ..Default::default()
        };
        s.original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        s.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                current_mac: Some("11:22:33:44:55:66".into()),
                pinned: Some("11:22:33:44:55:66".into()),
                last_rotated: Some("2026-05-17T12:00:00Z".into()),
                rotation_count: 4,
            },
        );
        s.managed.interfaces.insert(
            "eth0".into(),
            InterfaceRecord {
                current_mac: Some("aa:00:00:00:00:01".into()),
                pinned: None,
                last_rotated: None,
                rotation_count: 0,
            },
        );
        let uuid = "aabbccdd-eeff-1122-3344-556677889900".to_string();
        s.managed.connections.insert(
            uuid,
            ConnectionRecord {
                current_mac: Some("11:22:33:44:55:66".into()),
                pinned: None,
                last_rotated: Some("2026-05-17T12:00:00Z".into()),
                rotation_count: 4,
            },
        );
        s.known_portal_ssids.push("Coffee Shop".into());
        s
    }

    #[test]
    fn build_report_counts_match_state() {
        let s = populated_state();
        let report = build_report(&s, Path::new("/tmp/state.json"), 123, false);
        assert!(!report.cold);
        assert_eq!(report.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(report.size_bytes, 123);
        assert_eq!(report.counts.managed_interfaces, 2);
        assert_eq!(report.counts.managed_connections, 1);
        assert_eq!(report.counts.pinned_interfaces, 1);
        assert_eq!(report.counts.pinned_connections, 0);
        assert_eq!(report.counts.original_macs, 1);
        assert_eq!(report.counts.known_portal_ssids, 1);
        assert_eq!(report.captured_by_version.as_deref(), Some("0.4.4-beta"));
        assert_eq!(report.original_hostname.as_deref(), Some("factory-laptop"));
        assert_eq!(report.interfaces.len(), 2);
        // BTreeMap iteration order: eth0 before wlan0.
        assert_eq!(report.interfaces[0].iface, "eth0");
        assert_eq!(report.interfaces[1].iface, "wlan0");
        assert_eq!(
            report.interfaces[1].last_rotated.as_deref(),
            Some("2026-05-17T12:00:00Z")
        );
        assert_eq!(report.interfaces[1].rotation_count, 4);
    }

    #[test]
    fn build_report_handles_empty_state() {
        let s = State::default();
        let report = build_report(&s, Path::new("/tmp/state.json"), 0, true);
        assert!(report.cold);
        assert_eq!(report.counts.managed_interfaces, 0);
        assert_eq!(report.counts.original_macs, 0);
        assert!(report.interfaces.is_empty());
        assert!(report.captured_by_version.is_none());
        assert!(report.original_hostname.is_none());
    }

    #[test]
    fn run_info_cold_state_returns_success() {
        // Point at a path that genuinely doesn't exist — `run_info` must
        // still exit 0 (the support diagnostic must work on a fresh
        // install).
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-info-cold-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let code = run_info(true, Some(&path)).expect("run_info on missing state file");
        assert_eq!(code, exit::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_info_reads_existing_state() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-state-info-warm-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        populated_state().save(&path).unwrap();
        let code = run_info(true, Some(&path)).expect("run_info on warm state");
        assert_eq!(code, exit::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
