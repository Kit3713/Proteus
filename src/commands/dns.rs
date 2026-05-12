// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus dns` subcommand handlers.
//!
//! Read commands work for any user; mutating ones require root and exit 66
//! otherwise. The hard guard runs first on apply: if anything is detected,
//! we exit 0 with a friendly note and do nothing. The user's DNS setup
//! always wins.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::dns::apply as dns_apply;
use crate::dns::{self, DeferReason, Paths};
use crate::exit;

const SYSTEMD_MARKER: &str = "/run/systemd/system";
const RESOLVED_UNIT: &str = "systemd-resolved.service";

#[derive(Debug, Serialize)]
struct StatusReport {
    /// True iff the Proteus drop-in is on disk right now.
    drop_in_present: bool,
    /// True iff the drop-in is on disk AND the guard says nothing else
    /// owns DNS. Reflects "feature applied" semantics.
    applied: bool,
    /// Tool name we deferred to, if any.
    deferred_to: Option<String>,
    /// Structured reason, populated whenever `deferred_to` is set.
    reason: Option<DeferReason>,
    /// Path of the drop-in (whether present or not).
    drop_in_path: String,
    /// Echoes config for debuggability.
    strip_edns_client_subnet: bool,
    /// True when systemd-resolved is detected on the host. Read-only hint.
    systemd_resolved_present: bool,
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

pub fn apply(yes: bool, config_path: Option<&Path>) -> Result<u8> {
    // Issue #242: gate behind --yes so a stray invocation can't restart
    // systemd-resolved without confirmation. `commands::apply` clears its
    // own gate first and passes `yes=true` here.
    if let Err(code) = super::require_yes(yes, "'dns apply' is mutating", "proteus help dns") {
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

    if !cfg.dns.strip_edns_client_subnet {
        println!(
            "dns: disabled in config (dns.strip_edns_client_subnet = false); removing any \
             prior drop-in"
        );
        // Treat disable-via-config the same as `revert` so apply stays
        // idempotent against config flips.
        return remove_and_restart(&paths);
    }

    if let Some(reason) = dns::detect_defer_system(&paths) {
        println!(
            "dns: deferred to {} ({}); leaving your DNS setup alone",
            reason.tool_name(),
            describe_reason(&reason)
        );
        // Defer also means: do not leave a stale Proteus drop-in around. If
        // the user added e.g. dnscrypt-proxy after running apply earlier,
        // we should clean up so the foreign tool isn't fighting our knob.
        if dns_apply::dropin_present(&paths) {
            if let Err(e) = dns_apply::remove_dropin(&paths) {
                eprintln!("proteus: failed to remove stale drop-in: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            if let Err(e) = restart_resolved() {
                eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
                return Ok(exit::GENERIC_ERROR);
            }
            println!("dns: removed stale Proteus drop-in");
        }
        return Ok(exit::SUCCESS);
    }

    let path = match dns_apply::write_dropin(&paths) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("proteus: writing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    println!("dns: wrote {}", path.display());
    if let Err(e) = restart_resolved() {
        eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
        return Ok(exit::GENERIC_ERROR);
    }
    println!("dns: restarted {RESOLVED_UNIT}");
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool) -> Result<u8> {
    // Issue #242: gate behind --yes for symmetry with `proteus revert`.
    if let Err(code) = super::require_yes(yes, "'dns revert' is mutating", "proteus help dns") {
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
    let removed = match dns_apply::remove_dropin(paths) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("proteus: removing drop-in failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };
    if removed {
        println!("dns: removed {}", dns_apply::dropin_path(paths).display());
        if let Err(e) = restart_resolved() {
            eprintln!("proteus: failed to restart {RESOLVED_UNIT}: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
        println!("dns: restarted {RESOLVED_UNIT}");
    } else {
        println!("dns: nothing to do (no Proteus drop-in present)");
    }
    Ok(exit::SUCCESS)
}

fn build_status(cfg: &Config, paths: &Paths) -> StatusReport {
    let drop_in_present = dns_apply::dropin_present(paths);
    let reason = dns::detect_defer_system(paths);
    let applied = drop_in_present && reason.is_none();
    let deferred_to = reason.as_ref().map(|r| r.tool_name().to_string());
    StatusReport {
        drop_in_present,
        applied,
        deferred_to,
        reason,
        drop_in_path: dns_apply::dropin_path(paths).display().to_string(),
        strip_edns_client_subnet: cfg.dns.strip_edns_client_subnet,
        systemd_resolved_present: Path::new("/run/systemd/resolve").exists(),
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
    println!(
        "dns: {}",
        if r.applied {
            "applied"
        } else if r.deferred_to.is_some() {
            "deferred"
        } else if !cfg.dns.strip_edns_client_subnet {
            "idle (disabled in config)"
        } else {
            "idle"
        }
    );
    println!(
        "  config:        strip_edns_client_subnet = {}",
        r.strip_edns_client_subnet
    );
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
    println!(
        "  systemd-resolved present: {}",
        if r.systemd_resolved_present {
            "yes"
        } else {
            "no"
        }
    );
}

fn restart_resolved() -> Result<()> {
    if !Path::new(SYSTEMD_MARKER).is_dir() {
        // Not running systemd. Nothing to restart; the drop-in still on
        // disk is harmless. Surface a debug line and move on so e.g. CI
        // containers without systemd don't flag this as an error.
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
        "systemctl restart {RESOLVED_UNIT} exited with {}: {}; see proteus wiki dns",
        out.status,
        stderr.trim()
    ))
}

fn load_config(path: Option<&Path>) -> Config {
    let path = super::config_path(path);
    Config::default_or_loaded(&path).unwrap_or_default()
}
