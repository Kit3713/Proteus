// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::{Config, RawConfig};
use crate::display::display_safe;
use crate::exit;

#[derive(Serialize)]
struct MissingReport<'a> {
    config_present: bool,
    path: String,
    note: &'a str,
}

/// Roadmap #404: `proteus config show --annotate` JSON shape. The `config`
/// payload is the same TOML-shaped serialisation as the unannotated form
/// (so existing `jq` recipes keep working); a parallel `_origins` map
/// records the per-section provenance label (`file`, `profile:<name>`,
/// `per-ssid:<ssid>`).
#[derive(Serialize)]
struct AnnotatedReport<'a> {
    config: &'a Config,
    /// Section-name -> provenance label. Keys are the TOML section names
    /// (`"mac"`, `"timers"`, ...); per-SSID entries are emitted as
    /// `"per_ssid.<ssid>"` with `per-ssid:<ssid>` as the value.
    #[serde(rename = "_origins")]
    origins: &'a BTreeMap<String, String>,
}

pub fn run(json: bool, annotate: bool, override_path: Option<&Path>) -> Result<u8> {
    let path = super::config_path(override_path);

    if !path.exists() {
        if json {
            super::print_json(&MissingReport {
                config_present: false,
                path: path.display().to_string(),
                note: "no config file; defaults are in effect — see `proteus show-defaults`",
            })?;
        } else {
            println!(
                "no config file at {}; using built-in defaults — see `proteus show-defaults`",
                path.display()
            );
        }
        return Ok(exit::SUCCESS);
    }
    if let Err(e) = std::fs::metadata(&path) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            // Roadmap Stream 7 / E4: a config the user can't read is a
            // hard error, not a warning. The exit code (PERMISSION_ERROR)
            // is unchanged; the journal level is promoted so an operator
            // grepping `journalctl -p err` actually sees it.
            tracing::error!("skip {}: permission denied", path.display());
            return Ok(exit::PERMISSION_ERROR);
        }
        return Err(e).with_context(|| format!("stat {}", path.display()));
    }

    let cfg = Config::default_or_loaded(&path)?;
    if annotate {
        let origins = collect_origins(&path, &cfg)?;
        render_annotated(&cfg, &origins, json)?;
    } else {
        super::render_config(&cfg, json)?;
    }
    Ok(exit::SUCCESS)
}

/// Build the section -> provenance map. Coarse-but-shipping: granularity
/// is section-level, not field-level — every scalar inside `[mac]` shares
/// the same label as the section header. Field-level provenance is a
/// follow-up (see `wiki/cli.md`).
fn collect_origins(path: &Path, cfg: &Config) -> Result<BTreeMap<String, String>> {
    let (explicit, profile_in_file) = match std::fs::read_to_string(path) {
        Ok(s) if !s.is_empty() => {
            let raw: RawConfig =
                toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
            (raw.explicit_sections(), raw.profile.is_some())
        }
        _ => (BTreeSet::new(), false),
    };

    let profile_label = format!("profile:{}", cfg.profile.name());
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for section in ALL_SECTIONS {
        let label = if explicit.contains(section) {
            "file".to_string()
        } else {
            profile_label.clone()
        };
        out.insert((*section).to_string(), label);
    }
    // Top-level `profile` key: `file` if the user wrote it, else `default`
    // (the built-in default profile applies regardless of any section).
    out.insert(
        "profile".to_string(),
        if profile_in_file { "file" } else { "default" }.to_string(),
    );
    // Per-SSID: one entry per SSID. The SSID is sanitised through
    // `display_safe` so a hostile SSID can't corrupt the JSON / human
    // render via embedded control bytes.
    for ssid in cfg.per_ssid.keys() {
        let safe = display_safe(ssid);
        out.insert(format!("per_ssid.{safe}"), format!("per-ssid:{safe}"));
    }
    Ok(out)
}

