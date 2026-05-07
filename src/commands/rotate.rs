// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::mac::generator::{self, GenerateOptions};
use crate::mac::{Mac, arp};
use crate::nm::{self, DeviceInfo, DeviceKind};
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
struct RotateReport {
    rotated: Vec<RotatedEntry>,
    skipped: Vec<SkippedEntry>,
}

#[derive(Debug, Serialize)]
struct RotatedEntry {
    iface: String,
    previous: Option<String>,
    new: String,
    connection: Option<String>,
    /// How many NM profiles bound to this device were rewritten. Always
    /// equals `profiles_total` on a clean run — strictly less if a single
    /// profile failed to update (issue #122).
    profiles_updated: usize,
    profiles_total: usize,
}

#[derive(Debug, Serialize)]
struct SkippedEntry {
    iface: String,
    reason: String,
}

pub fn run(
    iface_filter: Option<&str>,
    _yes: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }

    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;
    let mut state = State::load_or_default(&state_path)?;

    let arp_macs = arp::read_arp_macs();
    let gateway_mac = arp::read_default_gateway_mac();
    let mut avoid: HashSet<Mac> = arp_macs;
    if let Some(gw) = gateway_mac {
        avoid.insert(gw);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result: Result<RotateReport> = rt.block_on(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        let devices = nm::list_devices(&conn).await?;
        rotate_devices(&conn, devices, iface_filter, &config, &avoid, &mut state).await
    });

    let report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("proteus: rotate failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    if report.rotated.is_empty() && report.skipped.is_empty() {
        eprintln!("proteus: no NetworkManager-managed interfaces matched");
        return Ok(exit::GENERIC_ERROR);
    }

    print_report(&report);
    Ok(exit::SUCCESS)
}

async fn rotate_devices(
    conn: &zbus::Connection,
    devices: Vec<DeviceInfo>,
    iface_filter: Option<&str>,
    config: &Config,
    avoid: &HashSet<Mac>,
    state: &mut State,
) -> Result<RotateReport> {
    let mut report = RotateReport {
        rotated: Vec::new(),
        skipped: Vec::new(),
    };
    for dev in devices {
        if let Some(f) = iface_filter
            && dev.interface != f
        {
            continue;
        }
        if !matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) {
            if iface_filter.is_some() {
                report.skipped.push(SkippedEntry {
                    iface: dev.interface.clone(),
                    reason: format!("device kind {:?} not supported", dev.kind),
                });
            }
            continue;
        }
        if !dev.managed && iface_filter.is_none() {
            // Quietly skip when iterating all devices.
            continue;
        }
        if let Some(rec) = state.managed.interfaces.get(&dev.interface)
            && let Some(pin) = &rec.pinned
        {
            report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("pinned to {pin}"),
            });
            continue;
        }
        match rotate_one(conn, &dev, config, avoid, state).await {
            Ok(entry) => report.rotated.push(entry),
            Err(e) => report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    Ok(report)
}

async fn rotate_one(
    conn: &zbus::Connection,
    dev: &DeviceInfo,
    config: &Config,
    avoid: &HashSet<Mac>,
    state: &mut State,
) -> Result<RotatedEntry> {
    if dev.connections.is_empty() {
        return Err(anyhow!("no NM connection profile available"));
    }

    capture_original_mac(state, &dev.interface, dev.hw_address.as_deref());

    let forbidden = build_forbidden(state, dev.hw_address.as_deref());
    let opts = GenerateOptions {
        pool: &config.mac.oui_pool,
        forbidden: &forbidden,
        avoid,
    };
    let new_mac = generator::generate(&opts)?;

    // Issue #122: walk ALL profiles bound to this device. The first-only
    // shortcut left secondary SSIDs/profiles holding the previous MAC, so
    // joining them later leaked the prior identity. Errors on a single
    // profile are recorded but never abort the rest.
    let mut connection_label: Option<String> = None;
    let mut updated_uuids: Vec<String> = Vec::new();
    let mut updated_count: usize = 0;
    let mut update_errors: Vec<String> = Vec::new();
    for path in &dev.connections {
        // Single GetSettings + Update round trip per profile — each
        // accessor call would otherwise re-fetch the whole dict.
        match nm::apply::set_cloned_mac_returning_ids(conn, path, dev.kind, new_mac).await {
            Ok((id, uuid)) => {
                if connection_label.is_none() {
                    connection_label = id;
                }
                if let Some(u) = uuid {
                    updated_uuids.push(u);
                }
                updated_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    "rotate: failed to set MAC on profile for {}: {e:#}",
                    dev.interface
                );
                update_errors.push(format!("{e:#}"));
            }
        }
    }
    if updated_count == 0 {
        return Err(anyhow!(
            "no profiles updated for {} ({} attempt(s) failed: {})",
            dev.interface,
            dev.connections.len(),
            update_errors.join("; "),
        ));
    }

    let previous = record_rotation(
        state,
        &dev.interface,
        dev.hw_address.as_deref(),
        new_mac,
        &updated_uuids,
        &super::now_iso8601(),
    );

    Ok(RotatedEntry {
        iface: dev.interface.clone(),
        previous,
        new: new_mac.to_string(),
        connection: connection_label,
        profiles_updated: updated_count,
        profiles_total: dev.connections.len(),
    })
}

