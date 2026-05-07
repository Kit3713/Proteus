// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus probe` — manual probe quorum check.
//!
//! Read-only. No root required. Reads `[probes]` from config (defaults from
//! `Config::default()`). With `--quick`, contacts only the first endpoint and
//! treats `quorum_n=1, quorum_total=1` so the result is binary. With `--json`,
//! emits the schema-versioned report on stdout.

use std::path::Path;

use anyhow::Result;

use crate::config::Config;
use crate::probe::{self, ProbeReport};

pub fn run(json: bool, quick: bool, config_path: Option<&Path>) -> Result<u8> {
    let cfg = load_config(config_path);

    let report = if quick {
        run_quick(&cfg)
    } else {
        run_full(&cfg)
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }

    Ok(report.classification.exit_code())
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    Config::default_or_loaded(&path).unwrap_or_default()
}

fn run_full(cfg: &Config) -> ProbeReport {
    let endpoints = effective_endpoints(cfg);
    let quorum_total = endpoints.len() as u8;
    // Honour configured quorum_n but clamp to the actual endpoint count so
    // an admin can't accidentally make `clear` unreachable by setting
    // quorum_n above quorum_total.
    let quorum_n = cfg.probes.quorum_n.min(quorum_total).max(1);
    let results = probe::run_endpoints(&endpoints);
    probe::build_report(results, quorum_n, quorum_total)
}

fn run_quick(cfg: &Config) -> ProbeReport {
    let endpoints = effective_endpoints(cfg);
    let first: Vec<String> = endpoints.into_iter().take(1).collect();
    let results = probe::run_endpoints(&first);
    probe::build_report(results, 1, 1)
}

/// Endpoints from config, falling back to defaults if the user emptied the
/// list. An empty `[probes].endpoints = []` would otherwise classify every
/// round as `inconclusive` — surprising and useless.
fn effective_endpoints(cfg: &Config) -> Vec<String> {
    if cfg.probes.endpoints.is_empty() {
        crate::config::ProbesConfig::default().endpoints
    } else {
        cfg.probes.endpoints.clone()
    }
}

fn print_human(r: &ProbeReport) {
    println!(
        "classification: {} ({}/{} succeeded; quorum {} of {})",
        classification_str(r.classification),
        r.successes,
        r.endpoints.len(),
        r.quorum_n,
        r.quorum_total,
    );
    println!();
    for ep in &r.endpoints {
        let mark = if ep.ok { "ok " } else { "FAIL" };
        let err = ep.error.as_deref().unwrap_or("");
        println!(
            "  {:<22} {:<4} {:>5} ms  {} {}",
            ep.target, ep.method, ep.duration_ms, mark, err
        );
    }
}

fn classification_str(c: probe::Classification) -> &'static str {
    match c {
        probe::Classification::Clear => "clear",
        probe::Classification::Down => "down",
        probe::Classification::Inconclusive => "inconclusive",
        probe::Classification::PortalSuspected => "portal-suspected",
    }
}
