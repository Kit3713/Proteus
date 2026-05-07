// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::bluetooth;
use crate::commands::status;
use crate::exit;
use crate::state::State;

#[derive(Debug, Serialize)]
struct Entry {
    iface: String,
    mac: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    pinned: Option<String>,
    managed: bool,
    last_rotated: Option<String>,
}

#[derive(Debug, Serialize)]
struct BluetoothAdapter {
    hci: String,
    address: Option<String>,
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct CurrentReport {
    interfaces: Vec<Entry>,
    bluetooth: Vec<BluetoothAdapter>,
}

pub fn run(json: bool, iface_filter: Option<&str>, state_path: Option<&Path>) -> Result<u8> {
    let path = super::state_path(state_path);
    let state = State::load(&path)?.unwrap_or_default();

    let entries: Vec<Entry> = status::enumerate_interfaces()
        .into_iter()
        .filter(|i| iface_filter.is_none_or(|f| f == i.name))
        .map(|i| {
            let rec = state.managed.interfaces.get(&i.name);
            Entry {
                iface: i.name,
                mac: i.mac,
                kind: i.kind,
                pinned: rec.and_then(|r| r.pinned.clone()),
                managed: rec.is_some(),
                last_rotated: rec.and_then(|r| r.last_rotated.clone()),
            }
        })
        .collect();

    let bt_adapters = if iface_filter.is_some() {
        Vec::new()
    } else {
        gather_bluetooth_adapters().unwrap_or_default()
    };

    if json {
        let report = CurrentReport {
            interfaces: entries,
            bluetooth: bt_adapters,
        };
        super::print_json(&report)?;
    } else if entries.is_empty() && bt_adapters.is_empty() {
        println!("(no matching interfaces)");
    } else {
        for e in &entries {
            let mac = e.mac.as_deref().unwrap_or("?");
            let pin_marker = e
                .pinned
                .as_deref()
                .map(|p| format!(" [pinned={p}]"))
                .unwrap_or_default();
            println!("{:<12} {:<8} {}{}", e.iface, e.kind, mac, pin_marker);
        }
        for a in &bt_adapters {
            println!(
                "{:<12} {:<8} {}  [alias={}]",
                a.hci,
                "bt",
                a.address.as_deref().unwrap_or("?"),
                a.alias.as_deref().unwrap_or("?")
            );
        }
    }
    Ok(exit::SUCCESS)
}

fn gather_bluetooth_adapters() -> Result<Vec<BluetoothAdapter>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    rt.block_on(async {
        let Some((_, adapters)) = bluetooth::connect_and_list().await? else {
            return Ok::<_, anyhow::Error>(Vec::new());
        };
        Ok(adapters
            .into_iter()
            .map(|a| BluetoothAdapter {
                hci: a.hci,
                address: a.address,
                alias: a.alias,
            })
            .collect())
    })
}
