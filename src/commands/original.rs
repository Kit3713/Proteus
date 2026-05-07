// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::exit;
use crate::state::State;

#[derive(Serialize)]
struct EmptyReport {
    #[serde(flatten)]
    state: State,
    note: &'static str,
}

pub fn run(json: bool, override_path: Option<&Path>) -> Result<u8> {
    let path = super::state_path(override_path);

    match State::load(&path)? {
        None => {
            if json {
                let empty = EmptyReport {
                    state: State::default(),
                    note: "no original cache yet",
                };
                super::print_json(&empty)?;
            } else {
                println!(
                    "no original cache yet — Proteus has not been run with `apply` on this system"
                );
                println!("(state file checked: {})", path.display());
            }
        }
        Some(state) => {
            if json {
                super::print_json(&state)?;
            } else {
                print_human(&state);
            }
        }
    }
    Ok(exit::SUCCESS)
}

fn print_human(state: &State) {
    println!(
        "captured by: proteus {}",
        state.captured_by_version.as_deref().unwrap_or("?")
    );
    println!(
        "captured at: {}",
        state.captured_at.as_deref().unwrap_or("?")
    );
    println!(
        "hostname:    {}",
        state
            .original_hostname
            .as_deref()
            .unwrap_or("(none cached)")
    );
    println!("MACs:");
    if state.original_macs.is_empty() {
        println!("  (none cached)");
    } else {
        for (iface, mac) in &state.original_macs {
            println!("  {iface:<12} {mac}");
        }
    }
}
