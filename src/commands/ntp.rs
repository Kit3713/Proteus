// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus ntp` subcommand handlers.
//!
//! Detect-and-defer when `chronyd` or `ntpd` is on the system; otherwise
//! ship a privacy-respecting timesyncd drop-in. Mirrors the shape of
//! `commands::dns` and `commands::resolved`.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::ntp::{self, DeferReason, Paths};

const SYSTEMD_MARKER: &str = "/run/systemd/system";
const TIMESYNCD_UNIT: &str = "systemd-timesyncd.service";

#[derive(Debug, Serialize)]
struct StatusReport {
    drop_in_present: bool,
    /// True iff drop-in is on disk AND nothing else owns NTP AND the
    /// feature is enabled in config.
    applied: bool,
    deferred_to: Option<String>,
    reason: Option<DeferReason>,
    drop_in_path: String,
    enabled: bool,
    ntp_servers: Vec<String>,
    fallback_servers: Vec<String>,
}

pub fn status(json: bool, config_path: Option<&Path>) -> Result<u8> {
    let cfg = load_config(config_path);
    let paths = Paths::system_default();
    let report = build_status(&cfg, &paths);
    if json {
        super::print_json(&report)?;
    } else {
        print_status_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(yes: bool, config_path: Option<&Path>) -> Result<u8> {
    // Issue #242: gate behind --yes so a stray invocation can't restart
    // systemd-timesyncd without confirmation. `commands::apply` clears
    // its own gate first and passes `yes=true` here.
    if let Err(code) = super::require_yes(yes, "'ntp apply' is mutating", "proteus help ntp") {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(None) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let cfg = load_config(config_path);
    let paths = Paths::system_default();

    if !cfg.ntp.enabled {
        println!("ntp: disabled in config (ntp.enabled = false); removing any prior drop-in");
        return remove_and_restart(&paths);
    }

    if let Some(reason) = ntp::detect_defer_system(&paths) {
        println!(
            "ntp: deferred to {} ({}); leaving your time setup alone",
            reason.tool_name(),
            describe_reason(&reason)
        );
        if ntp::dropin_present(&paths) {
            if let Err(e) = ntp::remove_dropin(&paths) {
                eprintln!("proteus: failed to remove stale drop-in: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            if let Err(e) = restart_timesyncd() {
                eprintln!("proteus: failed to restart {TIMESYNCD_UNIT}: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            println!("ntp: removed stale Proteus drop-in");
        }
        return Ok(exit::SUCCESS);
    }

    // Roadmap Milestone 4a: when a stealth persona is active, override
    // the configured NTP servers with the persona's vendor pool so the
    // wire-side NTP queries match the cover identity. The user's own
    // `[ntp]` block always wins for fields the persona doesn't supply.
    let effective_ntp = persona_shaped_ntp(&cfg);
    let path = match ntp::write_dropin(&paths, &effective_ntp) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("proteus: writing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    println!("ntp: wrote {}", path.display());
    if effective_ntp.ntp_servers != cfg.ntp.ntp_servers {
        println!(
            "ntp: persona-shaped servers active (NTP={})",
            effective_ntp.ntp_servers.join(" ")
        );
    }
    if let Err(e) = restart_timesyncd() {
        eprintln!("proteus: failed to restart {TIMESYNCD_UNIT}: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    println!("ntp: restarted {TIMESYNCD_UNIT}");
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool) -> Result<u8> {
    // Issue #242: gate behind --yes for symmetry with `proteus revert`.
    if let Err(code) = super::require_yes(yes, "'ntp revert' is mutating", "proteus help ntp") {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(None) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let paths = Paths::system_default();
    remove_and_restart(&paths)
}

fn remove_and_restart(paths: &Paths) -> Result<u8> {
    let removed = match ntp::remove_dropin(paths) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("proteus: removing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if removed {
        println!("ntp: removed {}", ntp::dropin_path(paths).display());
        if let Err(e) = restart_timesyncd() {
            eprintln!("proteus: failed to restart {TIMESYNCD_UNIT}: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
        println!("ntp: restarted {TIMESYNCD_UNIT}");
    } else {
        println!("ntp: nothing to do (no Proteus drop-in present)");
    }
    Ok(exit::SUCCESS)
}

fn build_status(cfg: &Config, paths: &Paths) -> StatusReport {
    let drop_in_present = ntp::dropin_present(paths);
    let reason = ntp::detect_defer_system(paths);
    let applied = drop_in_present && reason.is_none() && cfg.ntp.enabled;
    let deferred_to = reason.as_ref().map(|r| r.tool_name().to_string());
    StatusReport {
        drop_in_present,
        applied,
        deferred_to,
        reason,
        drop_in_path: ntp::dropin_path(paths).display().to_string(),
        enabled: cfg.ntp.enabled,
        ntp_servers: cfg.ntp.ntp_servers.clone(),
        fallback_servers: cfg.ntp.fallback_servers.clone(),
    }
}

fn describe_reason(r: &DeferReason) -> String {
    match r {
        DeferReason::BinaryPresent { path, .. } => format!("binary at {path}"),
        DeferReason::ServiceActive { unit, .. } => format!("{unit} active"),
        DeferReason::ForeignDropIn { path } => format!("non-Proteus drop-in {path}"),
    }
}

fn print_status_human(r: &StatusReport) {
    let label = if r.applied {
        "applied"
    } else if r.deferred_to.is_some() {
        "deferred"
    } else if !r.enabled {
        "idle (disabled in config)"
    } else {
        "idle"
    };
    println!("ntp: {label}");
    println!("  config:        enabled = {}", r.enabled);
    if !r.ntp_servers.is_empty() {
        println!("                 NTP={}", r.ntp_servers.join(" "));
    }
    if !r.fallback_servers.is_empty() {
        println!(
            "                 FallbackNTP={}",
            r.fallback_servers.join(" ")
        );
    }
    println!("  drop-in path:  {}", r.drop_in_path);
    println!(
        "  drop-in:       {}",
        if r.drop_in_present {
            "present"
        } else {
            "absent"
        }
    );
    if let Some(name) = &r.deferred_to {
        println!("  deferred to:   {name}");
        if let Some(reason) = &r.reason {
            println!("  reason:        {}", describe_reason(reason));
        }
    }
}

fn restart_timesyncd() -> Result<()> {
    if !Path::new(SYSTEMD_MARKER).is_dir() {
        tracing::debug!("systemd not detected; skipping {TIMESYNCD_UNIT} restart");
        return Ok(());
    }
    let out = Command::new("systemctl")
        .args(["restart", TIMESYNCD_UNIT])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(anyhow::anyhow!(
        "systemctl restart {TIMESYNCD_UNIT} exited with {}: {}; see proteus wiki troubleshooting",
        out.status,
        stderr.trim()
    ))
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    Config::default_or_loaded(&path).unwrap_or_default()
}

/// Roadmap Milestone 4a — apply persona-defined NTP servers on top of
/// the configured `[ntp]` block. The persona's pool replaces both
/// `ntp_servers` and `fallback_servers` when provided; the user's
/// config wins when no persona is active or the persona has no opinion.
fn persona_shaped_ntp(cfg: &Config) -> crate::config::NtpConfig {
    let user_root = crate::persona::resolve::default_user_root();
    let active = crate::persona::active_for(cfg, None, user_root);
    let mut effective = cfg.ntp.clone();
    if let Some(p) = active.as_ref()
        && let Some((primary, fallback)) = ntp::servers_for_persona(p)
    {
        effective.ntp_servers = primary;
        effective.fallback_servers = fallback;
    }
    effective
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NtpConfig;
    use crate::testing::TempRoot;
    use std::fs;

    fn cfg_with_ntp(enabled: bool) -> Config {
        Config {
            ntp: NtpConfig {
                enabled,
                ..NtpConfig::default()
            },
            ..Config::default()
        }
    }

    fn clean_root() -> TempRoot {
        let root = TempRoot::new("ntp-cmd");
        fs::create_dir_all(root.path.join("etc/systemd/timesyncd.conf.d")).unwrap();
        root
    }

    #[test]
    fn status_idle_when_no_dropin_and_no_defer() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        assert!(!report.drop_in_present);
        assert!(!report.applied);
        assert!(report.deferred_to.is_none());
    }

    #[test]
    fn status_applied_after_dropin_written() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        ntp::write_dropin(&paths, &cfg.ntp).unwrap();
        let report = build_status(&cfg, &paths);
        assert!(report.applied);
    }

    #[test]
    fn status_idle_when_disabled_in_config() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(false);
        let report = build_status(&cfg, &paths);
        assert!(!report.applied);
        assert!(!report.enabled);
    }

    #[test]
    fn status_deferred_when_chrony_binary_present() {
        let root = clean_root();
        let bin_dir = root.path.join("usr/sbin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("chronyd"), "").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        assert!(report.deferred_to.is_some());
        assert!(!report.applied);
    }

    #[test]
    fn status_deferred_when_ntpd_binary_present() {
        let root = clean_root();
        let bin_dir = root.path.join("usr/sbin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("ntpd"), "").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        assert!(matches!(
            report.reason,
            Some(DeferReason::BinaryPresent { tool, .. }) if tool == "ntpd"
        ));
    }

    #[test]
    fn status_deferred_when_foreign_dropin_present() {
        let root = clean_root();
        let dir = root.path.join("etc/systemd/timesyncd.conf.d");
        fs::write(dir.join("99-mine.conf"), "[Time]\nNTP=mine\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        match report.reason {
            Some(DeferReason::ForeignDropIn { ref path }) => {
                assert!(path.ends_with("99-mine.conf"));
            }
            other => panic!("expected ForeignDropIn, got {other:?}"),
        }
    }

    #[test]
    fn revert_only_removes_proteus_managed_file() {
        let root = clean_root();
        let dir = root.path.join("etc/systemd/timesyncd.conf.d");
        fs::write(dir.join("99-third-party.conf"), "[Time]\nNTP=other\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        ntp::write_dropin(&paths, &cfg.ntp).unwrap();
        assert!(ntp::remove_dropin(&paths).unwrap());
        assert!(!dir.join(ntp::PROTEUS_NTP_DROPIN_NAME).exists());
        assert!(dir.join("99-third-party.conf").exists());
    }

    #[test]
    fn applied_requires_enabled_in_config() {
        // Even with the file on disk, status must report not-applied when
        // the master switch is off — we'll be removing it on next apply.
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(false);
        // Bypass the apply path's own gate by writing directly.
        ntp::write_dropin(&paths, &cfg.ntp).unwrap();
        let report = build_status(&cfg, &paths);
        assert!(report.drop_in_present);
        assert!(!report.applied);
    }

    #[test]
    fn report_carries_servers_for_inspection() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        assert!(report.ntp_servers.contains(&"2.fedora.pool.ntp.org".into()));
        assert!(
            report
                .fallback_servers
                .contains(&"time.cloudflare.com".into())
        );
    }

    #[test]
    fn describe_reason_covers_every_variant() {
        let bin = describe_reason(&DeferReason::BinaryPresent {
            tool: "chrony",
            path: "/usr/sbin/chronyd".into(),
        });
        assert!(bin.contains("/usr/sbin/chronyd"));
        let svc = describe_reason(&DeferReason::ServiceActive {
            tool: "ntpd",
            unit: "ntpd.service",
        });
        assert!(svc.contains("ntpd.service"));
        let drop = describe_reason(&DeferReason::ForeignDropIn {
            path: "/etc/systemd/timesyncd.conf.d/99.conf".into(),
        });
        assert!(drop.contains("99.conf"));
    }

    #[test]
    fn report_serialises_to_json_without_error() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_ntp(true);
        let report = build_status(&cfg, &paths);
        let s = serde_json::to_string(&report).expect("serialise");
        assert!(s.contains("drop_in_present"));
        assert!(s.contains("ntp_servers"));
    }
}
