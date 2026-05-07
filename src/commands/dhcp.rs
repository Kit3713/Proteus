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
use crate::nm::{self, ConnectionSettings, dhcp as nmdhcp};
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
    let result = rt.block_on(async { do_apply(&config, &mut state).await });
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

async fn do_apply(config: &Config, state: &mut State) -> Result<Vec<ApplyOutcome>> {
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
        // State keys by uuid (issue #124). A connection without a uuid is
        // a transient/in-memory profile NM hasn't persisted; skip it
        // rather than risk colliding-by-id state.
        let Some(uuid) = nmdhcp::connection_uuid(&settings) else {
            outcomes.push(ApplyOutcome {
                id,
                kind,
                changed: false,
                note: Some("skipped (connection has no uuid)".into()),
            });
            continue;
        };

        // Cache originals on first touch, so revert restores even after
        // multiple applies.
        capture_originals(state, &uuid, &settings);

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
        if !nmdhcp::is_proteus_managed(&settings) {
            continue;
        }
        // State is uuid-keyed (issue #124); fall back to a no-snap revert
        // when this profile hasn't been persisted (no uuid).
        let uuid = nmdhcp::connection_uuid(&settings);
        let snap = uuid
            .as_deref()
            .and_then(|u| state.originals.connections.get(u))
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
                if let Some(u) = uuid {
                    to_clear.push(u);
                }
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
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    fn make_settings(id: &str, uuid: &str, send_hostname: bool) -> ConnectionSettings {
        let mut settings: ConnectionSettings = HashMap::new();
        let conn = settings
            .entry(nmdhcp::SECTION_CONNECTION.to_string())
            .or_default();
        conn.insert(
            "id".to_string(),
            Value::from(id.to_string()).try_into().unwrap(),
        );
        conn.insert(
            "uuid".to_string(),
            Value::from(uuid.to_string()).try_into().unwrap(),
        );
        let ipv4 = settings
            .entry(nmdhcp::SECTION_IPV4.to_string())
            .or_default();
        ipv4.insert(
            nmdhcp::KEY_DHCP_SEND_HOSTNAME.to_string(),
            Value::from(send_hostname).try_into().unwrap(),
        );
        settings
    }

    #[test]
    fn capture_originals_keys_distinct_uuids_under_separate_entries() {
        // Issue #124: two NM profiles with the same connection.id but
        // different uuids must NOT collide in state. capture_originals is
        // the per-apply hook that owns the keying; it has to land each
        // uuid under its own slot.
        let mut state = State::default();
        let id = "MyHomeWiFi";
        let uuid_a = "11111111-1111-1111-1111-111111111111";
        let uuid_b = "22222222-2222-2222-2222-222222222222";
        // Distinct on-wire DHCP shape so we can tell the snapshots apart
        // without poking into private fields.
        let s_a = make_settings(id, uuid_a, true);
        let s_b = make_settings(id, uuid_b, false);
        capture_originals(&mut state, uuid_a, &s_a);
        capture_originals(&mut state, uuid_b, &s_b);
        assert_eq!(
            state.originals.connections.len(),
            2,
            "two uuids -> two state entries; pre-#124 keying collapsed them"
        );
        let snap_a = state.originals.connections[uuid_a]
            .dhcp_settings
            .as_ref()
            .expect("uuid_a snapshot");
        let snap_b = state.originals.connections[uuid_b]
            .dhcp_settings
            .as_ref()
            .expect("uuid_b snapshot");
        assert_eq!(snap_a.ipv4_dhcp_send_hostname, Some(true));
        assert_eq!(snap_b.ipv4_dhcp_send_hostname, Some(false));
    }

    #[test]
    fn capture_originals_is_idempotent_per_uuid() {
        // Re-running apply on a uuid we already cached must NOT clobber the
        // pre-Proteus snapshot — that's how revert can put the profile back
        // even after multiple toggles. The dhcp.rs flow always tags the
        // current settings, so the second call could accidentally overwrite
        // the originals if `or_default` + unconditional assign were used.
        let mut state = State::default();
        let uuid = "33333333-3333-3333-3333-333333333333";
        let original = make_settings("WiFi", uuid, true);
        capture_originals(&mut state, uuid, &original);
        // Now imagine apply ran and the live dict shows our suppressed
        // values — re-capture must preserve the original.
        let post_apply = make_settings("WiFi", uuid, false);
        capture_originals(&mut state, uuid, &post_apply);
        let snap = state.originals.connections[uuid]
            .dhcp_settings
            .as_ref()
            .expect("snapshot present");
        assert_eq!(
            snap.ipv4_dhcp_send_hostname,
            Some(true),
            "second capture must NOT overwrite the cached original"
        );
    }
}
