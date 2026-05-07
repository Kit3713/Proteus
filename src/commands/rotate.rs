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
        rotate_devices(
            &conn,
            devices,
            iface_filter,
            &config,
            &avoid,
            &mut state,
            &state_path,
        )
        .await
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
    state_path: &Path,
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
        match rotate_one(conn, &dev, config, avoid, state, state_path).await {
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
    state_path: &Path,
) -> Result<RotatedEntry> {
    let connection_path = dev
        .connections
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("no NM connection profile available"))?;

    // Capture-then-save-then-mutate: the original factory MAC must be
    // durable on disk BEFORE we ask NetworkManager to set a cloned MAC.
    // Otherwise a crash between the DBus write and the final state.save()
    // at the end of `run` would lose the factory MAC and turn `revert` into
    // a no-op (sacred-originals invariant; issue #119).
    capture_original_mac(state, &dev.interface, dev.hw_address.as_deref());
    persist_capture_metadata(state);
    state.save(state_path)?;

    let forbidden = build_forbidden(state, dev.hw_address.as_deref());
    let opts = GenerateOptions {
        pool: &config.mac.oui_pool,
        forbidden: &forbidden,
        avoid,
    };
    let new_mac = generator::generate(&opts)?;
    let connection_id = nm::apply::read_connection_id(conn, &connection_path)
        .await
        .ok()
        .flatten();
    nm::apply::set_cloned_mac(conn, &connection_path, dev.kind, new_mac).await?;

    let rec = state
        .managed
        .interfaces
        .entry(dev.interface.clone())
        .or_default();
    let previous = rec.current_mac.clone().or_else(|| dev.hw_address.clone());
    rec.current_mac = Some(new_mac.to_string());
    rec.last_rotated = Some(super::now_iso8601());
    rec.rotation_count += 1;

    let last_rotated = rec.last_rotated.clone();
    if let Some(id) = &connection_id {
        let crec = state.managed.connections.entry(id.clone()).or_default();
        crec.current_mac = Some(new_mac.to_string());
        crec.last_rotated = last_rotated;
        crec.rotation_count += 1;
    }

    Ok(RotatedEntry {
        iface: dev.interface.clone(),
        previous,
        new: new_mac.to_string(),
        connection: connection_id,
    })
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
        match &r.connection {
            Some(id) => println!("rotated {} ({}): {} -> {}", r.iface, id, prev, r.new),
            None => println!("rotated {}: {} -> {}", r.iface, prev, r.new),
        }
    }
    for s in &report.skipped {
        println!("skipped {}: {}", s.iface, s.reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #119 — sacred-originals invariant. `rotate_one` now saves
    /// state.json AFTER `capture_original_mac` and BEFORE the
    /// `nm::set_cloned_mac` DBus write. This test pins the round-trip half:
    /// a captured factory MAC must survive a crash between save and the
    /// DBus mutation so revert can restore it.
    #[test]
    fn captured_factory_mac_persists_to_disk() {
        let dir = crate::testing::TempRoot::new("rotate");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        capture_original_mac(&mut state, "wlan0", Some("aa:bb:cc:dd:ee:ff"));
        capture_original_mac(&mut state, "eth0", Some("11:22:33:44:55:66"));
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "wlan0 factory MAC must be on disk before any DBus mutation"
        );
        assert_eq!(
            loaded.original_macs.get("eth0").map(String::as_str),
            Some("11:22:33:44:55:66")
        );
        assert!(loaded.captured_at.is_some());
    }

    /// `capture_original_mac` is capture-once: a second call with a
    /// different MAC must not clobber the first capture.
    #[test]
    fn capture_original_mac_is_idempotent() {
        let mut state = State::default();
        capture_original_mac(&mut state, "wlan0", Some("aa:bb:cc:dd:ee:ff"));
        capture_original_mac(&mut state, "wlan0", Some("00:00:00:00:00:00"));
        assert_eq!(
            state.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "second capture must not overwrite (sacred-originals)"
        );
    }
}
