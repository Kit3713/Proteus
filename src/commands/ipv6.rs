// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus ipv6` — status / apply / revert for the IPv6 fingerprint pass.
//!
//! `status` is read-only and works as any user; `apply` and `revert` need
//! root + `--yes`. The flow mirrors `proteus bluetooth` and `proteus
//! hostname`: classify per managed interface, push live values through the
//! `ipv6` module helpers, persist originals into `state.json` once.
//!
//! Only ethernet + wifi are managed; loopback and virtual interfaces are
//! skipped silently. NM connection updates are best-effort — if NM isn't on
//! the bus we still write the sysctl drop-in and surface the NM piece as a
//! per-iface skip so the operator can see what landed.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::commands::status as status_cmd;
use crate::config::Config;
use crate::exit;
use crate::ipv6::{self, InterfaceSnapshot};
use crate::nm::{self, DeviceInfo, DeviceKind};
use crate::state::{Ipv6Originals, State};
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    enabled: bool,
    addr_gen_mode: String,
    use_temp_addresses: bool,
    ndp_hardening: bool,
    drop_in_path: String,
    drop_in_present: bool,
    drop_in_managed_by_proteus: bool,
    interfaces: Vec<InterfaceStatus>,
}

#[derive(Debug, Serialize)]
struct InterfaceStatus {
    iface: String,
    kind: String,
    use_tempaddr: Option<String>,
    addr_gen_mode: Option<String>,
    temp_valid_lft: Option<String>,
    temp_prefered_lft: Option<String>,
    privacy_mode: String,
    originals_cached: bool,
    nm: Option<NmStatus>,
}

#[derive(Debug, Serialize)]
struct NmStatus {
    connection: Option<String>,
    addr_gen_mode: Option<String>,
    dhcp_duid: Option<String>,
    dhcp_iaid: Option<String>,
}

