// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus ssid` subcommand handlers — roadmap Milestone 3.
//!
//! Read commands (`list`, `show`) work for any user. Mutating commands
//! (`set`, `clear`) require root because they write to
//! `/etc/proteus/config.toml`.
//!
//! Integration with the NM connection-up dispatcher is the follow-up
//! tracked in roadmap Milestone 3 — this module ships the schema and
//! the surfaced CLI; consumers come next.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::SsidAction;
use crate::config::{Config, PerSsidPolicy};
use crate::exit;
use crate::per_ssid::{self, EffectivePolicy};
use crate::profile::Profile;

/// Top-level dispatch for `proteus ssid ...`.
pub fn run(action: SsidAction, config_override: Option<&Path>) -> Result<u8> {
    match action {
        SsidAction::List { json } => list(json, config_override),
        SsidAction::Show { ssid, json } => show(&ssid, json, config_override),
        SsidAction::Set {
            ssid,
            key,
            value,
            yes,
        } => set(&ssid, &key, &value, yes, config_override),
        SsidAction::Clear { ssid, yes } => clear(&ssid, yes, config_override),
    }
}

// ---- list ------------------------------------------------------------

#[derive(Serialize)]
struct ListEntry {
    ssid: String,
    persona: Option<String>,
    aggressiveness_profile: Option<String>,
    pin_mac: Option<String>,
    rotate_interval: Option<String>,
    portal_policy: Option<String>,
}

fn list(json: bool, config_override: Option<&Path>) -> Result<u8> {
    let cfg = Config::default_or_loaded(&super::config_path(config_override)).unwrap_or_default();
    let entries: Vec<ListEntry> = cfg
        .per_ssid
        .iter()
        .map(|(ssid, p)| ListEntry {
            ssid: ssid.clone(),
            persona: p.persona.clone(),
            aggressiveness_profile: p.aggressiveness_profile.clone(),
            pin_mac: p.pin_mac.clone(),
            rotate_interval: p.rotate_interval.clone(),
            portal_policy: p.portal_policy.clone(),
        })
        .collect();
    if json {
        super::print_json(&entries)?;
        return Ok(exit::SUCCESS);
    }
    if entries.is_empty() {
        println!("(no per-SSID entries; every SSID falls through to global config)");
        return Ok(exit::SUCCESS);
    }
    for e in &entries {
        println!("[{}]", e.ssid);
        if let Some(v) = &e.persona {
            println!("  persona:                {v}");
        }
        if let Some(v) = &e.aggressiveness_profile {
            println!("  aggressiveness_profile: {v}");
        }
        if let Some(v) = &e.pin_mac {
            println!("  pin_mac:                {v}");
        }
        if let Some(v) = &e.rotate_interval {
            println!("  rotate_interval:        {v}");
        }
        if let Some(v) = &e.portal_policy {
            println!("  portal_policy:          {v}");
        }
    }
    Ok(exit::SUCCESS)
}

// ---- show ------------------------------------------------------------

#[derive(Serialize)]
struct ShowReport {
    ssid: String,
    persona: Option<String>,
    profile: String,
    pin_mac: Option<String>,
    rotate_interval_secs: Option<u64>,
    portal_policy: Option<String>,
    source: Vec<&'static str>,
}

fn show(ssid: &str, json: bool, config_override: Option<&Path>) -> Result<u8> {
    let cfg = Config::default_or_loaded(&super::config_path(config_override)).unwrap_or_default();
    let eff = per_ssid::resolve_for_ssid(&cfg, ssid);
    let report = report_for(ssid, &eff);
    if json {
        super::print_json(&report)?;
        return Ok(exit::SUCCESS);
    }
    println!("ssid:                   {}", report.ssid);
    println!("  profile:              {}", report.profile);
    match &report.persona {
        Some(p) => println!("  persona:              {p}"),
        None => println!("  persona:              (none)"),
    }
    match &report.pin_mac {
        Some(m) => println!("  pin_mac:              {m}"),
        None => println!("  pin_mac:              (unset)"),
    }
    match report.rotate_interval_secs {
        Some(s) => println!("  rotate_interval:      {s}s"),
        None => println!("  rotate_interval:      (global)"),
    }
    match &report.portal_policy {
        Some(p) => println!("  portal_policy:        {p}"),
        None => println!("  portal_policy:        (global)"),
    }
    println!("  source (per_ssid > persona > profile > defaults):");
    for s in &report.source {
        println!("    - {s}");
    }
    Ok(exit::SUCCESS)
}

