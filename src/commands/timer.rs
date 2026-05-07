// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus timer` subcommand handlers.
//!
//! Read commands (`status`, `list`, `logs`) work for any user. Mutating
//! commands (`enable`, `disable`, `set`, `reset`) require root and exit 66
//! otherwise. If systemd isn't running we exit 70 with `systemd not detected`.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use crate::exit;
use crate::timer::{self, TimerKind, TimerSpec};

/// Marker file that systemd creates when running. Same probe `status` uses.
const SYSTEMD_MARKER: &str = "/run/systemd/system";

#[derive(Debug, Serialize)]
struct TimerStatusReport {
    timers: Vec<TimerStatusEntry>,
}

#[derive(Debug, Serialize)]
struct TimerStatusEntry {
    name: &'static str,
    unit: &'static str,
    kind: &'static str,
    description: &'static str,
    default: &'static str,
    enabled: Option<bool>,
    active: Option<bool>,
    interval: Option<String>,
    next_elapse: Option<String>,
    last_trigger: Option<String>,
    has_override: bool,
}

#[derive(Debug, Serialize)]
struct TimerListReport {
    timers: Vec<TimerListEntry>,
}

#[derive(Debug, Serialize)]
struct TimerListEntry {
    name: &'static str,
    unit: &'static str,
    kind: &'static str,
    default: &'static str,
    description: &'static str,
}

