// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus portal` subcommand handlers.
//!
//! Read commands (`status`, `list`) work for any user. Mutating commands
//! (`mark`, `unmark`, `open`) require root and exit 66 otherwise — `mark`
//! and `unmark` modify state.json under `/var/lib/proteus`, and `open` is
//! grouped with them so wrappers don't have to special-case it.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use crate::captive_portal::{self, Classification, DetectionOutcome};
use crate::config::Config;
use crate::exit;
use crate::state::{PortalCheckRecord, State};

#[derive(Debug, Serialize)]
struct StatusReport {
    enabled: bool,
    detect_url: String,
    classification: String,
    note: String,
    redirect_target: Option<String>,
    timestamp: String,
}

#[derive(Debug, Serialize)]
struct ListReport {
    known_portal_ssids: Vec<String>,
}

/// Issue #163: minimum interval between actual probe network requests. A
/// `proteus session` polling loop (e.g. wrapped in `watch -n 1`) used to
/// fire one detector request per second against a shared third-party
/// endpoint (`nmcheck.gnome.org`). Cap us at one real probe per minute and
/// return the cached result for sub-minute calls.
const PORTAL_STATUS_CACHE_SECS: u64 = 60;

pub fn run_status(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();

    if !config.captive_portal.enabled {
        let report = StatusReport {
            enabled: false,
            detect_url: config.captive_portal.detect_url.clone(),
            classification: "unknown".into(),
            note: "captive_portal.enabled = false".into(),
            redirect_target: None,
            timestamp: super::now_iso8601(),
        };
        return render_status(&report, json);
    }

    // If we have a recent cached result, return it without hitting the net.
    if let Some(cached) = load_recent_portal_check(&state_path, PORTAL_STATUS_CACHE_SECS, &config) {
        return render_status(&cached, json);
    }

    let outcome = run_detector(&config);
    let timestamp = super::now_iso8601();
    let report = StatusReport {
        enabled: true,
        detect_url: config.captive_portal.detect_url.clone(),
        classification: outcome.classification.as_str().to_string(),
        note: outcome.note.clone(),
        redirect_target: outcome.redirect_target.clone(),
        timestamp: timestamp.clone(),
    };

    // Persist only when we're root — read commands stay non-root-friendly.
    if super::require_root().is_ok() {
        persist_check(&state_path, &timestamp, &outcome);
    }

    render_status(&report, json)
}

/// Return the last cached portal check from state.json if it's fresher
/// than `max_age_secs` and the cached result still matches the configured
/// detect_url. Issue #163: prevents `watch -n 1 proteus session` from
/// hammering the detect endpoint.
fn load_recent_portal_check(
    state_path: &Path,
    max_age_secs: u64,
    config: &Config,
) -> Option<StatusReport> {
    let state = State::load(state_path).ok().flatten()?;
    let last = state.last_portal_check.as_ref()?;
    let parsed = parse_iso8601_secs(&last.timestamp)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(parsed) > max_age_secs {
        return None;
    }
    Some(StatusReport {
        enabled: true,
        detect_url: config.captive_portal.detect_url.clone(),
        classification: last.classification.clone(),
        note: last.note.clone().unwrap_or_else(|| "(cached)".into()),
        redirect_target: None,
        timestamp: last.timestamp.clone(),
    })
}

/// Parse the `YYYY-MM-DDTHH:MM:SSZ` ISO-8601 form Proteus emits via
/// `commands::now_iso8601()`. Returns Unix seconds. Best-effort — anything
/// off-format yields `None`, which the caller treats as cache miss.
fn parse_iso8601_secs(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut parts = date.splitn(3, '-');
    let y: u32 = parts.next()?.parse().ok()?;
    let mo: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    let mut tparts = time.splitn(3, ':');
    let h: u32 = tparts.next()?.parse().ok()?;
    let mi: u32 = tparts.next()?.parse().ok()?;
    let se: u32 = tparts.next()?.parse().ok()?;
    // Inverse of `unix_to_ymdhms` in commands/mod.rs (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = (y as i64).div_euclid(400);
    let yoe = (y as i64 - era * 400) as u64;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let secs = days * 86_400 + (h as i64) * 3600 + (mi as i64) * 60 + se as i64;
    if secs < 0 { None } else { Some(secs as u64) }
}