pub fn status(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let state = State::load_or_default(&state_path).unwrap_or_default();

    let drop_in = std::fs::read_to_string(ipv6::DROPIN_PATH).ok();
    let drop_in_present = drop_in.is_some();
    let drop_in_managed_by_proteus = drop_in
        .as_deref()
        .map(|s| s.starts_with("# managed by proteus"))
        .unwrap_or(false);

    let ifaces = managed_iface_names();
    let names: Vec<String> = ifaces.iter().map(|(n, _)| n.clone()).collect();
    let nm_data = with_nm(|conn, devices| async move {
        let mut out: BTreeMap<String, NmIfaceData> = BTreeMap::new();
        for iface in &names {
            if let Some(data) = read_nm_iface(&conn, &devices, iface).await {
                out.insert(iface.clone(), data);
            }
        }
        out
    })
    .unwrap_or_default();

    let mut interface_reports = Vec::with_capacity(ifaces.len());
    for (iface, kind) in &ifaces {
        let snap = ipv6::read_snapshot(None, iface);
        let nm_block = nm_data.get(iface).map(|n| NmStatus {
            connection: n.connection.clone(),
            addr_gen_mode: n.snapshot.addr_gen_mode.clone(),
            dhcp_duid: n.snapshot.dhcp_duid.clone(),
            dhcp_iaid: n.snapshot.dhcp_iaid.clone(),
        });
        interface_reports.push(InterfaceStatus {
            iface: iface.clone(),
            kind: kind.clone(),
            privacy_mode: classify_privacy(&snap),
            use_tempaddr: snap.use_tempaddr,
            addr_gen_mode: snap.addr_gen_mode,
            temp_valid_lft: snap.temp_valid_lft,
            temp_prefered_lft: snap.temp_prefered_lft,
            originals_cached: state.originals.ipv6.contains_key(iface),
            nm: nm_block,
        });
    }

    let report = StatusReport {
        enabled: config.ipv6.enabled,
        addr_gen_mode: config.ipv6.addr_gen_mode.clone(),
        use_temp_addresses: config.ipv6.use_temp_addresses,
        ndp_hardening: config.ipv6.ndp_hardening,
        drop_in_path: ipv6::DROPIN_PATH.into(),
        drop_in_present,
        drop_in_managed_by_proteus,
        interfaces: interface_reports,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_status(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(yes: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    // Root check before --yes so non-root invocations land on PERMISSION_ERROR
    // (66), matching what `proteus help` documents and what wrappers grep for.
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !yes {
        eprintln!("proteus: 'ipv6 apply' is mutating; pass --yes (see `proteus help ipv6`)");
        return Ok(exit::NOT_IMPLEMENTED);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;

    if !config.ipv6.enabled {
        println!("ipv6: disabled in config (ipv6.enabled = false)");
        return Ok(exit::SUCCESS);
    }

    let mut state = State::load_or_default(&state_path)?;
    let ifaces = managed_iface_names();
    if ifaces.is_empty() {
        println!("ipv6: no managed interfaces detected");
        return Ok(exit::SUCCESS);
    }

    // Capture originals BEFORE writing live values so revert can undo cleanly
    // even if the apply is interrupted between sysctl writes.
    for (iface, _) in &ifaces {
        capture_originals(&mut state, iface);
    }

    let names: Vec<&str> = ifaces.iter().map(|(n, _)| n.as_str()).collect();
    let body = ipv6::render_dropin(&names);
    if let Err(e) = super::write_atomic(Path::new(ipv6::DROPIN_PATH), body.as_bytes()) {
        eprintln!("proteus: ipv6 apply: writing drop-in failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }

    let mut applied: BTreeMap<String, ApplyEntry> = ifaces
        .iter()
        .map(|(iface, _)| {
            let mut entry = ApplyEntry {
                iface: iface.clone(),
                sysctls_written: 0,
                nm: NmApplyOutcome::Skipped("DBus unavailable".into()),
            };
            for s in ipv6::SYSCTLS {
                match ipv6::write_sysctl(None, iface, s.key, s.value) {
                    Ok(()) => entry.sysctls_written += 1,
                    Err(e) => {
                        tracing::warn!("ipv6: write {}::{} failed: {e:#}", iface, s.key);
                    }
                }
            }
            (iface.clone(), entry)
        })
        .collect();

    // Re-read the drop-in via `sysctl --system` so `all`/`default` siblings
    // pick up matching defaults set by other admins.
    if let Err(e) = ipv6::reload_sysctls() {
        tracing::warn!("ipv6: sysctl --system failed: {e:#}");
    }

    if let Some(results) = with_nm(|conn, devices| async move {
        let mut out: Vec<(String, NmApplyOutcome)> = Vec::with_capacity(names.len());
        for iface in &names {
            out.push((
                (*iface).to_string(),
                apply_nm_one(&conn, &devices, iface).await,
            ));
        }
        out
    }) {
        for (iface, outcome) in results {
            if let Some(slot) = applied.get_mut(&iface) {
                slot.nm = outcome;
            }
        }
    }

    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    print_apply(applied.values());
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !yes {
        eprintln!("proteus: 'ipv6 revert' is mutating; pass --yes (see `proteus help ipv6`)");
        return Ok(exit::NOT_IMPLEMENTED);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if state.originals.ipv6.is_empty() {
        // Drop-in might still be on disk from an earlier version; clear it
        // either way so revert is idempotent.
        let _ = remove_dropin();
        let _ = ipv6::reload_sysctls();
        println!("ipv6: no originals cached, drop-in removed (if any)");
        return Ok(exit::SUCCESS);
    }

    let originals: Vec<(String, Ipv6Originals)> = state
        .originals
        .ipv6
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let entries: Vec<RevertEntry> = originals
        .iter()
        .map(|(iface, orig)| revert_one(iface, orig))
        .collect();

    let removed = remove_dropin();
    let _ = ipv6::reload_sysctls();
    state.originals.ipv6.clear();
    state.save(&state_path)?;

    print_revert(&entries, removed);
    Ok(exit::SUCCESS)
}

fn revert_one(iface: &str, orig: &Ipv6Originals) -> RevertEntry {
    let mut entry = RevertEntry {
        iface: iface.to_string(),
        restored: 0,
        missing: 0,
    };
    for s in ipv6::SYSCTLS {
        match original_for_key(orig, s.key) {
            Some(val) => match ipv6::write_sysctl(None, iface, s.key, val) {
                Ok(()) => entry.restored += 1,
                Err(e) => {
                    tracing::warn!("ipv6 revert: write {}::{} failed: {e:#}", iface, s.key);
                }
            },
            None => entry.missing += 1,
        }
    }
    entry
}

#[derive(Debug)]
struct ApplyEntry {
    iface: String,
    sysctls_written: usize,
    nm: NmApplyOutcome,
}

#[derive(Debug, Clone)]
enum NmApplyOutcome {
    Applied { connection: Option<String> },
    Skipped(String),
}

#[derive(Debug)]
struct RevertEntry {
    iface: String,
    restored: usize,
    missing: usize,
}

#[derive(Debug, Clone)]
struct NmIfaceData {
    connection: Option<String>,
    snapshot: crate::ipv6::nm::Ipv6Snapshot,
}

/// Run `body` on the system bus with the NM device list pre-fetched. Returns
/// `None` if the runtime, the bus, or the NM device list is unavailable —
/// callers treat that as "skip the NM-side work" rather than fail the apply.
fn with_nm<F, Fut, T>(body: F) -> Option<T>
where
    F: FnOnce(zbus::Connection, Vec<DeviceInfo>) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")
        .ok()?;
    runtime.block_on(async move {
        let conn = zbus::Connection::system().await.ok()?;
        let devices = nm::list_devices(&conn).await.ok()?;
        Some(body(conn, devices).await)
    })
}

async fn read_nm_iface(
    conn: &zbus::Connection,
    devices: &[DeviceInfo],
    iface: &str,
) -> Option<NmIfaceData> {
    let dev = devices.iter().find(|d| d.interface == iface)?;
    if !matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) {
        return None;
    }
    let path = dev.connections.first().cloned()?;
    let connection = nm::apply::read_connection_id(conn, &path)
        .await
        .ok()
        .flatten();
    let snapshot = crate::ipv6::nm::read_settings(conn, &path)
        .await
        .unwrap_or_default();
    Some(NmIfaceData {
        connection,
        snapshot,
    })
}

async fn apply_nm_one(
    conn: &zbus::Connection,
    devices: &[DeviceInfo],
    iface: &str,
) -> NmApplyOutcome {
    let Some(dev) = devices.iter().find(|d| d.interface == iface) else {
        return NmApplyOutcome::Skipped("no NM device".into());
    };
    if !matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) {
        return NmApplyOutcome::Skipped(format!("unsupported kind {:?}", dev.kind));
    }
    let Some(path) = dev.connections.first().cloned() else {
        return NmApplyOutcome::Skipped("no connection profile".into());
    };
    let connection = nm::apply::read_connection_id(conn, &path)
        .await
        .ok()
        .flatten();
    match crate::ipv6::nm::apply_settings(conn, &path, &Default::default()).await {
        Ok(()) => NmApplyOutcome::Applied { connection },
        Err(e) => NmApplyOutcome::Skipped(format!("update failed: {e:#}")),
    }
}

fn managed_iface_names() -> Vec<(String, String)> {
    status_cmd::enumerate_interfaces()
        .into_iter()
        .filter_map(|i| match i.kind.as_str() {
            "wifi" | "ethernet" => Some((i.name, i.kind)),
            _ => None,
        })
        .collect()
}

fn capture_originals(state: &mut State, iface: &str) {
    if state.originals.ipv6.contains_key(iface) {
        return;
    }
    let snap = ipv6::read_snapshot(None, iface);
    state.originals.ipv6.insert(
        iface.to_string(),
        Ipv6Originals {
            use_tempaddr: snap.use_tempaddr,
            addr_gen_mode: snap.addr_gen_mode,
            temp_valid_lft: snap.temp_valid_lft,
            temp_prefered_lft: snap.temp_prefered_lft,
        },
    );
}

fn original_for_key<'a>(orig: &'a Ipv6Originals, key: &str) -> Option<&'a str> {
    match key {
        "use_tempaddr" => orig.use_tempaddr.as_deref(),
        "addr_gen_mode" => orig.addr_gen_mode.as_deref(),
        "temp_valid_lft" => orig.temp_valid_lft.as_deref(),
        "temp_prefered_lft" => orig.temp_prefered_lft.as_deref(),
        _ => None,
    }
}

fn remove_dropin() -> bool {
    match std::fs::remove_file(ipv6::DROPIN_PATH) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::warn!("ipv6 revert: removing drop-in failed: {e:#}");
            false
        }
    }
}

