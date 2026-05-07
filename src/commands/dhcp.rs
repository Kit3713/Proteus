// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus dhcp <status|apply|revert>` — DHCP option suppression via
//! per-NM-connection settings written over DBus.
//!
//! The wire-level intent is documented in `wiki/dhcp.md`. Mapping of toggles
//! to NM settings keys lives in `crate::nm::dhcp`. This module is the glue
//! between CLI args, config, NM DBus calls, and `state.json` originals.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::nm::{self, ConnectionSettings, DeviceKind, DeviceInfo, dhcp as nmdhcp};
use crate::state::{DhcpSettingsSnapshot, State};
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    network_manager: bool,
    connections: Vec<ConnectionStatus>,
}

#[derive(Debug, Serialize)]
struct ConnectionStatus {
    id: String,
    kind: String,
    proteus_managed: bool,
    suppression: SuppressionState,
    settings: SettingsView,
}

#[derive(Debug, Serialize)]
struct SuppressionState {
    hostname: String,
    vendor_class: String,
    client_id: String,
    duid: String,
}

#[derive(Debug, Serialize)]
struct SettingsView {
    ipv4_dhcp_send_hostname: Option<bool>,
    ipv4_dhcp_fqdn: Option<String>,
    ipv4_dhcp_vendor_class_identifier: Option<String>,
    ipv4_dhcp_client_id: Option<String>,
    ipv6_dhcp_duid: Option<String>,
    ipv6_dhcp_iaid: Option<String>,
}