fn report_for(ssid: &str, eff: &EffectivePolicy) -> ShowReport {
    ShowReport {
        ssid: ssid.to_string(),
        persona: eff.persona.clone(),
        profile: eff.profile.name().to_string(),
        pin_mac: eff.pin_mac.clone(),
        rotate_interval_secs: eff.rotate_interval.map(|d| d.as_secs()),
        portal_policy: eff.portal_policy.clone(),
        source: eff.source.clone(),
    }
}

// ---- set / clear ------------------------------------------------------

const KNOWN_KEYS: &[&str] = &[
    "persona",
    "aggressiveness_profile",
    "pin_mac",
    "rotate_interval",
    "portal_policy",
];

fn set(ssid: &str, key: &str, value: &str, yes: bool, config_override: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "writes [per_ssid.<ssid>] to config",
        "proteus help ssid",
    ) {
        return Ok(code);
    }
    if ssid.is_empty() {
        eprintln!("proteus: ssid must not be empty");
        return Ok(exit::CONFIG_ERROR);
    }
    if !KNOWN_KEYS.contains(&key) {
        eprintln!(
            "proteus: unknown ssid key '{key}'; expected one of: {}",
            KNOWN_KEYS.join(", ")
        );
        return Ok(exit::CONFIG_ERROR);
    }
    // Validate `aggressiveness_profile` up-front so a typo lands here
    // rather than later in the resolver's silent fall-through.
    if key == "aggressiveness_profile" && Profile::parse(value).is_none() {
        eprintln!(
            "proteus: invalid aggressiveness_profile '{value}'; expected one of off|min|low|med|high|agr"
        );
        return Ok(exit::CONFIG_ERROR);
    }
    let path = super::config_path(config_override);
    write_field(&path, ssid, key, value)?;
    println!("set per_ssid.\"{ssid}\".{key} = \"{value}\" in {}", path.display());
    Ok(exit::SUCCESS)
}

fn clear(ssid: &str, yes: bool, config_override: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "clears [per_ssid.<ssid>] in config",
        "proteus help ssid",
    ) {
        return Ok(code);
    }
    if ssid.is_empty() {
        eprintln!("proteus: ssid must not be empty");
        return Ok(exit::CONFIG_ERROR);
    }
    let path = super::config_path(config_override);
    let removed = drop_block(&path, ssid)?;
    if removed {
        println!("cleared per_ssid.\"{ssid}\" in {}", path.display());
    } else {
        println!("(no per_ssid.\"{ssid}\" block to clear)");
    }
    Ok(exit::SUCCESS)
}

/// Write one field on `[per_ssid."<ssid>"]`, creating the table if absent.
/// Uses `toml_edit` so non-`per_ssid` sections, comments, and key ordering
/// are preserved (mirrors `commands::persona::write_active_to_config`).
fn write_field(path: &Path, ssid: &str, key: &str, value: &str) -> Result<()> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = if raw.is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        raw.parse()
            .with_context(|| format!("parsing {}", path.display()))?
    };
    // `per_ssid` itself is a table-of-tables. `entry(...).or_insert(table())`
    // yields the parent; we then index by `ssid` to land on the per-SSID
    // sub-table.
    let parent = doc
        .entry("per_ssid")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .context("[per_ssid] is not a table in the config file")?;
    parent.set_implicit(true);
    let entry = parent
        .entry(ssid)
        .or_insert(toml_edit::table())
        .as_table_mut()
        .with_context(|| format!("per_ssid.{ssid} is not a table"))?;
    entry.insert(key, toml_edit::value(value));
    super::write_atomic(path, doc.to_string().as_bytes())?;
    Ok(())
}

/// Remove the entire `[per_ssid."<ssid>"]` block. Returns `true` when the
/// block existed and was removed; `false` is a no-op success (idempotent).
fn drop_block(path: &Path, ssid: &str) -> Result<bool> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    if raw.is_empty() {
        return Ok(false);
    }
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let Some(parent_item) = doc.get_mut("per_ssid") else {
        return Ok(false);
    };
    let Some(parent) = parent_item.as_table_mut() else {
        return Ok(false);
    };
    if parent.remove(ssid).is_none() {
        return Ok(false);
    }
    if parent.is_empty() {
        doc.remove("per_ssid");
    }
    super::write_atomic(path, doc.to_string().as_bytes())?;
    Ok(true)
}