pub fn run_status(json: bool) -> Result<u8> {
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let report = TimerStatusReport {
        timers: timer::TIMERS.iter().map(build_status_entry).collect(),
    };
    if json {
        super::print_json(&report)?;
    } else {
        print_status_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn run_list(json: bool) -> Result<u8> {
    let report = TimerListReport {
        timers: timer::TIMERS
            .iter()
            .map(|t| TimerListEntry {
                name: t.short,
                unit: t.unit,
                kind: kind_str(t.kind),
                default: t.default,
                description: t.description,
            })
            .collect(),
    };
    if json {
        super::print_json(&report)?;
    } else {
        for t in &report.timers {
            println!(
                "{:<8} {:<22} {:<8} {:<6}  {}",
                t.name, t.unit, t.kind, t.default, t.description
            );
        }
    }
    Ok(exit::SUCCESS)
}

pub fn run_enable(name: &str) -> Result<u8> {
    if let Some(code) = require_root_or_exit() {
        return Ok(code);
    }
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let spec = match timer::resolve(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("proteus: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    let res = match spec.kind {
        TimerKind::Timer => systemctl(&["enable", "--now", spec.unit]),
        TimerKind::BootOneshot => systemctl(&["enable", spec.unit]),
    };
    match res {
        Ok(()) => {
            println!("enabled {}", spec.unit);
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: failed to enable {}: {e:#}", spec.unit);
            Ok(exit::GENERIC_ERROR)
        }
    }
}

pub fn run_disable(name: &str) -> Result<u8> {
    if let Some(code) = require_root_or_exit() {
        return Ok(code);
    }
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let spec = match timer::resolve(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("proteus: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    let res = match spec.kind {
        TimerKind::Timer => systemctl(&["disable", "--now", spec.unit]),
        TimerKind::BootOneshot => systemctl(&["disable", spec.unit]),
    };
    match res {
        Ok(()) => {
            println!("disabled {}", spec.unit);
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: failed to disable {}: {e:#}", spec.unit);
            Ok(exit::GENERIC_ERROR)
        }
    }
}

pub fn run_set(name: &str, interval_str: &str) -> Result<u8> {
    if let Some(code) = require_root_or_exit() {
        return Ok(code);
    }
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let spec = match timer::resolve(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("proteus: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if spec.kind != TimerKind::Timer {
        eprintln!(
            "proteus: '{}' is a {} unit; intervals only apply to timers",
            spec.short,
            kind_str(spec.kind)
        );
        return Ok(exit::CONFIG_ERROR);
    }
    let interval = match timer::parse_interval(interval_str) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("proteus: {e:#}");
            return Ok(exit::CONFIG_ERROR);
        }
    };
    if let Err(e) = write_dropin(spec, &interval) {
        eprintln!("proteus: writing drop-in failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    if let Err(e) = systemctl(&["daemon-reload"]) {
        eprintln!("proteus: daemon-reload failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    if let Err(e) = systemctl(&["restart", spec.unit]) {
        eprintln!("proteus: restart {} failed: {e:#}", spec.unit);
        return Ok(exit::GENERIC_ERROR);
    }
    println!("set {} interval to {}", spec.unit, interval_str);
    Ok(exit::SUCCESS)
}

pub fn run_reset(name: &str) -> Result<u8> {
    if let Some(code) = require_root_or_exit() {
        return Ok(code);
    }
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let spec = match timer::resolve(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("proteus: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if spec.kind != TimerKind::Timer {
        eprintln!(
            "proteus: '{}' is a {} unit; nothing to reset",
            spec.short,
            kind_str(spec.kind)
        );
        return Ok(exit::CONFIG_ERROR);
    }
    let path = timer::dropin_file(spec);
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("proteus: removing {} failed: {e}", path.display());
        return Ok(exit::GENERIC_ERROR);
    }
    // Best-effort: drop the now-empty drop-in dir. NotFound and NotEmpty are fine.
    let _ = std::fs::remove_dir(timer::dropin_dir(spec));
    if let Err(e) = systemctl(&["daemon-reload"]) {
        eprintln!("proteus: daemon-reload failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    if let Err(e) = systemctl(&["restart", spec.unit]) {
        eprintln!("proteus: restart {} failed: {e:#}", spec.unit);
        return Ok(exit::GENERIC_ERROR);
    }
    println!("reset {} to default ({})", spec.unit, spec.default);
    Ok(exit::SUCCESS)
}

pub fn run_logs(name: &str, lines: u32) -> Result<u8> {
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    let spec = match timer::resolve(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("proteus: {e}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    let lines_str = lines.to_string();
    let status = Command::new("journalctl")
        .args(["-u", spec.unit, "-n", &lines_str, "--no-pager"])
        .status()
        .context("invoking journalctl")?;
    if !status.success() {
        return Ok(exit::GENERIC_ERROR);
    }
    Ok(exit::SUCCESS)
}

fn require_systemd() -> Option<u8> {
    if Path::new(SYSTEMD_MARKER).is_dir() {
        None
    } else {
        eprintln!("proteus: systemd not detected (missing {SYSTEMD_MARKER})");
        Some(exit::SYSTEM_NOT_SUPPORTED)
    }
}

fn require_root_or_exit() -> Option<u8> {
    match super::require_root() {
        Ok(()) => None,
        Err(e) => {
            eprintln!("proteus: {e}");
            Some(exit::PERMISSION_ERROR)
        }
    }
}

fn write_dropin(spec: &TimerSpec, interval: &timer::Interval) -> Result<()> {
    let path = timer::dropin_file(spec);
    let body = timer::render_dropin(interval);
    // Atomic write with a randomized temp name + parent fsync so a partial
    // crash never leaves a half-written drop-in or a `.tmp` symlink target
    // an attacker could pre-place.
    super::write_atomic(&path, body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .context("invoking systemctl")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "systemctl {} exited with {}: {}",
        args.join(" "),
        output.status,
        stderr.trim()
    ))
}

fn build_status_entry(spec: &TimerSpec) -> TimerStatusEntry {
    let dropin_body = std::fs::read_to_string(timer::dropin_file(spec)).ok();
    let has_override = dropin_body.is_some();
    let interval = dropin_body
        .as_deref()
        .and_then(extract_cadence)
        .or_else(|| Some(spec.default.to_string()));

    let enabled = systemctl_is_enabled(spec.unit);
    let active = systemctl_is_active(spec.unit);
    let (next_elapse, last_trigger) = if spec.kind == TimerKind::Timer {
        list_timer_columns(spec.unit)
    } else {
        (None, None)
    };

    TimerStatusEntry {
        name: spec.short,
        unit: spec.unit,
        kind: kind_str(spec.kind),
        description: spec.description,
        default: spec.default,
        enabled,
        active,
        interval,
        next_elapse,
        last_trigger,
        has_override,
    }
}

fn extract_cadence(dropin_body: &str) -> Option<String> {
    for line in dropin_body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("OnUnitActiveSec=") {
            return Some(rest.to_string());
        }
        if let Some(rest) = line.strip_prefix("OnCalendar=")
            && !rest.is_empty()
        {
            return Some(rest.to_string());
        }
    }
    None
}

fn systemctl_is_enabled(unit: &str) -> Option<bool> {
    let out = Command::new("systemctl")
        .args(["is-enabled", unit])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match s.as_str() {
        "enabled" | "enabled-runtime" | "alias" | "static" | "indirect" => Some(true),
        "disabled" | "masked" | "linked" => Some(false),
        _ => None,
    }
}

fn systemctl_is_active(unit: &str) -> Option<bool> {
    let out = Command::new("systemctl")
        .args(["is-active", unit])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match s.as_str() {
        "active" | "activating" | "reloading" => Some(true),
        "inactive" | "failed" | "deactivating" => Some(false),
        _ => None,
    }
}

/// Returns (NEXT, LAST) columns from `systemctl list-timers <unit>`.
fn list_timer_columns(unit: &str) -> (Option<String>, Option<String>) {
    let out = match Command::new("systemctl")
        .args(["list-timers", "--all", "--no-pager", "--no-legend", unit])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return (None, None),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    // Header is suppressed by --no-legend, but list-timers still has variable
    // column widths. We're after the unit row; split on multiple spaces.
    for line in text.lines() {
        if !line.contains(unit) {
            continue;
        }
        // Format roughly: NEXT LEFT LAST PASSED UNIT ACTIVATES
        // Two date fields are 3 columns each (date, time, tz).
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Heuristic: NEXT = fields 0..3, LAST = fields 4..7 if present.
        if fields.len() >= 7 {
            let next = fields[0..3].join(" ");
            let last = fields[4..7].join(" ");
            let next = if next.contains('-') { Some(next) } else { None };
            let last = if last.contains('-') { Some(last) } else { None };
            return (next, last);
        }
        return (None, None);
    }
    (None, None)
}

fn kind_str(kind: TimerKind) -> &'static str {
    match kind {
        TimerKind::Timer => "timer",
        TimerKind::BootOneshot => "oneshot",
    }
}

fn print_status_human(r: &TimerStatusReport) {
    println!(
        "{:<8} {:<22} {:<8} {:<10} {:<10} {:<10}",
        "name", "unit", "kind", "enabled", "active", "interval"
    );
    for t in &r.timers {
        let enabled = t
            .enabled
            .map(|b| if b { "yes" } else { "no" })
            .unwrap_or("?");
        let active = t
            .active
            .map(|b| if b { "yes" } else { "no" })
            .unwrap_or("?");
        let interval = t.interval.as_deref().unwrap_or(t.default);
        let mark = if t.has_override { "*" } else { " " };
        println!(
            "{:<8} {:<22} {:<8} {:<10} {:<10} {}{}",
            t.name, t.unit, t.kind, enabled, active, interval, mark
        );
        if let Some(next) = &t.next_elapse {
            println!("         next:  {next}");
        }
        if let Some(last) = &t.last_trigger {
            println!("         last:  {last}");
        }
    }
    println!();
    println!("(* = drop-in override under /etc/systemd/system/proteus-*.timer.d/)");
}