/// Bump per-interface and per-profile rotation counters. Issue #122
/// requires every profile bound to the device land under its own
/// `state.managed.connections` entry — keyed by uuid (issue #124).
/// Returns the previous MAC so the caller can show before/after in output.
fn record_rotation(
    state: &mut State,
    iface: &str,
    hw_address: Option<&str>,
    new_mac: Mac,
    updated_uuids: &[String],
    now_iso: &str,
) -> Option<String> {
    let rec = state
        .managed
        .interfaces
        .entry(iface.to_string())
        .or_default();
    let previous = rec
        .current_mac
        .clone()
        .or_else(|| hw_address.map(str::to_string));
    rec.current_mac = Some(new_mac.to_string());
    rec.last_rotated = Some(now_iso.to_string());
    rec.rotation_count += 1;

    for uuid in updated_uuids {
        let crec = state.managed.connections.entry(uuid.clone()).or_default();
        crec.current_mac = Some(new_mac.to_string());
        crec.last_rotated = Some(now_iso.to_string());
        crec.rotation_count += 1;
    }
    previous
}

fn capture_original_mac(state: &mut State, iface: &str, hw: Option<&str>) {
    if state.original_macs.contains_key(iface) {
        return;
    }
    if let Some(mac) = hw {
        state
            .original_macs
            .insert(iface.to_string(), mac.to_string());
    }
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(super::now_iso8601());
    }
}

fn build_forbidden(state: &State, hw: Option<&str>) -> HashSet<Mac> {
    let mut set = HashSet::new();
    for mac_str in state.original_macs.values() {
        if let Ok(m) = mac_str.parse::<Mac>() {
            set.insert(m);
        }
    }
    if let Some(h) = hw
        && let Ok(m) = h.parse::<Mac>()
    {
        set.insert(m);
    }
    for rec in state.managed.interfaces.values() {
        if let Some(m) = rec.current_mac.as_ref().and_then(|s| s.parse::<Mac>().ok()) {
            set.insert(m);
        }
    }
    set
}

fn print_report(report: &RotateReport) {
    for r in &report.rotated {
        let prev = r.previous.as_deref().unwrap_or("?");
        let profile_label = if r.profiles_total > 1 {
            format!(" [{}/{} profiles]", r.profiles_updated, r.profiles_total)
        } else {
            String::new()
        };
        match &r.connection {
            Some(id) => println!(
                "rotated {} ({}): {} -> {}{}",
                r.iface, id, prev, r.new, profile_label
            ),
            None => println!(
                "rotated {}: {} -> {}{}",
                r.iface, prev, r.new, profile_label
            ),
        }
    }
    for s in &report.skipped {
        println!("skipped {}: {}", s.iface, s.reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(s: &str) -> Mac {
        s.parse::<Mac>().expect("valid MAC")
    }

    #[test]
    fn record_rotation_walks_every_profile_uuid() {
        // Issue #122: a device may carry multiple NM connection profiles
        // (e.g. two Wi-Fi SSIDs). The pre-fix code only updated the first;
        // every secondary profile kept the old MAC, leaking on next join.
        // record_rotation is the per-rotate state-update primitive — it
        // must touch every uuid it's handed.
        let mut state = State::default();
        let new = mac("aa:bb:cc:dd:ee:ff");
        let uuids: Vec<String> = vec![
            "11111111-1111-1111-1111-111111111111".to_string(),
            "22222222-2222-2222-2222-222222222222".to_string(),
            "33333333-3333-3333-3333-333333333333".to_string(),
        ];
        let prev = record_rotation(
            &mut state,
            "wlan0",
            Some("00:11:22:33:44:55"),
            new,
            &uuids,
            "2026-05-07T00:00:00Z",
        );
        assert_eq!(prev.as_deref(), Some("00:11:22:33:44:55"));
        // Per-iface row must reflect the rotation.
        let iface_rec = state
            .managed
            .interfaces
            .get("wlan0")
            .expect("iface row created");
        assert_eq!(iface_rec.current_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(iface_rec.rotation_count, 1);
        // Every uuid landed under its own state slot — and crucially, all
        // three carry the new MAC. Pre-fix this would have been one entry.
        assert_eq!(
            state.managed.connections.len(),
            uuids.len(),
            "every profile uuid must own a state entry (issue #122)"
        );
        for uuid in &uuids {
            let rec = state
                .managed
                .connections
                .get(uuid)
                .expect("uuid present in state");
            assert_eq!(
                rec.current_mac.as_deref(),
                Some("aa:bb:cc:dd:ee:ff"),
                "secondary profile {uuid} kept stale MAC"
            );
            assert_eq!(rec.rotation_count, 1);
        }
    }

    #[test]
    fn record_rotation_skips_profiles_with_no_uuid() {
        // Transient/in-memory NM profiles can be uuid-less. We deliberately
        // don't record those — they'd collide on the next apply if multiple
        // shared a name. Uuid-bearing siblings still go in.
        let mut state = State::default();
        let new = mac("aa:bb:cc:dd:ee:ff");
        let uuids = vec!["44444444-4444-4444-4444-444444444444".to_string()];
        record_rotation(
            &mut state,
            "wlan0",
            None,
            new,
            &uuids,
            "2026-05-07T00:00:00Z",
        );
        assert_eq!(state.managed.connections.len(), 1);
    }
}