// Used to silence the unused-import warning when only tests reference it.
#[allow(dead_code)]
fn _expose_per_ssid_policy_type(_: &PerSsidPolicy) {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Wrap {
        #[command(subcommand)]
        cmd: SsidAction,
    }

    #[test]
    fn cli_parses_list() {
        let w = Wrap::try_parse_from(["x", "list"]).expect("parse");
        match w.cmd {
            SsidAction::List { json } => assert!(!json),
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_list_with_json() {
        let w = Wrap::try_parse_from(["x", "list", "--json"]).expect("parse");
        match w.cmd {
            SsidAction::List { json } => assert!(json),
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_show() {
        let w = Wrap::try_parse_from(["x", "show", "coffee-shop"]).expect("parse");
        match w.cmd {
            SsidAction::Show { ssid, .. } => assert_eq!(ssid, "coffee-shop"),
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_set() {
        let w = Wrap::try_parse_from(["x", "set", "my-wifi", "persona", "iphone-15", "--yes"])
            .expect("parse");
        match w.cmd {
            SsidAction::Set {
                ssid,
                key,
                value,
                yes,
            } => {
                assert_eq!(ssid, "my-wifi");
                assert_eq!(key, "persona");
                assert_eq!(value, "iphone-15");
                assert!(yes);
            }
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_clear() {
        let w = Wrap::try_parse_from(["x", "clear", "my-wifi", "--yes"]).expect("parse");
        match w.cmd {
            SsidAction::Clear { ssid, yes } => {
                assert_eq!(ssid, "my-wifi");
                assert!(yes);
            }
            _ => panic!("wrong action"),
        }
    }

    /// Round-trip: `set` writes a field; reloading the config picks it
    /// up. Mirrors what the user sees from `proteus ssid set foo persona
    /// iphone-15 --yes` followed by `proteus ssid show foo`.
    #[test]
    fn set_persists_through_to_loaded_config() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-ssid-set-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        write_field(&path, "my-wifi", "persona", "iphone-15").unwrap();
        let cfg = Config::default_or_loaded(&path).unwrap();
        let entry = cfg.per_ssid.get("my-wifi").expect("entry written");
        assert_eq!(entry.persona.as_deref(), Some("iphone-15"));

        // Setting another field on the same SSID merges, not stomps.
        write_field(&path, "my-wifi", "pin_mac", "aa:bb:cc:dd:ee:ff").unwrap();
        let cfg = Config::default_or_loaded(&path).unwrap();
        let entry = cfg.per_ssid.get("my-wifi").unwrap();
        assert_eq!(entry.persona.as_deref(), Some("iphone-15"));
        assert_eq!(entry.pin_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `drop_block` removes the entire `[per_ssid."<ssid>"]` table and is
    /// idempotent on a missing entry.
    #[test]
    fn drop_block_removes_table_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-ssid-drop-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        write_field(&path, "x", "persona", "iphone-15").unwrap();
        let removed = drop_block(&path, "x").unwrap();
        assert!(removed);
        let cfg = Config::default_or_loaded(&path).unwrap();
        assert!(cfg.per_ssid.is_empty());

        let removed_again = drop_block(&path, "x").unwrap();
        assert!(!removed_again, "second drop is a no-op");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `list` over a config with no per-SSID entries returns an empty
    /// list and exits cleanly.
    #[test]
    fn list_handles_empty_config_cleanly() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-ssid-list-empty-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "profile = \"med\"\n").unwrap();
        let code = list(true, Some(&path)).unwrap();
        assert_eq!(code, exit::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `list` over a config with multiple per-SSID entries returns all of
    /// them in BTreeMap (alphabetical) order.
    #[test]
    fn list_handles_multiple_entries() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-ssid-list-multi-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        write_field(&path, "alpha", "persona", "iphone-15").unwrap();
        write_field(&path, "bravo", "pin_mac", "aa:bb:cc:dd:ee:ff").unwrap();
        let cfg = Config::default_or_loaded(&path).unwrap();
        let keys: Vec<_> = cfg.per_ssid.keys().cloned().collect();
        assert_eq!(keys, vec!["alpha".to_string(), "bravo".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