fn classify_privacy(snap: &InterfaceSnapshot) -> String {
    let temp = snap.use_tempaddr.as_deref();
    let mode = snap.addr_gen_mode.as_deref();
    match (mode, temp) {
        (Some("3"), Some(t)) if t == "2" || t == "1" => "stable-privacy + temp".into(),
        (Some("3"), _) => "stable-privacy".into(),
        (Some("0"), _) => "eui64 (LEAK)".into(),
        (Some("1"), _) => "no-iid (link-local only without stable_secret)".into(),
        (Some(other), _) => format!("addr_gen_mode={other}"),
        (None, _) => "unknown".into(),
    }
}

fn print_status(r: &StatusReport) {
    println!("ipv6:");
    println!("  enabled:                {}", yesno(r.enabled));
    println!("  addr_gen_mode:          {}", r.addr_gen_mode);
    println!("  use_temp_addresses:     {}", yesno(r.use_temp_addresses));
    println!("  ndp_hardening:          {}", yesno(r.ndp_hardening));
    println!("drop-in:");
    println!("  path:                   {}", r.drop_in_path);
    println!("  present:                {}", yesno(r.drop_in_present));
    println!(
        "  managed by proteus:     {}",
        yesno(r.drop_in_managed_by_proteus)
    );
    println!("interfaces:");
    if r.interfaces.is_empty() {
        println!("  (none managed)");
        return;
    }
    for i in &r.interfaces {
        println!("  {} ({}): {}", i.iface, i.kind, i.privacy_mode);
        println!(
            "    use_tempaddr={} addr_gen_mode={} temp_valid_lft={} temp_prefered_lft={}",
            i.use_tempaddr.as_deref().unwrap_or("?"),
            i.addr_gen_mode.as_deref().unwrap_or("?"),
            i.temp_valid_lft.as_deref().unwrap_or("?"),
            i.temp_prefered_lft.as_deref().unwrap_or("?"),
        );
        println!("    originals cached:     {}", yesno(i.originals_cached));
        match &i.nm {
            Some(nm) => {
                println!(
                    "    NM connection:        {}",
                    nm.connection.as_deref().unwrap_or("(unset)")
                );
                println!(
                    "    NM addr-gen-mode:     {}",
                    nm.addr_gen_mode.as_deref().unwrap_or("(unset)")
                );
                println!(
                    "    NM dhcp-duid:         {}",
                    nm.dhcp_duid.as_deref().unwrap_or("(unset)")
                );
                println!(
                    "    NM dhcp-iaid:         {}",
                    nm.dhcp_iaid.as_deref().unwrap_or("(unset)")
                );
            }
            None => println!("    NM connection:        (no NM data)"),
        }
    }
}

