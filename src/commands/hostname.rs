// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::hostname::{self, apply as host_apply};
use crate::state::{HostnameOriginals, State};
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    enabled: bool,
    mode: String,
    pinned_value: Option<String>,
    rotate_with_mac: bool,
    current: HostnameOriginals,
    originals: OriginalsSection,
}

#[derive(Debug, Serialize, Default)]
struct OriginalsSection {
    captured: bool,
    #[serde(flatten)]
    triple: HostnameOriginals,
}

pub fn status(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let state = State::load_or_default(&state_path).unwrap_or_default();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let snapshot = rt.block_on(async {
        match hostname::dbus::proxy().await {
            Ok(p) => Some(hostname::dbus::read_snapshot(&p).await),
            Err(_) => None,
        }
    });

    let current = snapshot
        .map(|s| HostnameOriginals {
            kernel: s.static_name,
            pretty: s.pretty_name,
            transient: s.transient_name,
        })
        .unwrap_or_default();

    let originals = match state.originals.hostname.clone() {
        Some(triple) => OriginalsSection {
            captured: true,
            triple,
        },
        None => OriginalsSection::default(),
    };

    let report = StatusReport {
        enabled: config.hostname.enabled,
        mode: config.hostname.mode.clone(),
        pinned_value: config.hostname.pinned_value.clone(),
        rotate_with_mac: config.hostname.rotate_with_mac,
        current,
        originals,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_status(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn rotate(state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;

    if !config.hostname.enabled {
        println!("hostname: disabled in config (hostname.enabled = false)");
        return Ok(exit::SUCCESS);
    }

    let mut state = State::load_or_default(&state_path)?;
    let mode_label = config.hostname.mode.clone();

    let new_name = match hostname::resolve_hostname(&config.hostname) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("proteus: hostname rotate failed: {e:#}");
            return Ok(exit::CONFIG_ERROR);
        }
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result =
        rt.block_on(async { host_apply::apply_hostname(&new_name, &mode_label, &mut state).await });

    match result {
        Ok(outcome) => {
            persist_capture_metadata(&mut state);
            state.save(&state_path)?;
            print_apply(&outcome);
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: hostname rotate failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

pub fn pin(name: &str, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(e) = hostname::validate_hostname(name) {
        eprintln!("proteus: invalid hostname '{name}': {e}");
        return Ok(exit::CONFIG_ERROR);
    }

    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;

    if !config.hostname.enabled {
        // Pinning is an explicit, user-initiated action — surface a clean
        // hint rather than silently applying.
        eprintln!(
            "proteus: hostname is disabled in config (hostname.enabled = false); not applying"
        );
        return Ok(exit::CONFIG_ERROR);
    }

    let mut state = State::load_or_default(&state_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let mode_label = hostname::Mode::Pinned.as_str();
    let result =
        rt.block_on(async { host_apply::apply_hostname(name, mode_label, &mut state).await });

    match result {
        Ok(outcome) => {
            persist_capture_metadata(&mut state);
            state.save(&state_path)?;
            print_apply(&outcome);
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: hostname pin failed: {e:#}");
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
    let result = rt.block_on(async { host_apply::revert_hostname(&state).await });

    match result {
        Ok(outcome) => {
            if outcome.restored {
                println!(
                    "hostname restored: kernel={} pretty={} transient={}",
                    outcome.current.kernel.as_deref().unwrap_or("(unset)"),
                    outcome.current.pretty.as_deref().unwrap_or("(unset)"),
                    outcome.current.transient.as_deref().unwrap_or("(unset)"),
                );
            } else {
                println!("hostname: no original cached, leaving current state untouched");
            }
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: hostname revert failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

fn print_status(r: &StatusReport) {
    println!("hostname:");
    println!("  enabled:          {}", yesno(r.enabled));
    println!("  mode:             {}", r.mode);
    println!(
        "  pinned_value:     {}",
        r.pinned_value.as_deref().unwrap_or("(unset)")
    );
    println!("  rotate_with_mac:  {}", yesno(r.rotate_with_mac));
    println!("current:");
    println!(
        "  kernel:           {}",
        r.current.kernel.as_deref().unwrap_or("(unset)")
    );
    println!(
        "  pretty:           {}",
        r.current.pretty.as_deref().unwrap_or("(unset)")
    );
    println!(
        "  transient:        {}",
        r.current.transient.as_deref().unwrap_or("(unset)")
    );
    println!("originals:");
    if r.originals.captured {
        println!(
            "  kernel:           {}",
            r.originals.triple.kernel.as_deref().unwrap_or("(unset)")
        );
        println!(
            "  pretty:           {}",
            r.originals.triple.pretty.as_deref().unwrap_or("(unset)")
        );
        println!(
            "  transient:        {}",
            r.originals.triple.transient.as_deref().unwrap_or("(unset)")
        );
    } else {
        println!("  (none cached — first apply has not run yet)");
    }
}

fn print_apply(o: &host_apply::ApplyOutcome) {
    println!(
        "hostname [{}]: kernel={} pretty={} transient={}",
        o.mode,
        o.current.kernel.as_deref().unwrap_or("(unset)"),
        o.current.pretty.as_deref().unwrap_or("(unset)"),
        o.current.transient.as_deref().unwrap_or("(unset)")
    );
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
