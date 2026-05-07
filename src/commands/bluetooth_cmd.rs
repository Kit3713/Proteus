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
    let result = rt.block_on(async {
        let Some((conn, adapters)) = bluetooth::connect_and_list().await? else {
            return Ok::<_, anyhow::Error>(None);
        };
        let mut outcomes = Vec::new();
        for a in &adapters {
            let alias = bt_alias::select_alias(&config.bluetooth)?;
            let outcome =
                bt_apply::apply_one(&conn, a, &config.bluetooth, &alias, &mut state).await?;
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
            persist_capture_metadata(&mut state);
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