fn print_apply<'a>(entries: impl IntoIterator<Item = &'a ApplyEntry>) {
    println!("ipv6 apply: drop-in {}", ipv6::DROPIN_PATH);
    let total = ipv6::SYSCTLS.len();
    for e in entries {
        let nm_label = match &e.nm {
            NmApplyOutcome::Applied {
                connection: Some(c),
            } => format!("nm=applied ({c})"),
            NmApplyOutcome::Applied { connection: None } => "nm=applied".into(),
            NmApplyOutcome::Skipped(reason) => format!("nm=skipped ({reason})"),
        };
        println!(
            "  {}: sysctls {}/{} {}",
            e.iface, e.sysctls_written, total, nm_label
        );
    }
}

fn print_revert(entries: &[RevertEntry], removed: bool) {
    println!(
        "ipv6 revert: drop-in {}",
        if removed { "removed" } else { "absent" }
    );
    for e in entries {
        let suffix = if e.missing > 0 {
            format!(" ({} unset, kernel default kept)", e.missing)
        } else {
            String::new()
        };
        println!("  {}: restored {} sysctl(s){suffix}", e.iface, e.restored);
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

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_privacy_recognises_known_modes() {
        let mut snap = InterfaceSnapshot {
            iface: "x".into(),
            addr_gen_mode: Some("3".into()),
            use_tempaddr: Some("2".into()),
            temp_valid_lft: None,
            temp_prefered_lft: None,
        };
        assert_eq!(classify_privacy(&snap), "stable-privacy + temp");

        snap.use_tempaddr = Some("0".into());
        assert_eq!(classify_privacy(&snap), "stable-privacy");

        snap.addr_gen_mode = Some("0".into());
        assert!(classify_privacy(&snap).contains("LEAK"));

        snap.addr_gen_mode = None;
        assert_eq!(classify_privacy(&snap), "unknown");
    }

    #[test]
    fn original_for_key_dispatches_correctly() {
        let orig = Ipv6Originals {
            use_tempaddr: Some("2".into()),
            addr_gen_mode: None,
            temp_valid_lft: Some("604800".into()),
            temp_prefered_lft: None,
        };
        assert_eq!(original_for_key(&orig, "use_tempaddr"), Some("2"));
        assert_eq!(original_for_key(&orig, "addr_gen_mode"), None);
        assert_eq!(original_for_key(&orig, "temp_valid_lft"), Some("604800"));
        assert_eq!(original_for_key(&orig, "nope"), None);
    }

    #[test]
    fn capture_records_first_snapshot_and_skips_thereafter() {
        let mut state = State::default();
        capture_originals(&mut state, "fake-iface-for-test");
        assert!(state.originals.ipv6.contains_key("fake-iface-for-test"));
        // Inject a sentinel and confirm the second call leaves it alone.
        state
            .originals
            .ipv6
            .get_mut("fake-iface-for-test")
            .unwrap()
            .use_tempaddr = Some("sentinel".into());
        capture_originals(&mut state, "fake-iface-for-test");
        assert_eq!(
            state.originals.ipv6["fake-iface-for-test"]
                .use_tempaddr
                .as_deref(),
            Some("sentinel")
        );
    }

    #[test]
    fn sysctls_table_covers_all_managed_keys() {
        let keys: Vec<&str> = ipv6::SYSCTLS.iter().map(|s| s.key).collect();
        assert!(keys.contains(&"use_tempaddr"));
        assert!(keys.contains(&"addr_gen_mode"));
        assert!(keys.contains(&"temp_valid_lft"));
        assert!(keys.contains(&"temp_prefered_lft"));
        assert_eq!(keys.len(), 4);
    }
}
