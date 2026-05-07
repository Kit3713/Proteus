// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::bluetooth;
use crate::bluetooth::AdapterInfo;
use crate::bluetooth::alias as bt_alias;
use crate::bluetooth::apply as bt_apply;
use crate::config::Config;
use crate::exit;
use crate::state::State;
use crate::version;

const SKIP_NOTE: &str = "bluez: not detected, skipping";

#[derive(Debug, Serialize)]
struct StatusReport {
    bluez: BluezStatus,
    adapters: Vec<AdapterReport>,
}

#[derive(Debug, Serialize)]
struct BluezStatus {
    runtime_present: bool,
    service_present: bool,
}

#[derive(Debug, Serialize)]
struct AdapterReport {
    hci: String,
    address: Option<String>,
    address_type: Option<String>,
    alias: Option<String>,
    name: Option<String>,
    discoverable: Option<bool>,
    pairable: Option<bool>,
    powered: Option<bool>,
    rpa_active: bool,
    privacy_capable: bool,
}

pub fn status(json: bool) -> Result<u8> {
    let runtime = bluetooth::detect_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;

    let outcome = rt.block_on(async { gather_status(runtime).await });

    match outcome {
        Ok(Some(report)) => render_status(&report, json),
        Ok(None) => render_skipped(json),
        Err(e) => {
            eprintln!("proteus: bluetooth status failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

async fn gather_status(runtime: bool) -> Result<Option<StatusReport>> {
    let Some((_, adapters)) = bluetooth::connect_and_list().await? else {
        if runtime {
            tracing::debug!("bluez runtime detected but service not on bus");
        }
        return Ok(None);
    };
    Ok(Some(StatusReport {
        bluez: BluezStatus {
            runtime_present: runtime,
            service_present: true,
        },
        adapters: adapters.iter().map(adapter_report).collect(),
    }))
}

fn adapter_report(a: &AdapterInfo) -> AdapterReport {
    AdapterReport {
        hci: a.hci.clone(),
        address: a.address.clone(),
        address_type: a.address_type.clone(),
        alias: a.alias.clone(),
        name: a.name.clone(),
        discoverable: a.discoverable,
        pairable: a.pairable,
        powered: a.powered,
        rpa_active: a.privacy_active,
        privacy_capable: a.privacy_capable,
    }
}

fn render_status(report: &StatusReport, json: bool) -> Result<u8> {
    if json {
        super::print_json(report)?;
    } else {
        println!("bluez: present (service on bus)");
        if report.adapters.is_empty() {
            println!("(no adapters)");
        }
        for a in &report.adapters {
            println!("adapter {}", a.hci);
            println!(
                "  address:       {} ({})",
                a.address.as_deref().unwrap_or("?"),
                a.address_type.as_deref().unwrap_or("?")
            );
            println!("  alias:         {}", a.alias.as_deref().unwrap_or("?"));
            println!("  discoverable:  {}", bool_str(a.discoverable));
            println!("  pairable:      {}", bool_str(a.pairable));
            println!("  powered:       {}", bool_str(a.powered));
            println!(
                "  rpa:           {} (capable={})",
                if a.rpa_active { "active" } else { "inactive" },
                a.privacy_capable
            );
        }
    }
    Ok(exit::SUCCESS)
}

fn render_skipped(json: bool) -> Result<u8> {
    if json {
        super::print_json(&serde_json::json!({
            "bluez": { "runtime_present": false, "service_present": false },
            "adapters": [],
            "note": SKIP_NOTE,
        }))?;
    } else {
        println!("{SKIP_NOTE}");
    }
    Ok(exit::SUCCESS)
}

fn bool_str(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "yes",
        Some(false) => "no",
        None => "?",
    }
}

pub fn apply(state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #126: lock is reentrant within a process, so the orchestrator
    // calling us is fine; a parallel `proteus bluetooth apply` is not.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;

    if !config.bluetooth.enabled {
        println!("bluetooth: disabled in config (bluetooth.enabled = false)");
        return Ok(exit::SUCCESS);
    }

    let mut state = State::load_or_default(&state_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;

    // Capture-then-save-then-mutate: list adapters, capture originals into
    // `state`, persist to disk, THEN mutate via DBus. Persisting before any
    // DBus write means a crash between capture and mutation cannot leave
    // the system mutated with no on-disk record of what to revert to
    // (sacred-originals invariant; issue #119).
    let listed = match rt.block_on(async { bluetooth::connect_and_list().await }) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("proteus: bluetooth apply failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    let Some((conn, adapters)) = listed else {
        println!("{SKIP_NOTE}");
        return Ok(exit::SUCCESS);
    };

    for a in &adapters {
        bt_apply::capture_originals_step(&mut state, a);
    }
    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    // Roadmap M2 "Integration": per-persona BT alias template. When a
    // persona is active, its `bt_name_template` shapes the alias.
    // `select_alias_with_persona` falls through to `select_alias` when
    // no persona / no template / `alias_source = "pinned"` so v0.2.x
    // users see no change.
    let active_persona = crate::persona::active_for(
        &config,
        None,
        crate::persona::resolve::default_user_root(),
    );
    let result = rt.block_on(async {
        let mut outcomes = Vec::new();
        for a in &adapters {
            let alias = bt_alias::select_alias_with_persona(
                &config.bluetooth,
                active_persona.as_ref(),
            )?;
            let outcome = bt_apply::apply_one(&conn, a, &config.bluetooth, &alias).await?;
            outcomes.push(outcome);
        }
        Ok::<_, anyhow::Error>(outcomes)
    });

    match result {
        Ok(outcomes) => {
            // Re-save to record post-mutation metadata; idempotent against
            // unchanged originals.
            state.save(&state_path)?;
            for o in &outcomes {
                println!(
                    "adapter {}: alias={} discoverable={} rpa={:?}",
                    o.hci,
                    o.alias_after.as_deref().unwrap_or("?"),
                    bool_str(o.discoverable_after),
                    o.rpa_action
                );
                for n in &o.notes {
                    println!("  note: {n}");
                }
            }
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: bluetooth apply failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
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
    let state = State::load_or_default(&state_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result = rt.block_on(async {
        let Some((conn, adapters)) = bluetooth::connect_and_list().await? else {
            return Ok::<_, anyhow::Error>(None);
        };
        let mut outcomes = Vec::new();
        for a in &adapters {
            let outcome = bt_apply::revert_one(&conn, a, &state).await?;
            outcomes.push(outcome);
        }
        Ok(Some(outcomes))
    });

    match result {
        Ok(None) => {
            println!("{SKIP_NOTE}");
            Ok(exit::SUCCESS)
        }
        Ok(Some(outcomes)) => {
            for o in &outcomes {
                if o.restored {
                    println!(
                        "adapter {}: alias restored ({} -> {})",
                        o.hci,
                        o.alias_before.as_deref().unwrap_or("?"),
                        o.original.as_deref().unwrap_or("?")
                    );
                } else {
                    println!(
                        "adapter {}: no original alias cached, leaving '{}'",
                        o.hci,
                        o.alias_before.as_deref().unwrap_or("?")
                    );
                }
            }
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: bluetooth revert failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
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
    use crate::bluetooth::AdapterInfo;

    /// Issue #119 — sacred-originals invariant. The new
    /// `bt_apply::capture_originals_step` populates `state.originals` from
    /// adapter info; this test verifies that round-tripping the captured
    /// state through disk preserves the alias so revert can later restore.
    #[test]
    fn captured_bluetooth_alias_persists_to_disk() {
        let dir = crate::testing::TempRoot::new("bluetooth");
        let state_path = dir.path.join("state.json");

        let info = AdapterInfo {
            hci: "hci0".into(),
            path: "/org/bluez/hci0".try_into().unwrap(),
            address: Some("AA:BB:CC:DD:EE:FF".into()),
            address_type: Some("public".into()),
            alias: Some("Factory Laptop".into()),
            name: Some("Factory Laptop".into()),
            discoverable: Some(false),
            pairable: Some(true),
            powered: Some(true),
            privacy_capable: false,
            privacy_active: false,
        };

        let mut state = State::default();
        bt_apply::capture_originals_step(&mut state, &info);
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");

        // Simulate a crash: drop in-memory state. On-disk originals remain.
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded
                .originals
                .bluetooth_aliases
                .get("hci0")
                .map(String::as_str),
            Some("Factory Laptop"),
            "captured alias must be durable on disk before any DBus mutation"
        );
        assert!(loaded.captured_at.is_some());
    }

    /// `capture_originals_step` is capture-once; a second call with a
    /// different alias must NOT clobber the first capture.
    #[test]
    fn capture_originals_step_is_idempotent() {
        let mut state = State::default();
        let info_first = AdapterInfo {
            hci: "hci0".into(),
            path: "/org/bluez/hci0".try_into().unwrap(),
            address: None,
            address_type: None,
            alias: Some("first".into()),
            name: None,
            discoverable: None,
            pairable: None,
            powered: None,
            privacy_capable: false,
            privacy_active: false,
        };
        let info_second = AdapterInfo {
            alias: Some("second".into()),
            ..info_first.clone()
        };
        bt_apply::capture_originals_step(&mut state, &info_first);
        bt_apply::capture_originals_step(&mut state, &info_second);
        assert_eq!(
            state
                .originals
                .bluetooth_aliases
                .get("hci0")
                .map(String::as_str),
            Some("first"),
            "second capture must not overwrite the first (sacred-originals)"
        );
    }
}
