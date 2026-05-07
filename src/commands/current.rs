// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

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

    if json {
        super::print_json(&entries)?;
    } else if entries.is_empty() {
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
    }
    Ok(exit::SUCCESS)
}