/// Render the resolved config with per-section provenance comments.
///
/// In TOML output we keep the file shape valid by appending the
/// provenance as a trailing `# <source>` comment — TOML treats that as
/// whitespace so the output still round-trips through `toml::from_str`.
fn render_annotated(cfg: &Config, origins: &BTreeMap<String, String>, json: bool) -> Result<()> {
    if json {
        let report = AnnotatedReport {
            config: cfg,
            origins,
        };
        super::print_json(&report)?;
        return Ok(());
    }

    let toml_str = toml::to_string_pretty(cfg).context("serializing config to TOML")?;
    // Track the current section so each scalar line is suffixed with the
    // same label as its header. Lines before any section header (e.g. the
    // top-level `profile = "med"`) take the `profile` slot.
    let mut current: &str = "profile";
    for line in toml_str.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('[')
            && let Some(header) = rest.strip_suffix(']')
        {
            // Section header `[name]` or `[per_ssid."ssid"]`. Resolve to
            // the origin map key.
            let key = if let Some(rest) = header.strip_prefix("per_ssid.") {
                // `per_ssid."<ssid>"` — strip the surrounding quotes for
                // the lookup; the SSID is sanitised so a hostile name
                // can't corrupt the rendered line.
                let safe = display_safe(rest.trim_matches('"'));
                format!("per_ssid.{safe}")
            } else {
                // Sub-tables like `[timers.rotate]` roll up to the
                // top-level section name for provenance purposes.
                header
                    .split_once('.')
                    .map(|(top, _)| top)
                    .unwrap_or(header)
                    .to_string()
            };
            current = origins.get(&key).map(String::as_str).unwrap_or("default");
            println!("{line}  # {current}");
            continue;
        }
        if line.is_empty() {
            println!();
            continue;
        }
        if trimmed.starts_with('#') {
            // Existing comment line: pass through untouched.
            println!("{line}");
            continue;
        }
        // Scalar `key = value` line.
        let label = if trimmed.starts_with("profile") {
            origins
                .get("profile")
                .map(String::as_str)
                .unwrap_or("default")
        } else {
            current
        };
        println!("{line}  # {label}");
    }
    Ok(())
}

/// Top-level TOML section names in render order. Mirrors the field order
/// in `Config` so the annotation walker can match section headers as they
/// appear in the serialised TOML.
const ALL_SECTIONS: &[&str] = &[
    "mac",
    "bluetooth",
    "hostname",
    "dns",
    "resolved",
    "ntp",
    "nft",
    "discovery",
    "probes",
    "ipv6",
    "enterprise_wifi",
    "stack",
    "dhcp",
    "captive_portal",
    "rf",
    "timers",
    "persona",
    "events",
    "backend",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Roadmap Stream 7 acceptance: the success path of `show-config`
    /// emits zero `tracing::info!` events from this module. The
    /// permission-denied path now uses `error!` (E4) — that lives on
    /// the failure path so it does not violate the default-stderr
    /// discipline. We pin by asserting:
    /// 1) zero `tracing::info!` calls anywhere in the prod source;
    /// 2) `tracing::warn!` is gone (E4 promoted it to error).
    #[test]
    fn no_info_calls_and_no_warn_calls_in_prod() {
        let src = include_str!("show_config.rs");
        // Cut at the test module so this test's own assertion
        // strings don't trigger.
        let prod = src
            .split_once("\n#[cfg(test)]\n")
            .map(|(prod, _)| prod)
            .unwrap_or(src);
        let mut without_comments = String::with_capacity(prod.len());
        for line in prod.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            without_comments.push_str(line);
            without_comments.push('\n');
        }
        assert!(
            !without_comments.contains("tracing::info!"),
            "show_config must not call tracing::info!"
        );
        assert!(
            !without_comments.contains("tracing::warn!"),
            "Stream 7 / E4: show_config permission-denied must use error!, not warn!"
        );
    }

    /// Roadmap #404: with no on-disk overrides, every section's origin
    /// label is `profile:<name>` and the synthetic `profile` key is
    /// `default`.
    #[test]
    fn origins_default_when_no_overrides_present() {
        let cfg = Config::default();
        // Point at a missing file under a fresh tempdir so `collect_origins`
        // exercises the file-not-found branch without racing other tests.
        let tmp = crate::testing::TempRoot::new("origins-missing");
        let path = tmp.path.join("absent.toml");
        let origins = collect_origins(&path, &cfg).expect("collect_origins on missing path");
        let expected = format!("profile:{}", cfg.profile.name());
        for section in ALL_SECTIONS {
            assert_eq!(
                origins.get(*section).map(String::as_str),
                Some(expected.as_str()),
                "section {section} should default to profile baseline"
            );
        }
        assert_eq!(
            origins.get("profile").map(String::as_str),
            Some("default"),
            "top-level profile key with no file should be `default`"
        );
    }

    /// Roadmap #404: a section with an explicit override is labelled
    /// `file`; sections the user did not touch keep `profile:<name>`.
    #[test]
    fn origins_mark_explicit_sections_as_file() {
        let tmp = crate::testing::TempRoot::new("origins-mixed");
        let path = tmp.path.join("config.toml");
        std::fs::write(
            &path,
            "profile = \"med\"\n[mac]\nrotation_interval = \"30m\"\n",
        )
        .unwrap();
        let cfg = Config::default_or_loaded(&path).expect("loads");
        let origins = collect_origins(&path, &cfg).expect("collect_origins");
        assert_eq!(
            origins.get("mac").map(String::as_str),
            Some("file"),
            "mac was overridden in the file"
        );
        assert_eq!(
            origins.get("profile").map(String::as_str),
            Some("file"),
            "profile key was written in the file"
        );
        // A section the file did not touch stays at the profile baseline.
        let expected = format!("profile:{}", cfg.profile.name());
        assert_eq!(
            origins.get("dns").map(String::as_str),
            Some(expected.as_str()),
            "dns section was not overridden; should be profile baseline"
        );
    }
}
