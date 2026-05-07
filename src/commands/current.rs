// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use serde::Serialize;

use crate::commands::status;
use crate::exit;

#[derive(Debug, Serialize)]
struct Entry {
    iface: String,
    mac: Option<String>,
    #[serde(rename = "type")]
    kind: String,
}

pub fn run(json: bool, iface_filter: Option<&str>) -> Result<u8> {
    let entries: Vec<Entry> = status::enumerate_interfaces()
        .into_iter()
        .filter(|i| iface_filter.is_none_or(|f| f == i.name))
        .map(|i| Entry {
            iface: i.name,
            mac: i.mac,
            kind: i.kind,
        })
        .collect();

    if json {
        super::print_json(&entries)?;
    } else if entries.is_empty() {
        println!("(no matching interfaces)");
    } else {
        for e in &entries {
            let mac = e.mac.as_deref().unwrap_or("?");
            println!("{:<12} {:<8} {}", e.iface, e.kind, mac);
        }
    }
    Ok(exit::SUCCESS)
}
