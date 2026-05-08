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
    extra_drops: ExtraStatus,
    rendered_ruleset: String,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChainStatus {
    enabled: bool,
    ssdp_block: bool,
    wsd_block: bool,
}

#[derive(Debug, Serialize)]
struct ExtraStatus {
    enabled: bool,
    icmpv4_timestamp_drop: bool,
    broadcast_ping_drop: bool,
    igmp_query_drop: bool,
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

    let extra = ExtraStatus {
        enabled: nft::extra_chain_active(&config.nft),
        icmpv4_timestamp_drop: config.nft.icmpv4_timestamp_drop,
        broadcast_ping_drop: config.nft.broadcast_ping_drop,
        igmp_query_drop: config.nft.igmp_query_drop,
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
        extra_drops: extra,
        rendered_ruleset: nft::render_ruleset(&config.discovery, &config.nft),
        note,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(yes: bool, config_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(yes, "'nft apply' is mutating", "proteus help nft") {
        return Ok(code);
    }
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
    // Roadmap Milestone 4a follow-up: shape the ruleset by the active
    // persona so stealth covers contribute their discovery posture
    // (e.g. iOS stealth covers without `mdns_advertise` get an inbound
    // 5353 drop alongside the existing icmp/discovery chains).
    let user_root = crate::persona::resolve::default_user_root();
    let persona = crate::persona::active_for(&config, None, user_root);
    if let Err(e) =
        nft::apply_ruleset_with_persona(&config.discovery, &config.nft, persona.as_ref())
    {
        eprintln!("proteus: nft apply failed: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    let mut extras: Vec<&str> = Vec::new();
    if config.discovery.ssdp_block {
        extras.push("SSDP block");
    }
    if config.discovery.wsd_block {
        extras.push("WSD block");
    }
    if config.nft.icmpv4_timestamp_drop {
        extras.push("ICMPv4 timestamp drop");
    }
    if config.nft.broadcast_ping_drop {
        extras.push("broadcast-ping drop");
    }
    if config.nft.igmp_query_drop {
        extras.push("IGMP query drop");
    }
    let extra_text = if extras.is_empty() {
        String::new()
    } else {
        format!(" + {}", extras.join(" + "))
    };
    println!(
        "applied table {} {} (ICMP info-drops{})",
        nft::TABLE_FAMILY,
        nft::TABLE_NAME,
        extra_text
    );
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool) -> Result<u8> {
    if let Err(code) = super::require_yes(yes, "'nft revert' is mutating", "proteus help nft") {
        return Ok(code);
    }
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
    let extra_state = if r.extra_drops.enabled {
        "configured"
    } else {
        "disabled (default)"
    };
    println!("  extra_drops:     {extra_state}");
    if r.extra_drops.enabled {
        println!(
            "    icmpv4_timestamp_drop: {}",
            yesno(r.extra_drops.icmpv4_timestamp_drop)
        );
        println!(
            "    broadcast_ping_drop:   {}",
            yesno(r.extra_drops.broadcast_ping_drop)
        );
        println!(
            "    igmp_query_drop:       {}",
            yesno(r.extra_drops.igmp_query_drop)
        );
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
