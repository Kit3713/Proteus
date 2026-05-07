// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus stack` subcommand handlers.
//!
//! `status` is a read command and works for any user. `apply` and `revert`
//! mutate `/etc/sysctl.d/95-proteus.conf` and require root (exit 66
//! otherwise). The actual rendering, line mapping, and SHA computation
//! lives in `crate::stack`; this file is the surface that ties config +
//! state + filesystem + `sysctl --system` reload together.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::commands;
use crate::config::Config;
use crate::exit;
use crate::stack::{self, DROPIN_PATH};
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    proteus_version: &'static str,
    dropin_path: &'static str,
    dropin_present: bool,
    dropin_sha: Option<String>,
    expected_sha: String,
    drift: bool,
    interfaces: Vec<String>,
    entries: Vec<EntryReport>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EntryReport {
    key: String,
    expected: String,
    live: Option<String>,
    matches: bool,
}

pub fn status(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let config_path = commands::config_path(config_path);
    let state_path = commands::state_path(state_path);
    let config = Config::default_or_loaded(&config_path)?;
    let state = State::load(&state_path).ok().flatten();

    let ifaces = stack::detect_managed_interfaces();
    let lines = stack::lines_for(&config.stack, &ifaces);
    let body = stack::render_body(&lines);
    let expected_sha = stack::sha256::hex(body.as_bytes());

    let mut notes = Vec::new();
    let dropin_present = Path::new(DROPIN_PATH).exists();
    let dropin_sha = if dropin_present {
        let bytes = fs::read(DROPIN_PATH)
            .with_context(|| format!("reading {DROPIN_PATH}"))
            .ok();
        bytes.and_then(|b| extract_header_sha(&b))
    } else {
        None
    };

    let drift = match (dropin_present, &dropin_sha) {
        (true, Some(s)) => s != &expected_sha,
        (true, None) => {
            notes.push("drop-in present but header SHA not parseable".into());
            true
        }
        (false, _) => false,
    };

    if state.is_none() {
        notes.push("no state file yet — apply hasn't been run".into());
    }

    let entries: Vec<EntryReport> = lines
        .iter()
        .map(|l| {
            let live = stack::read_sysctl(&l.key);
            let matches = live.as_deref() == Some(l.value.as_str());
            EntryReport {
                key: l.key.clone(),
                expected: l.value.clone(),
                live,
                matches,
            }
        })
        .collect();

    let report = StatusReport {
        proteus_version: version::VERSION,
        dropin_path: DROPIN_PATH,
        dropin_present,
        dropin_sha,
        expected_sha,
        drift,
        interfaces: ifaces,
        entries,
        notes,
    };

    if json {
        commands::print_json(&report)?;
    } else {
        print_status_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(yes: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = commands::require_yes(yes, "'stack apply' is mutating", "proteus help stack")
    {
        return Ok(code);
    }
    if let Err(e) = commands::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match commands::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let config_path = commands::config_path(config_path);
    let state_path = commands::state_path(state_path);
    let config = Config::default_or_loaded(&config_path)?;
    let mut state = State::load_or_default(&state_path)?;

    let ifaces = stack::detect_managed_interfaces();
    let lines = stack::lines_for(&config.stack, &ifaces);

    // Cache live values *before* writing — this is the revert anchor. Empty
    // string means "key did not exist on this kernel". Never overwrite an
    // existing capture; first apply wins.
    for l in &lines {
        if state.originals.sysctls.contains_key(&l.key) {
            continue;
        }
        let captured = stack::read_sysctl(&l.key).unwrap_or_default();
        state.originals.sysctls.insert(l.key.clone(), captured);
    }

    // Persist originals to disk BEFORE the destructive drop-in write so a
    // crash or SIGKILL between here and `sysctl --system` cannot leave the
    // system mutated with no on-disk record of the originals to revert to
    // (sacred-originals invariant; see issue #119).
    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    let rendered = stack::render_dropin(&config.stack, &ifaces);

    if let Err(e) = write_dropin(&rendered) {
        eprintln!("proteus: writing {DROPIN_PATH} failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }

    let reload = sysctl_system_reload();
    match reload {
        Ok(()) => println!(
            "wrote {DROPIN_PATH} ({n} keys) and reloaded sysctl",
            n = lines.len()
        ),
        Err(e) => {
            eprintln!("proteus: wrote {DROPIN_PATH} but sysctl reload failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    }
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) =
        commands::require_yes(yes, "'stack revert' is mutating", "proteus help stack")
    {
        return Ok(code);
    }
    if let Err(e) = commands::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match commands::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    match fs::remove_file(DROPIN_PATH) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("no drop-in at {DROPIN_PATH}; nothing to remove");
            return Ok(exit::SUCCESS);
        }
        Err(e) => {
            eprintln!("proteus: removing {DROPIN_PATH} failed: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    }

    // Defaults restore on reboot; `sysctl --system` re-reads the remaining
    // drop-ins now so the live values fall back to whatever the rest of the
    // system declares. Sysctls that were only set by us at runtime stay set
    // until reboot; that's documented in wiki/stack-fingerprint.md.
    if let Err(e) = sysctl_system_reload() {
        eprintln!("proteus: removed {DROPIN_PATH} but sysctl reload failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    println!("removed {DROPIN_PATH} and reloaded sysctl defaults");
    Ok(exit::SUCCESS)
}

fn write_dropin(body: &str) -> Result<()> {
    let path = Path::new(DROPIN_PATH);
    commands::write_atomic(path, body.as_bytes())
        .with_context(|| format!("writing {DROPIN_PATH}"))?;
    Ok(())
}

fn sysctl_system_reload() -> Result<()> {
    let output = Command::new("sysctl")
        .arg("--system")
        .output()
        .context("invoking `sysctl --system`")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "sysctl --system exited with {}: {}",
        output.status,
        stderr.trim()
    )
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(commands::now_iso8601());
    }
}

/// Pull the `# sha256:<hex>` line out of a managed-file header. Returns the
/// hex string only — caller compares to the expected SHA.
fn extract_header_sha(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("# sha256:") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn print_status_human(r: &StatusReport) {
    println!("proteus {} — stack-fingerprint", r.proteus_version);
    println!();
    println!("drop-in:  {}", r.dropin_path);
    println!(
        "  present:       {}",
        if r.dropin_present { "yes" } else { "no" }
    );
    if let Some(sha) = &r.dropin_sha {
        println!("  on-disk sha:   {sha}");
    }
    println!("  expected sha:  {}", r.expected_sha);
    println!(
        "  drift:         {}",
        if r.drift { "yes (manual edits?)" } else { "no" }
    );
    println!();
    println!("interfaces:");
    if r.interfaces.is_empty() {
        println!("  (none detected)");
    } else {
        for i in &r.interfaces {
            println!("  {i}");
        }
    }
    println!();
    println!("sysctl entries (expected | live):");
    if r.entries.is_empty() {
        println!("  (no entries — every [stack] knob is off)");
    } else {
        for e in &r.entries {
            let live = e.live.as_deref().unwrap_or("?");
            let mark = if e.matches { " " } else { "*" };
            println!("  {mark} {:<48} {} | {}", e.key, e.expected, live);
        }
        println!();
        println!("(* = live value differs from what we'd write)");
    }
    if !r.notes.is_empty() {
        println!();
        for n in &r.notes {
            println!("note: {n}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_header_sha_finds_hash() {
        let body = "# managed by proteus v0.1.0\n# do not edit\n# sha256:abc123\nnet.ipv4.tcp_timestamps = 0\n";
        assert_eq!(
            extract_header_sha(body.as_bytes()),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn extract_header_sha_returns_none_when_absent() {
        let body = "# managed by proteus v0.1.0\nnet.ipv4.tcp_timestamps = 0\n";
        assert!(extract_header_sha(body.as_bytes()).is_none());
    }

    #[test]
    fn extract_header_sha_returns_none_for_blank_value() {
        let body = "# sha256:\n";
        assert!(extract_header_sha(body.as_bytes()).is_none());
    }

    /// Issue #119 — sacred-originals invariant. Verifies that captured sysctl
    /// originals round-trip through `State::save()` and land on disk. The
    /// apply path now saves state BEFORE writing the drop-in, so a crash
    /// between save and `sysctl --system` cannot lose the originals; this
    /// test pins the round-trip half of that contract.
    #[test]
    fn captured_sysctl_originals_persist_to_disk() {
        let dir = crate::testing::TempRoot::new("stack");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        state
            .originals
            .sysctls
            .insert("net.ipv4.tcp_timestamps".into(), "1".into());
        state
            .originals
            .sysctls
            .insert("net.ipv6.conf.default.use_tempaddr".into(), "".into());

        state.save(&state_path).expect("state.save");
        assert!(state_path.exists(), "state.json must be on disk");

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded
                .originals
                .sysctls
                .get("net.ipv4.tcp_timestamps")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            loaded
                .originals
                .sysctls
                .get("net.ipv6.conf.default.use_tempaddr")
                .map(String::as_str),
            Some(""),
            "empty-string capture (key absent on kernel) must round-trip"
        );
    }

    /// Issue #119 — simulates a crash AFTER state.save() but BEFORE mutation
    /// completes. After such a crash, the state file must still exist with
    /// captured originals so revert can restore. This test pins that
    /// post-save / pre-mutation crash window leaves the originals durable.
    #[test]
    fn originals_survive_simulated_crash_between_save_and_mutate() {
        let dir = crate::testing::TempRoot::new("stack");
        let state_path = dir.path.join("state.json");

        // Step 1: capture into in-memory state (mirrors apply()).
        let mut state = State::default();
        state
            .originals
            .sysctls
            .insert("net.ipv4.tcp_timestamps".into(), "1".into());
        persist_capture_metadata(&mut state);

        // Step 2: persist to disk BEFORE any mutation (the fix).
        state.save(&state_path).expect("state.save");

        // Step 3: simulate a crash — drop in-memory state without writing
        // the drop-in. The on-disk file must still contain the original.
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded
                .originals
                .sysctls
                .get("net.ipv4.tcp_timestamps")
                .map(String::as_str),
            Some("1"),
            "post-save crash must leave originals durable for revert"
        );
        assert!(
            loaded.captured_at.is_some(),
            "captured_at must be on disk so the originals are dated"
        );
    }
}
