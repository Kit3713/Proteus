// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus nft` subcommand handlers.
//!
//! `status` is read-only and works for any user. `apply` and `revert` are
//! mutating and require root; non-root invocations exit 66.
//!
//! When `nft` itself is missing we surface that as exit 70 (system not
//! supported) rather than a generic error so wrapping GUIs can distinguish
//! "the host doesn't have nftables" from "the rule install failed".

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::nft::{self, TableProbe};

#[derive(Debug, Serialize)]
struct StatusReport {
    nft_present: bool,
    table_installed: bool,
    table_family: &'static str,
    table_name: &'static str,
    icmp_drops: bool,
    discovery_drops: ChainStatus,
    rendered_ruleset: String,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChainStatus {
    enabled: bool,
    ssdp_block: bool,
    wsd_block: bool,
}

pub fn status(json: bool, config_path: Option<&Path>) -> Result<u8> {
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let nft_present = nft::nft_present();
    let (table_installed, note) = if nft_present {
        match nft::list_our_table() {
            Ok(TableProbe::Present(_)) => (true, None),
            Ok(TableProbe::Absent) => (false, None),
            Ok(TableProbe::PermissionDenied) => (
                false,
                Some("table state unknown — re-run as root to inspect live rules".into()),
            ),
            Err(e) => (false, Some(format!("{e:#}"))),
        }
    } else {
        (false, Some("nft binary not on PATH".into()))
    };

    let discovery = ChainStatus {
        enabled: config.discovery.ssdp_block || config.discovery.wsd_block,
        ssdp_block: config.discovery.ssdp_block,
        wsd_block: config.discovery.wsd_block,
    };

    let report = StatusReport {
        nft_present,
        table_installed,
        table_family: nft::TABLE_FAMILY,
        table_name: nft::TABLE_NAME,
        // The icmp_drops chain is always part of our ruleset, so it's a
        // function of `table_installed`, not a separately toggled feature.
        icmp_drops: table_installed,
        discovery_drops: discovery,
        rendered_ruleset: nft::render_ruleset(&config.discovery),
        note,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(_yes: bool, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !nft::nft_present() {
        eprintln!("proteus: nft binary not on PATH; install nftables and retry");
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }
    let _lock = match super::acquire_state_lock_or_print(None) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;
    if let Err(e) = nft::apply_ruleset(&config.discovery) {
        eprintln!("proteus: nft apply failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    let extra = match (config.discovery.ssdp_block, config.discovery.wsd_block) {
        (false, false) => String::new(),
        (true, false) => " + SSDP block".into(),
        (false, true) => " + WSD block".into(),
        (true, true) => " + SSDP + WSD blocks".into(),
    };
    println!(
        "applied table {} {} (ICMP info-drops{})",
        nft::TABLE_FAMILY,
        nft::TABLE_NAME,
        extra
    );
    Ok(exit::SUCCESS)
}

pub fn revert(_yes: bool) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !nft::nft_present() {
        eprintln!("proteus: nft binary not on PATH; nothing to revert");
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }
    let _lock = match super::acquire_state_lock_or_print(None) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    if let Err(e) = nft::revert_ruleset() {
        eprintln!("proteus: nft revert failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    println!("removed table {} {}", nft::TABLE_FAMILY, nft::TABLE_NAME);
    Ok(exit::SUCCESS)
}

fn print_human(r: &StatusReport) {
    println!(
        "nft binary:        {}",
        if r.nft_present { "present" } else { "missing" }
    );
    println!(
        "table {} {}: {}",
        r.table_family,
        r.table_name,
        if r.table_installed {
            "installed"
        } else {
            "not installed"
        }
    );
    println!(
        "  icmp_drops:      {}",
        if r.icmp_drops { "active" } else { "inactive" }
    );
    let discovery_state = if r.discovery_drops.enabled {
        "configured"
    } else {
        "disabled (default)"
    };
    println!("  discovery_drops: {discovery_state}");
    if r.discovery_drops.enabled {
        println!("    ssdp_block:    {}", yesno(r.discovery_drops.ssdp_block));
        println!("    wsd_block:     {}", yesno(r.discovery_drops.wsd_block));
    }
    if let Some(note) = &r.note {
        println!("note:              {note}");
    }
    println!();
    println!("--- rendered ruleset ---");
    print!("{}", r.rendered_ruleset);
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
