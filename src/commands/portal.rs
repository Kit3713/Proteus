// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus portal` subcommand handlers.
//!
//! Read commands (`status`, `list`) work for any user. Mutating commands
//! (`mark`, `unmark`, `open`) require root and exit 66 otherwise — `mark`
//! and `unmark` modify state.json under `/var/lib/proteus`, and `open` is
//! grouped with them so wrappers don't have to special-case it.

use std::path::Path;
use std::process::Command as ProcCommand;
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

    match try_xdg_open(&url) {
        Ok(()) => {
            println!("opened {url} in default browser");
            Ok(exit::SUCCESS)
        }
        Err(reason) => {
            // Don't fail — print the URL so the user can paste it.
            println!("portal URL: {url}");
            eprintln!("proteus: could not launch browser ({reason}); paste the URL above");
            Ok(exit::SUCCESS)
        }
    }
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

fn try_xdg_open(url: &str) -> std::result::Result<(), String> {
    match ProcCommand::new("xdg-open").arg(url).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("xdg-open exited with {s}")),
        Err(e) => Err(format!("xdg-open not available: {e}")),
    }
}

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
