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

pub fn rotate(yes: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    // Issue #242: gate behind --yes so a stray invocation can't change
    // the kernel hostname without confirmation. `commands::apply` clears
    // its own gate first and passes `yes=true` here.
    if let Err(code) = super::require_yes(
        yes,
        "'hostname rotate' is mutating",
        "proteus help hostname",
    ) {
        return Ok(code);
    }
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

    if !config.hostname.enabled {
        println!("hostname: disabled in config (hostname.enabled = false)");
        return Ok(exit::SUCCESS);
    }

    let mut state = State::load_or_default(&state_path)?;
    let mode_label = config.hostname.mode.clone();

    // Roadmap M2 "Integration": when a persona is active, its
    // `hostname_template` shapes the rotated name. Falls through to the
    // wordlist/generic/pinned path otherwise so v0.2.x users see no
    // change on upgrade.
    let active_persona =
        crate::persona::active_for(&config, None, crate::persona::resolve::default_user_root());
    let new_name = match hostname::resolve_for_apply(&config.hostname, active_persona.as_ref()) {
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

    // Capture-then-save-then-mutate: persist originals to disk BEFORE the
    // DBus write so a crash between capture and mutation can't lose them
    // (sacred-originals invariant; issue #119).
    let before = match rt.block_on(async { host_apply::capture_originals_step(&mut state).await }) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("proteus: hostname rotate failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    let result =
        rt.block_on(async { host_apply::mutate_hostname(&new_name, &mode_label, before).await });

    match result {
        Ok(outcome) => {
            // Re-save to record post-mutation metadata; idempotent against
            // unchanged originals.
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

pub fn pin(
    name: &str,
    yes: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    // Issue #242: gate behind --yes so an operator can't accidentally
    // lock in a typo'd hostname.
    if let Err(code) =
        super::require_yes(yes, "'hostname pin' is mutating", "proteus help hostname")
    {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(e) = hostname::validate_hostname(name) {
        eprintln!("proteus: invalid hostname '{name}': {e}");
        return Ok(exit::CONFIG_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

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

    // Capture-then-save-then-mutate: persist originals to disk BEFORE the
    // DBus write so a crash between capture and mutation can't lose them
    // (sacred-originals invariant; issue #119).
    let before = match rt.block_on(async { host_apply::capture_originals_step(&mut state).await }) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("proteus: hostname pin failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    let result = rt.block_on(async { host_apply::mutate_hostname(name, mode_label, before).await });

    match result {
        Ok(outcome) => {
            // Re-save to record post-mutation metadata; idempotent against
            // unchanged originals.
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

pub fn revert(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    // Issue #242: gate behind --yes for symmetry with `proteus revert`.
    // `commands::revert::revert_best_effort` passes `yes=true` once the
    // parent gate has cleared.
    if let Err(code) = super::require_yes(
        yes,
        "'hostname revert' is mutating",
        "proteus help hostname",
    ) {
        return Ok(code);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #119 — sacred-originals invariant. Verifies that a captured
    /// hostname triple round-trips through `State::save()` and stays on
    /// disk so revert can restore even after a crash between save and the
    /// hostnamed DBus calls.
    #[test]
    fn captured_hostname_originals_persist_to_disk() {
        let dir = crate::testing::TempRoot::new("hostname");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        state.originals.hostname = Some(HostnameOriginals {
            kernel: Some("factory-laptop".into()),
            pretty: Some("Factory Laptop".into()),
            transient: Some("factory-laptop".into()),
        });
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");

        // Simulate a crash: drop in-memory state. On-disk originals must
        // remain.
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        let triple = loaded.originals.hostname.expect("hostname captured");
        assert_eq!(triple.kernel.as_deref(), Some("factory-laptop"));
        assert_eq!(triple.pretty.as_deref(), Some("Factory Laptop"));
        assert_eq!(triple.transient.as_deref(), Some("factory-laptop"));
        assert!(loaded.captured_at.is_some());
    }
}