pub fn status(json: bool) -> Result<u8> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let outcome = rt.block_on(async { gather_status().await });
    let report = match outcome {
        Ok(r) => r,
        Err(e) => {
            eprintln!("proteus: dhcp status failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if json {
        super::print_json(&report)?;
    } else {
        print_status(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;
    if !config.dhcp.enabled {
        println!("dhcp: disabled in config (dhcp.enabled = false)");
        return Ok(exit::SUCCESS);
    }
    let mut state = State::load_or_default(&state_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    // Pass `state_path` through so `do_apply` can save originals to disk
    // BEFORE each per-connection NM update (sacred-originals invariant;
    // issue #119).
    let result = rt.block_on(async { do_apply(&config, &mut state, &state_path).await });
    let outcomes = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("proteus: dhcp apply failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    persist_capture_metadata(&mut state);
    state.save(&state_path)?;
    print_apply(&outcomes);
    Ok(exit::SUCCESS)
}

pub fn revert(state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result = rt.block_on(async { do_revert(&mut state).await });
    let outcomes = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("proteus: dhcp revert failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    state.save(&state_path)?;
    print_revert(&outcomes);
    Ok(exit::SUCCESS)
}

/// `proteus dhcp renew` — release + renew the DHCP lease without touching
/// the cloned MAC. Roadmap Milestone 4c.
///
/// Mechanic per device:
/// 1. Locate the NM device (by `iface`, or all managed wifi/ethernet).
/// 2. Skip devices with no active connection (a renew is meaningless).
/// 3. Try `Device.Reapply` (cheap, link-up). On failure, fall back to
///    `Device.Disconnect` + `NetworkManager.ActivateConnection`.
/// 4. Print one `renewed <iface>` line per device. Lease numbers aren't
///    sourced from NM (no monotonic counter exposed), so the report
///    uses a placeholder — the IP-on-the-wire is the verifiable signal.
pub fn renew(iface: Option<&str>, yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'dhcp renew' is mutating (cycles the active connection on each iface)",
        "proteus help dhcp",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #126: serialize concurrent mutators on <state-dir>/.lock.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result: Result<Vec<RenewOutcome>> = rt.block_on(async { do_renew(iface).await });

    let outcomes = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("proteus: dhcp renew failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    if outcomes.is_empty() {
        // Match the rotate.rs idiom: no matching iface is a hard error so
        // wrappers can distinguish "nothing to do" from "all good".
        match iface {
            Some(name) => {
                eprintln!(
                    "proteus: no NetworkManager-managed device for interface '{name}'"
                );
            }
            None => {
                eprintln!(
                    "proteus: no NetworkManager-managed wifi/ethernet interfaces found"
                );
            }
        }
        return Ok(exit::GENERIC_ERROR);
    }

    print_renew(&outcomes);
    Ok(exit::SUCCESS)
}

#[derive(Debug, Clone, Serialize)]
struct RenewOutcome {
    iface: String,
    method: String,
    note: Option<String>,
}

async fn do_renew(iface_filter: Option<&str>) -> Result<Vec<RenewOutcome>> {
    let conn = zbus::Connection::system()
        .await
        .context("connecting to system DBus (NetworkManager required)")?;
    let devices = nm::list_devices(&conn).await?;
    let mut out = Vec::new();
    for dev in devices {
        if !device_matches(&dev, iface_filter) {
            continue;
        }
        let entry = match nmdhcp::renew_lease(&conn, &dev.path).await {
            Ok(nmdhcp::RenewOutcome::Reapplied) => RenewOutcome {
                iface: dev.interface.clone(),
                method: "reapply".into(),
                note: None,
            },
            Ok(nmdhcp::RenewOutcome::DisconnectActivated) => RenewOutcome {
                iface: dev.interface.clone(),
                method: "disconnect+activate".into(),
                note: Some("Reapply unsupported; cycled connection".into()),
            },
            Ok(nmdhcp::RenewOutcome::NoActiveConnection) => RenewOutcome {
                iface: dev.interface.clone(),
                method: "skipped".into(),
                note: Some("no active connection".into()),
            },
            Err(e) => RenewOutcome {
                iface: dev.interface.clone(),
                method: "failed".into(),
                note: Some(format!("{e:#}")),
            },
        };
        out.push(entry);
    }
    out.sort_by(|a, b| a.iface.cmp(&b.iface));
    Ok(out)
}

/// Filter rule: when `iface_filter` is set, only that exact interface
/// (regardless of kind/managed state — gives the user a way to surface
/// a "no such NM device" error). When unset, every managed wifi/ethernet
/// device matches.
fn device_matches(dev: &DeviceInfo, iface_filter: Option<&str>) -> bool {
    if let Some(name) = iface_filter {
        return dev.interface == name;
    }
    matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) && dev.managed
}

fn print_renew(outcomes: &[RenewOutcome]) {
    for o in outcomes {
        match (o.method.as_str(), &o.note) {
            ("reapply", _) => {
                // Lease number is a placeholder; NM doesn't expose a
                // monotonic counter and the IP-on-the-wire is the
                // observable signal.
                println!("renewed {}: lease N -> lease N+1 (reapply)", o.iface);
            }
            ("disconnect+activate", Some(n)) => {
                println!(
                    "renewed {}: lease N -> lease N+1 (disconnect+activate; {n})",
                    o.iface
                );
            }
            ("skipped", Some(n)) => {
                println!("skipped {}: {n}", o.iface);
            }
            ("failed", Some(n)) => {
                println!("failed  {}: {n}", o.iface);
            }
            _ => {
                println!("renewed {}", o.iface);
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct ApplyOutcome {
    id: String,
    kind: String,
    changed: bool,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct RevertOutcome {
    id: String,
    restored: bool,
    note: Option<String>,
}

async fn gather_status() -> Result<StatusReport> {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(_) => {
            return Ok(StatusReport {
                network_manager: false,
                connections: Vec::new(),
            });
        }
    };
    let settings_proxy = match nm::SettingsProxy::new(&conn).await {
        Ok(p) => p,
        Err(_) => {
            return Ok(StatusReport {
                network_manager: false,
                connections: Vec::new(),
            });
        }
    };
    let paths = settings_proxy.list_connections().await.unwrap_or_default();
    let mut entries = Vec::new();
    for path in paths {
        let settings = match nmdhcp::get_settings(&conn, &path).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let Some(id) = nmdhcp::connection_id(&settings) else {
            continue;
        };
        let kind = nmdhcp::connection_kind(&settings);
        let snap = nmdhcp::snapshot_dhcp(&settings);
        let proteus_managed = nmdhcp::is_proteus_managed(&settings);
        entries.push(ConnectionStatus {
            id,
            kind,
            proteus_managed,
            suppression: classify_suppression(&snap),
            settings: SettingsView {
                ipv4_dhcp_send_hostname: snap.ipv4_dhcp_send_hostname,
                ipv4_dhcp_fqdn: snap.ipv4_dhcp_fqdn.clone(),
                ipv4_dhcp_vendor_class_identifier: snap.ipv4_dhcp_vendor_class_identifier.clone(),
                ipv4_dhcp_client_id: snap.ipv4_dhcp_client_id.clone(),
                ipv6_dhcp_duid: snap.ipv6_dhcp_duid.clone(),
                ipv6_dhcp_iaid: snap.ipv6_dhcp_iaid.clone(),
            },
        });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(StatusReport {
        network_manager: true,
        connections: entries,
    })
}

async fn do_apply(
    config: &Config,
    state: &mut State,
    state_path: &Path,
) -> Result<Vec<ApplyOutcome>> {
    let conn = zbus::Connection::system()
        .await
        .context("connecting to system DBus (NetworkManager required)")?;
    let settings_proxy = nm::SettingsProxy::new(&conn)
        .await
        .context("connecting to NetworkManager Settings")?;
    let paths = settings_proxy
        .list_connections()
        .await
        .context("listing NetworkManager connections")?;

    let applied_at = super::now_iso8601();
    let mut outcomes = Vec::new();
    for path in paths {
        let settings = match nmdhcp::get_settings(&conn, &path).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(path = %path.as_str(), "skip get_settings: {e:#}");
                continue;
            }
        };
        let Some(id) = nmdhcp::connection_id(&settings) else {
            continue;
        };
        // Issue #124: state keys are NM uuids. id stays for display output.
        let Some(uuid) = nmdhcp::connection_uuid(&settings) else {
            tracing::debug!(id = %id, "skip: connection has no uuid");
            continue;
        };
        let kind = nmdhcp::connection_kind(&settings);
        if !is_managed_kind(&kind) {
            outcomes.push(ApplyOutcome {
                id,
                kind,
                changed: false,
                note: Some("skipped (unsupported connection type)".into()),
            });
            continue;
        }

        // Cache originals on first touch, then persist to disk BEFORE the
        // per-connection NM Update() so a crash between Update() and the
        // final state.save() can't leave a connection mutated with no
        // recorded original (sacred-originals invariant; issue #119).
        capture_originals(state, &uuid, &settings);
        persist_capture_metadata(state);
        state.save(state_path)?;

        let mut new_settings: ConnectionSettings = settings.clone();
        let changed_dhcp = nmdhcp::apply_dhcp_settings(
            &mut new_settings,
            config.dhcp.suppress_hostname,
            config.dhcp.suppress_vendor_class,
            config.dhcp.rotate_client_id,
        )?;
        nmdhcp::tag_user_data(&mut new_settings, version::VERSION, &applied_at)?;
        let was_managed = nmdhcp::is_proteus_managed(&settings);
        let changed = changed_dhcp || !was_managed;
        if changed {
            if let Err(e) = nmdhcp::update_connection(&conn, &path, new_settings).await {
                outcomes.push(ApplyOutcome {
                    id,
                    kind,
                    changed: false,
                    note: Some(format!("failed: {e:#}")),
                });
                continue;
            }
        }
        outcomes.push(ApplyOutcome {
            id,
            kind,
            changed,
            note: None,
        });
    }
    outcomes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(outcomes)
}

async fn do_revert(state: &mut State) -> Result<Vec<RevertOutcome>> {
    let conn = zbus::Connection::system()
        .await
        .context("connecting to system DBus (NetworkManager required)")?;
    let settings_proxy = nm::SettingsProxy::new(&conn)
        .await
        .context("connecting to NetworkManager Settings")?;
    let paths = settings_proxy
        .list_connections()
        .await
        .context("listing NetworkManager connections")?;

    let mut outcomes = Vec::new();
    let mut to_clear = Vec::new();
    for path in paths {
        let settings = match nmdhcp::get_settings(&conn, &path).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(path = %path.as_str(), "skip get_settings: {e:#}");
                continue;
            }
        };
        let Some(id) = nmdhcp::connection_id(&settings) else {
            continue;
        };
        // Issue #124: keyed by uuid; id stays for display.
        let Some(uuid) = nmdhcp::connection_uuid(&settings) else {
            continue;
        };
        if !nmdhcp::is_proteus_managed(&settings) {
            continue;
        }
        let snap = state
            .originals
            .connections
            .get(&uuid)
            .and_then(|c| c.dhcp_settings.clone());
        let mut new_settings: ConnectionSettings = settings.clone();
        match snap {
            Some(snap) => {
                nmdhcp::revert_dhcp_settings(&mut new_settings, &snap)?;
                nmdhcp::untag_user_data(&mut new_settings)?;
                if let Err(e) = nmdhcp::update_connection(&conn, &path, new_settings).await {
                    outcomes.push(RevertOutcome {
                        id,
                        restored: false,
                        note: Some(format!("failed: {e:#}")),
                    });
                    continue;
                }
                to_clear.push(uuid.clone());
                outcomes.push(RevertOutcome {
                    id,
                    restored: true,
                    note: None,
                });
            }
            None => {
                // No snapshot — at minimum drop the proteus tag so the
                // connection isn't claimed by us going forward. Leave the
                // DHCP keys alone since we don't know what to put back.
                nmdhcp::untag_user_data(&mut new_settings)?;
                if let Err(e) = nmdhcp::update_connection(&conn, &path, new_settings).await {
                    outcomes.push(RevertOutcome {
                        id,
                        restored: false,
                        note: Some(format!("failed: {e:#}")),
                    });
                    continue;
                }
                outcomes.push(RevertOutcome {
                    id,
                    restored: false,
                    note: Some("no cached originals; tag cleared, DHCP keys left as-is".into()),
                });
            }
        }
    }
    for uuid in to_clear {
        if let Some(c) = state.originals.connections.get_mut(&uuid) {
            c.dhcp_settings = None;
        }
    }
    outcomes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(outcomes)
}

fn capture_originals(state: &mut State, uuid: &str, settings: &ConnectionSettings) {
    let entry = state
        .originals
        .connections
        .entry(uuid.to_string())
        .or_default();
    if entry.dhcp_settings.is_none() {
        entry.dhcp_settings = Some(nmdhcp::snapshot_dhcp(settings));
    }
}

fn is_managed_kind(kind: &str) -> bool {
    matches!(kind, "wifi" | "ethernet")
}

fn classify_suppression(snap: &DhcpSettingsSnapshot) -> SuppressionState {
    SuppressionState {
        hostname: hostname_state(snap),
        vendor_class: vendor_class_state(snap),
        client_id: client_id_state(snap),
        duid: duid_state(snap),
    }
}

fn hostname_state(snap: &DhcpSettingsSnapshot) -> String {
    match snap.ipv4_dhcp_send_hostname {
        Some(false) => "suppressed".into(),
        Some(true) => "sending".into(),
        None => "default".into(),
    }
}

fn vendor_class_state(snap: &DhcpSettingsSnapshot) -> String {
    match snap.ipv4_dhcp_vendor_class_identifier.as_deref() {
        Some("") => "suppressed".into(),
        Some(_) => "set".into(),
        None => "default".into(),
    }
}

fn client_id_state(snap: &DhcpSettingsSnapshot) -> String {
    match snap.ipv4_dhcp_client_id.as_deref() {
        Some("mac") => "mac-coupled".into(),
        Some(other) => format!("custom ({other})"),
        None => "default".into(),
    }
}

fn duid_state(snap: &DhcpSettingsSnapshot) -> String {
    match snap.ipv6_dhcp_duid.as_deref() {
        Some("ll") => "link-layer".into(),
        Some(other) => format!("custom ({other})"),
        None => "default".into(),
    }
}

fn print_status(report: &StatusReport) {
    if !report.network_manager {
        println!("NetworkManager: not detected");
        return;
    }
    if report.connections.is_empty() {
        println!("(no NetworkManager connection profiles)");
        return;
    }
    for c in &report.connections {
        println!(
            "{} [{}] {}",
            c.id,
            c.kind,
            if c.proteus_managed {
                "proteus-managed"
            } else {
                "unmanaged"
            }
        );
        println!("  hostname:     {}", c.suppression.hostname);
        println!("  vendor-class: {}", c.suppression.vendor_class);
        println!("  client-id:    {}", c.suppression.client_id);
        println!("  duid:         {}", c.suppression.duid);
    }
}

fn print_apply(outcomes: &[ApplyOutcome]) {
    if outcomes.is_empty() {
        println!("(no NetworkManager connection profiles found)");
        return;
    }
    let mut applied = 0;
    let mut skipped = 0;
    let mut failed = 0;
    for o in outcomes {
        match (&o.note, o.changed) {
            (Some(n), _) if n.starts_with("failed:") => {
                println!("failed   {} [{}]: {}", o.id, o.kind, n);
                failed += 1;
            }
            (Some(n), _) => {
                println!("skipped  {} [{}]: {}", o.id, o.kind, n);
                skipped += 1;
            }
            (None, true) => {
                println!("applied  {} [{}]", o.id, o.kind);
                applied += 1;
            }
            (None, false) => {
                println!("idempotent {} [{}] (already proteus-managed)", o.id, o.kind);
                applied += 1;
            }
        }
    }
    println!(
        "summary: {applied} applied / {skipped} skipped / {failed} failed of {}",
        outcomes.len()
    );
}

fn print_revert(outcomes: &[RevertOutcome]) {
    if outcomes.is_empty() {
        println!("no proteus-managed DHCP connections found; nothing to revert");
        return;
    }
    for o in outcomes {
        match (&o.note, o.restored) {
            (Some(n), false) if n.starts_with("failed:") => {
                println!("failed   {}: {}", o.id, n);
            }
            (None, true) => {
                println!("restored {}", o.id);
            }
            (Some(n), false) => {
                println!("partial  {}: {}", o.id, n);
            }
            (Some(n), true) => {
                println!("restored {}: {}", o.id, n);
            }
            (None, false) => {
                println!("partial  {}: no cached originals", o.id);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ConnectionOriginals;

    /// Issue #119 — sacred-originals invariant. The dhcp apply path saves
    /// state.json AFTER each per-connection capture and BEFORE the
    /// `nm::Update()` DBus call. This test pins the round-trip half: a
    /// captured DHCP snapshot must round-trip through `State::save()` so
    /// revert can restore it after a crash mid-apply.
    #[test]
    fn captured_dhcp_originals_persist_to_disk() {
        let dir = crate::testing::TempRoot::new("dhcp");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        let uuid = "12345678-1234-1234-1234-1234567890ab"; // issue #124: uuid-keyed
        state.originals.connections.insert(
            uuid.into(),
            ConnectionOriginals {
                anonymous_identity: None,
                dhcp_settings: Some(DhcpSettingsSnapshot {
                    ipv4_dhcp_send_hostname: Some(true),
                    ipv4_dhcp_hostname: Some("factory-host".into()),
                    ipv4_dhcp_fqdn: None,
                    ipv4_dhcp_vendor_class_identifier: Some("vendor-x".into()),
                    ipv4_dhcp_client_id: Some("mac".into()),
                    ipv6_dhcp_duid: None,
                    ipv6_dhcp_iaid: None,
                }),
            },
        );
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");

        // Simulate a crash: drop in-memory state. On-disk record persists.
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        let snap = loaded
            .originals
            .connections
            .get(uuid)
            .and_then(|c| c.dhcp_settings.as_ref())
            .expect("dhcp_settings captured");
        assert_eq!(snap.ipv4_dhcp_send_hostname, Some(true));
        assert_eq!(snap.ipv4_dhcp_hostname.as_deref(), Some("factory-host"));
        assert_eq!(
            snap.ipv4_dhcp_vendor_class_identifier.as_deref(),
            Some("vendor-x")
        );
        assert_eq!(snap.ipv4_dhcp_client_id.as_deref(), Some("mac"));
        assert!(loaded.captured_at.is_some());
    }

    /// Roadmap 4c — `dhcp renew` requires `--yes`. The exit code wired
    /// through `require_yes` is `CONFIRMATION_REQUIRED` (== `CONFIG_ERROR`).
    /// This test pins both the gate and the exit code so wrappers that
    /// grep `65` keep working.
    #[test]
    fn renew_without_yes_returns_confirmation_required() {
        // We rely on the fact that `require_yes` runs first — even without
        // root, a missing --yes short-circuits before the root check.
        let code = renew(None, false, None).expect("renew should not error out");
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
    }

    /// Roadmap 4c — when invoked as a non-root user with `--yes`, the
    /// command must surface `PERMISSION_ERROR` rather than panicking on
    /// the DBus path it can't reach. This assertion hard-pins that
    /// behaviour so the CI-style test environment (which is non-root)
    /// keeps producing a deterministic exit code.
    #[test]
    fn renew_without_root_returns_permission_error() {
        // Skip on the rare case CI runs as root — the behaviour we want to
        // pin is the non-root branch.
        if super::super::read_uid() == Some(0) {
            return;
        }
        let code = renew(None, true, None).expect("renew should not error out");
        assert_eq!(code, exit::PERMISSION_ERROR);
    }

    /// `device_matches`: with an iface filter, only the named iface
    /// matches (regardless of kind/managed). Without one, only managed
    /// wifi/ethernet devices match.
    #[test]
    fn device_matches_respects_iface_filter() {
        let wifi_managed = DeviceInfo {
            interface: "wlan0".into(),
            kind: DeviceKind::Wifi,
            hw_address: None,
            path: zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Devices/1").unwrap(),
            managed: true,
            connections: vec![],
        };
        let ethernet_unmanaged = DeviceInfo {
            interface: "eth0".into(),
            kind: DeviceKind::Ethernet,
            hw_address: None,
            path: zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Devices/2").unwrap(),
            managed: false,
            connections: vec![],
        };
        let other = DeviceInfo {
            interface: "tun0".into(),
            kind: DeviceKind::Other(16),
            hw_address: None,
            path: zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Devices/3").unwrap(),
            managed: true,
            connections: vec![],
        };

        // No filter: managed wifi/ethernet only.
        assert!(device_matches(&wifi_managed, None));
        assert!(!device_matches(&ethernet_unmanaged, None));
        assert!(!device_matches(&other, None));

        // Iface filter: exact match wins regardless of kind/managed.
        assert!(device_matches(&wifi_managed, Some("wlan0")));
        assert!(device_matches(&ethernet_unmanaged, Some("eth0")));
        assert!(device_matches(&other, Some("tun0")));
        assert!(!device_matches(&wifi_managed, Some("eth0")));
    }

    /// Roadmap 4c — the `[dhcp] renew_on_apply` knob defaults to false
    /// so the orchestrator behaviour doesn't change until the
    /// integration follow-up wires it.
    #[test]
    fn renew_on_apply_defaults_to_false() {
        let cfg = crate::config::DhcpConfig::default();
        assert!(!cfg.renew_on_apply);
    }
}