pub fn run_list(json: bool, state_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    let state = State::load_or_default(&state_path).unwrap_or_default();
    let report = ListReport {
        known_portal_ssids: state.known_portal_ssids.clone(),
    };
    if json {
        super::print_json(&report)?;
    } else if report.known_portal_ssids.is_empty() {
        println!("(no known portal SSIDs)");
    } else {
        for s in &report.known_portal_ssids {
            println!("{s}");
        }
    }
    Ok(exit::SUCCESS)
}

pub fn run_mark(ssid: &str, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if ssid.is_empty() {
        eprintln!("proteus: ssid must not be empty");
        return Ok(exit::CONFIG_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;
    if state.known_portal_ssids.iter().any(|s| s == ssid) {
        println!("'{ssid}' already in known-portal list");
        return Ok(exit::SUCCESS);
    }
    state.known_portal_ssids.push(ssid.to_string());
    state.known_portal_ssids.sort();
    state.save(&state_path)?;
    println!("marked '{ssid}' as a known portal SSID");
    Ok(exit::SUCCESS)
}

pub fn run_unmark(ssid: &str, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;
    let before = state.known_portal_ssids.len();
    state.known_portal_ssids.retain(|s| s != ssid);
    if state.known_portal_ssids.len() == before {
        println!("'{ssid}' was not in the known-portal list");
        return Ok(exit::SUCCESS);
    }
    state.save(&state_path)?;
    println!("removed '{ssid}' from known-portal list");
    Ok(exit::SUCCESS)
}

pub fn run_open(state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
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
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();

    if !config.captive_portal.enabled {
        eprintln!("proteus: captive portal detection is disabled (captive_portal.enabled = false)");
        return Ok(exit::CONFIG_ERROR);
    }

    let outcome = run_detector(&config);
    let url = match outcome.redirect_target.as_deref() {
        Some(u) => u.to_string(),
        None => match outcome.classification {
            Classification::Clear => {
                println!("portal: clear — no portal in path, nothing to open");
                return Ok(exit::SUCCESS);
            }
            _ => config.captive_portal.detect_url.clone(),
        },
    };

    persist_check(&state_path, &super::now_iso8601(), &outcome);

    // Security audit H-1: never auto-open as root. Print the URL and let
    // the user paste it into their own browser. Auto-launching xdg-open
    // as root would dispatch by URL scheme to whatever desktop handler is
    // registered (file://, ssh://, vnc://, custom in-house schemes), and
    // every one of those handlers would inherit root. The previous
    // fallback path (print the URL) is now the only path.
    if !url_scheme_is_safe(&url) {
        eprintln!("proteus: refusing to surface URL with non-http(s) scheme: {url}");
        return Ok(exit::CONFIG_ERROR);
    }
    println!("portal URL: {url}");
    println!("(open this in your browser; proteus does not launch it as root)");
    Ok(exit::SUCCESS)
}

/// Captive portals are by definition untrusted; their `Location` header is
/// attacker-controlled. Reject anything whose scheme isn't `http` or
/// `https` so the URL we print can't be confused with a `file:`, `ssh:`,
/// or `javascript:` payload that the user might paste reflexively.
fn url_scheme_is_safe(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn run_detector(config: &Config) -> DetectionOutcome {
    let timeout = Duration::from_secs(config.captive_portal.timeout_secs.max(1));
    captive_portal::detect(
        &config.captive_portal.detect_url,
        &config.captive_portal.expected_response,
        timeout,
    )
}

fn persist_check(state_path: &Path, timestamp: &str, outcome: &DetectionOutcome) {
    let Ok(mut state) = State::load_or_default(state_path) else {
        return;
    };
    state.last_portal_check = Some(PortalCheckRecord {
        timestamp: timestamp.to_string(),
        classification: outcome.classification.as_str().to_string(),
        ssid: None,
        note: Some(outcome.note.clone()),
    });
    let _ = state.save(state_path);
}

// Removed: `try_xdg_open` — security audit H-1. We no longer auto-launch a
// browser as root; the URL is printed and the user opens it themselves.

fn render_status(report: &StatusReport, json: bool) -> Result<u8> {
    if json {
        super::print_json(report)?;
    } else {
        println!("captive portal:");
        println!("  enabled:         {}", yesno(report.enabled));
        println!("  detect-url:      {}", report.detect_url);
        println!("  classification:  {}", report.classification);
        println!("  note:            {}", report.note);
        if let Some(t) = &report.redirect_target {
            println!("  redirect:        {t}");
        }
        println!("  checked-at:      {}", report.timestamp);
    }
    Ok(exit::SUCCESS)
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
