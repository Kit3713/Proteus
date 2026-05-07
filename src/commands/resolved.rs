// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus resolved` subcommand handlers.
//!
//! Sibling to `commands::dns` — the latter owns the ECS-strip drop-in,
//! this one owns the mDNS+LLMNR drop-in. Two separate commands so the
//! operator can revert one without disturbing the other.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::dns::resolved as dns_resolved;
use crate::dns::{self, DeferReason, Paths};
use crate::exit;

const SYSTEMD_MARKER: &str = "/run/systemd/system";
const RESOLVED_UNIT: &str = "systemd-resolved.service";

#[derive(Debug, Serialize)]
struct StatusReport {
    drop_in_present: bool,
    /// True iff drop-in is on disk AND nothing else owns DNS AND at least
    /// one knob is on. Reflects "feature applied" semantics.
    applied: bool,
    deferred_to: Option<String>,
    reason: Option<DeferReason>,
    drop_in_path: String,
    mdns_off: bool,
    llmnr_off: bool,
}

pub fn status(json: bool, config_path: Option<&Path>) -> Result<u8> {
    let cfg = load_config(config_path);
    let paths = Paths::system_default();
    let report = build_status(&cfg, &paths);
    if json {
        super::print_json(&report)?;
    } else {
        print_status_human(&report, &cfg);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(config_path: Option<&Path>) -> Result<u8> {
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

    if !dns_resolved::is_active(&cfg.resolved) {
        println!(
            "resolved: disabled in config (resolved.mdns_off and resolved.llmnr_off both \
             false); removing any prior drop-in"
        );
        return remove_and_restart(&paths);
    }

    if let Some(reason) = dns::detect_defer_system(&paths) {
        println!(
            "resolved: deferred to {} ({}); leaving your DNS setup alone",
            reason.tool_name(),
            describe_reason(&reason)
        );
        if dns_resolved::dropin_present(&paths) {
            if let Err(e) = dns_resolved::remove_dropin(&paths) {
                eprintln!("proteus: failed to remove stale drop-in: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            if let Err(e) = restart_resolved() {
                eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            println!("resolved: removed stale Proteus drop-in");
        }
        return Ok(exit::SUCCESS);
    }

    let path = match dns_resolved::write_dropin(&paths, &cfg.resolved) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("proteus: writing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    println!("resolved: wrote {}", path.display());
    if let Err(e) = restart_resolved() {
        eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    println!("resolved: restarted {RESOLVED_UNIT}");
    Ok(exit::SUCCESS)
}

pub fn revert() -> Result<u8> {
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
    let removed = match dns_resolved::remove_dropin(paths) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("proteus: removing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if removed {
        println!(
            "resolved: removed {}",
            dns_resolved::dropin_path(paths).display()
        );
        if let Err(e) = restart_resolved() {
            eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
        println!("resolved: restarted {RESOLVED_UNIT}");
    } else {
        println!("resolved: nothing to do (no Proteus drop-in present)");
    }
    Ok(exit::SUCCESS)
}

fn build_status(cfg: &Config, paths: &Paths) -> StatusReport {
    let drop_in_present = dns_resolved::dropin_present(paths);
    let reason = dns::detect_defer_system(paths);
    let active = dns_resolved::is_active(&cfg.resolved);
    let applied = drop_in_present && reason.is_none() && active;
    let deferred_to = reason.as_ref().map(|r| r.tool_name().to_string());
    StatusReport {
        drop_in_present,
        applied,
        deferred_to,
        reason,
        drop_in_path: dns_resolved::dropin_path(paths).display().to_string(),
        mdns_off: cfg.resolved.mdns_off,
        llmnr_off: cfg.resolved.llmnr_off,
    }
}

fn describe_reason(r: &DeferReason) -> String {
    match r {
        DeferReason::BinaryPresent { path, .. } => format!("binary at {path}"),
        DeferReason::ServiceActive { unit, .. } => format!("{unit} active"),
        DeferReason::ProcessRunning { process, .. } => format!("process {process} running"),
        DeferReason::CustomResolvConf { detail } => detail.clone(),
        DeferReason::ForeignDropIn { path } => format!("non-Proteus drop-in {path}"),
        DeferReason::LocalhostResolverBound { detail } => format!("listener on :53 ({detail})"),
    }
}

fn print_status_human(r: &StatusReport, cfg: &Config) {
    let label = if r.applied {
        "applied"
    } else if r.deferred_to.is_some() {
        "deferred"
    } else if !dns_resolved::is_active(&cfg.resolved) {
        "idle (disabled in config)"
    } else {
        "idle"
    };
    println!("resolved: {label}");
    println!("  config:        mdns_off = {}", r.mdns_off);
    println!("                 llmnr_off = {}", r.llmnr_off);
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

fn restart_resolved() -> Result<()> {
    if !Path::new(SYSTEMD_MARKER).is_dir() {
        tracing::debug!("systemd not detected; skipping {RESOLVED_UNIT} restart");
        return Ok(());
    }
    let out = Command::new("systemctl")
        .args(["restart", RESOLVED_UNIT])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(anyhow::anyhow!(
        "systemctl restart {RESOLVED_UNIT} exited with {}: {}",
        out.status,
        stderr.trim()
    ))
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    Config::default_or_loaded(&path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedConfig;
    use crate::testing::TempRoot;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn cfg_with_resolved(mdns: bool, llmnr: bool) -> Config {
        Config {
            resolved: ResolvedConfig {
                mdns_off: mdns,
                llmnr_off: llmnr,
            },
            ..Config::default()
        }
    }

    /// Mirrors `dns::tests::clean_root` — set up a tempdir simulating a
    /// stock Fedora layout so the detect-and-defer guard sees no third
    /// party (and lets us write).
    fn clean_root() -> TempRoot {
        let root = TempRoot::new("resolved-cmd");
        let etc = root.path.join("etc");
        let dropin = etc.join("systemd/resolved.conf.d");
        fs::create_dir_all(&dropin).unwrap();
        let stub_dir = root.path.join("run/systemd/resolve");
        fs::create_dir_all(&stub_dir).unwrap();
        fs::write(stub_dir.join("stub-resolv.conf"), "# stub\n").unwrap();
        symlink(
            "../run/systemd/resolve/stub-resolv.conf",
            etc.join("resolv.conf"),
        )
        .unwrap();
        root
    }

    #[test]
    fn status_reports_idle_when_no_dropin_and_no_defer() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, true);
        let report = build_status(&cfg, &paths);
        assert!(!report.drop_in_present);
        assert!(!report.applied);
        assert!(report.deferred_to.is_none());
    }

    #[test]
    fn status_reports_applied_after_dropin_written() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, true);
        dns_resolved::write_dropin(&paths, &cfg.resolved).expect("write");
        let report = build_status(&cfg, &paths);
        assert!(report.drop_in_present);
        assert!(report.applied);
        assert!(report.deferred_to.is_none());
    }

    #[test]
    fn status_reports_deferred_when_foreign_dropin_present() {
        let root = clean_root();
        let dropin_dir = root.path.join("etc/systemd/resolved.conf.d");
        fs::write(dropin_dir.join("99-mine.conf"), "[Resolve]\nDNS=1.1.1.1\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, true);
        let report = build_status(&cfg, &paths);
        assert!(report.deferred_to.is_some());
        assert!(!report.applied);
        match report.reason {
            Some(DeferReason::ForeignDropIn { ref path }) => {
                assert!(path.ends_with("99-mine.conf"));
            }
            other => panic!("expected ForeignDropIn, got {other:?}"),
        }
    }

    #[test]
    fn status_reports_idle_disabled_when_config_disables_both() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(false, false);
        let report = build_status(&cfg, &paths);
        // applied requires is_active; both off => never applied.
        assert!(!report.applied);
        assert!(!dns_resolved::is_active(&cfg.resolved));
    }

    #[test]
    fn revert_only_removes_proteus_managed_file() {
        let root = clean_root();
        let dropin_dir = root.path.join("etc/systemd/resolved.conf.d");
        // Adjacent third-party drop-in.
        fs::write(dropin_dir.join("99-third-party.conf"), "[Resolve]\nDNS=2\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, true);
        // Write the proteus drop-in directly (the apply path would defer
        // because of the third-party file we just added — that's expected).
        dns_resolved::write_dropin(&paths, &cfg.resolved).expect("write");

        // The remove path is what `revert` invokes; assert it does the right
        // thing in isolation.
        assert!(dns_resolved::remove_dropin(&paths).expect("remove"));
        // Proteus file gone, third-party file untouched.
        assert!(!dropin_dir.join(dns_resolved::PROTEUS_RESOLVED_DROPIN_NAME).exists());
        assert!(dropin_dir.join("99-third-party.conf").exists());
    }

    #[test]
    fn build_status_applied_field_requires_active_knob() {
        // A drop-in with both knobs off should never report applied=true.
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(false, false);
        // Ensure the "drop-in is missing" precondition is real, then add it.
        let written = dns_resolved::write_dropin(&paths, &cfg.resolved).unwrap();
        assert!(written.is_file());
        let report = build_status(&cfg, &paths);
        assert!(report.drop_in_present);
        assert!(!report.applied, "is_active=false must keep applied=false");
    }

    #[test]
    fn applied_only_true_when_both_dropin_present_and_no_defer() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, false);
        // No file → not applied.
        let report1 = build_status(&cfg, &paths);
        assert!(!report1.applied);
        // Now write the file → applied.
        dns_resolved::write_dropin(&paths, &cfg.resolved).unwrap();
        let report2 = build_status(&cfg, &paths);
        assert!(report2.applied);
    }

    #[test]
    fn describe_reason_covers_every_variant() {
        // Pin the human-readable mapping so future variants force a
        // visible match-arm update rather than silently missing one.
        let bin = describe_reason(&DeferReason::BinaryPresent {
            tool: "x",
            path: "/p".into(),
        });
        assert!(bin.contains("/p"));
        let svc = describe_reason(&DeferReason::ServiceActive {
            tool: "x",
            unit: "x.service",
        });
        assert!(svc.contains("x.service"));
        let proc = describe_reason(&DeferReason::ProcessRunning {
            tool: "x",
            process: "xd",
        });
        assert!(proc.contains("xd"));
        let resolv = describe_reason(&DeferReason::CustomResolvConf {
            detail: "/x is regular".into(),
        });
        assert!(resolv.contains("/x"));
        let drop = describe_reason(&DeferReason::ForeignDropIn {
            path: "/x.conf".into(),
        });
        assert!(drop.contains("/x.conf"));
        let listener = describe_reason(&DeferReason::LocalhostResolverBound {
            detail: "pid=42".into(),
        });
        assert!(listener.contains("42"));
    }

    #[test]
    fn dropin_path_is_under_resolved_conf_d() {
        let paths = Paths::system_default();
        let p = dns_resolved::dropin_path(&paths);
        assert!(p.to_string_lossy().contains("resolved.conf.d"));
        assert!(p.ends_with(dns_resolved::PROTEUS_RESOLVED_DROPIN_NAME));
    }

    #[test]
    fn report_serialises_to_json_without_error() {
        // The JSON path is what the CLI uses with `--json`. Ensure the
        // shape stays serialisable so a runtime regression here surfaces
        // in CI rather than at first user.
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let cfg = cfg_with_resolved(true, true);
        let report = build_status(&cfg, &paths);
        let s = serde_json::to_string(&report).expect("serialise");
        assert!(s.contains("drop_in_present"));
        assert!(s.contains("mdns_off"));
        assert!(s.contains("llmnr_off"));
    }
}
