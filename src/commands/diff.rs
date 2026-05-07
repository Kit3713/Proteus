// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus diff` — config drift, managed-file drift, state summary.
//!
//! Read-only. Always exits 0; the drift is information, not an error. A GUI
//! that wants to alert on drift should inspect the JSON instead of trapping
//! the exit code.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::Config;
use crate::diff::{Report, build_report};
use crate::exit;
use crate::state::State;

pub fn run(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let config = load_config(config_path);
    let state = load_state(state_path);
    let etc_root = etc_root_from_env();
    let report = build_report(&config, state.as_ref(), &etc_root);

    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    Config::default_or_loaded(&path).unwrap_or_default()
}

fn load_state(path: Option<&Path>) -> Option<State> {
    let path = super::state_path(path);
    State::load(&path).ok().flatten()
}

/// Sandbox-friendly: `PROTEUS_ETC_ROOT` overrides the `/etc/` walk root so
/// integration tests can stage a fake tree without root.
fn etc_root_from_env() -> PathBuf {
    match std::env::var_os("PROTEUS_ETC_ROOT") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from("/"),
    }
}

fn print_human(r: &Report) {
    println!("proteus diff (schema v{})", r.schema_version);
    println!();

    println!("config drift:");
    if r.config_drift.is_empty() {
        println!("  (none) — config matches built-in defaults");
    } else {
        let key_w = r
            .config_drift
            .iter()
            .map(|d| d.key.len())
            .max()
            .unwrap_or(0);
        let cur_w = r
            .config_drift
            .iter()
            .map(|d| d.current.len())
            .max()
            .unwrap_or(0);
        for d in &r.config_drift {
            println!(
                "  {:<kw$}  current={:<cw$}  default={}",
                d.key,
                d.current,
                d.default,
                kw = key_w,
                cw = cur_w
            );
        }
    }
    println!();

    println!("managed-file drift:");
    if r.managed_file_drift.is_empty() {
        println!("  (none) — no managed files present, or all match their headers");
    } else {
        for f in &r.managed_file_drift {
            let tag = if f.drift { "DRIFT" } else { "ok" };
            println!("  [{tag}] {} — {}", f.path, f.reason);
            if let Some(exp) = &f.expected_sha {
                println!("        expected: {exp}");
                println!("        actual:   {}", f.actual_sha);
            } else {
                println!("        actual:   {}", f.actual_sha);
            }
        }
    }
    println!();

    println!("state summary:");
    println!(
        "  originals cached:   {}",
        yesno(r.state_summary.originals_cached)
    );
    println!(
        "  managed connections: {}",
        if r.state_summary.managed_connections.is_empty() {
            "(none)".to_string()
        } else {
            r.state_summary.managed_connections.join(", ")
        }
    );
    println!(
        "  pinned interfaces:  {}",
        if r.state_summary.pinned_interfaces.is_empty() {
            "(none)".to_string()
        } else {
            r.state_summary.pinned_interfaces.join(", ")
        }
    );
    println!(
        "  last rotation:      {}",
        r.state_summary
            .last_rotation_at
            .as_deref()
            .unwrap_or("(never)")
    );
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
