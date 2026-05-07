// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus revert` — restore cached originals + remove drop-ins.
//!
//! Phase G. Backing out has to be a real option from day one. This is the
//! "undo Proteus's network-layer side-effects" hatch: hostname, Bluetooth
//! aliases, sysctl/timesyncd/resolved drop-ins, NM dispatcher hook, and the
//! `proteus` nft table all go away. The binary, config, and state file stay
//! — those belong to `uninstall` (and `--purge`).
//!
//! `revert_best_effort` is the single source of truth: both this command and
//! `commands::uninstall` call it. Each step records a warning rather than
//! aborting so a partially-applied install can still get cleaned up.
//!
//! Idempotent: re-running on an already-reverted system does nothing.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::exit;

/// Drop-ins Proteus writes outside `/etc/proteus/`. Mirrors `wiki/uninstall.md`
/// and the install script.
const EXTERNAL_DROPINS: &[&str] = &[
    "/etc/sysctl.d/95-proteus.conf",
    "/etc/systemd/timesyncd.conf.d/10-proteus.conf",
    "/etc/NetworkManager/dispatcher.d/01-proteus",
];

const RESOLVED_DROPIN_DIR: &str = "/etc/systemd/resolved.conf.d";
const RESOLVED_DROPIN_PREFIX: &str = "10-proteus-";

/// `proteus revert [--yes]` entry point.
pub fn run(yes: bool) -> Result<u8> {
    if let Err(code) = super::require_yes(yes, "revert is destructive", "proteus help revert") {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }

    let mut warns: Vec<String> = Vec::new();
    revert_best_effort(&mut warns);

    if warns.is_empty() {
        println!("proteus revert: complete");
    } else {
        eprintln!("proteus revert: {} warning(s):", warns.len());
        for w in &warns {
            eprintln!("  {w}");
        }
        eprintln!("see `proteus wiki uninstall` for manual cleanup");
    }
    Ok(exit::SUCCESS)
}

/// Best-effort revert of every Proteus side-effect outside the binary, config,
/// and state files. Shared with `commands::uninstall` so both paths stay
/// consistent. Each step is independent: a failure pushes a warning and the
/// next step still runs.
///
/// File-removal handles the on-disk drop-ins (sysctl/timesyncd/dispatcher,
/// per-link resolved drop-ins, the proteus nft table). Per-NM-connection
/// DBus state for IPv6 and DHCP is *not* covered by file removal — those
/// settings live inside NetworkManager keyfiles or in-memory connections,
/// so the corresponding submodule reverts are called explicitly.
pub(crate) fn revert_best_effort(warns: &mut Vec<String>) {
    if let Err(e) = super::hostname::revert(None) {
        warns.push(format!("hostname: {e:#}"));
    }
    if let Err(e) = super::bluetooth_cmd::revert(None) {
        warns.push(format!("bluetooth: {e:#}"));
    }
    if let Err(e) = super::ipv6::revert(true, None) {
        warns.push(format!("ipv6: {e:#}"));
    }
    if let Err(e) = super::dhcp::revert(None) {
        warns.push(format!("dhcp: {e:#}"));
    }
    if let Err(e) = super::rf::revert(true, None) {
        warns.push(format!("rf: {e:#}"));
    }
    for p in EXTERNAL_DROPINS {
        let path = Path::new(p);
        note(path, remove_file_opt(path), warns);
    }
    remove_resolved_dropins(warns);
    let _ = run_quiet("nft", &["delete", "table", "inet", "proteus"]);
    let _ = run_quiet("sysctl", &["--system"]);
    let _ = run_quiet(
        "systemctl",
        &["restart", "systemd-resolved", "systemd-timesyncd"],
    );
}

/// Resolved drop-ins are name-prefixed so the per-link files Proteus writes
/// can be wiped without scanning every conf file in the directory.
fn remove_resolved_dropins(warns: &mut Vec<String>) {
    let dir = Path::new(RESOLVED_DROPIN_DIR);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warns.push(format!("{}: {e}", dir.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if is_proteus_resolved_dropin(&s) {
            let path = entry.path();
            note(&path, remove_file_opt(&path), warns);
        }
    }
}

/// Pulled out so the matcher (prefix + `.conf` suffix) is unit-testable.
pub(crate) fn is_proteus_resolved_dropin(name: &str) -> bool {
    name.starts_with(RESOLVED_DROPIN_PREFIX) && name.ends_with(".conf")
}

/// `Ok(true)` = removed, `Ok(false)` = was already absent.
pub(crate) type Outcome = std::io::Result<bool>;

pub(crate) fn remove_file_opt(p: &Path) -> Outcome {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub(crate) fn note(p: &Path, outcome: Outcome, warns: &mut Vec<String>) {
    let d = p.display();
    match outcome {
        Ok(true) => println!("removed {d}"),
        Ok(false) => {}
        Err(e) => warns.push(format!("{d}: {e}")),
    }
}

/// Run a command quietly. Returns `Err` with a single-line message on
/// non-zero exit so callers can collect warnings without aborting. Used for
/// best-effort reload steps (`nft delete`, `sysctl --system`, `systemctl …`)
/// where failure means the daemon wasn't running, which is fine.
pub(crate) fn run_quiet(program: &str, args: &[&str]) -> Result<(), String> {
    match Command::new(program).args(args).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("{program}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_dropin_matcher_accepts_proteus_files_only() {
        assert!(is_proteus_resolved_dropin("10-proteus-wlan0.conf"));
        assert!(is_proteus_resolved_dropin("10-proteus-eth0.conf"));
        // No prefix.
        assert!(!is_proteus_resolved_dropin("99-systemd.conf"));
        // Right prefix, wrong suffix — the operator may have hand-edited.
        assert!(!is_proteus_resolved_dropin("10-proteus-eth0.bak"));
        // Right suffix, wrong prefix — adjacent third-party drop-in.
        assert!(!is_proteus_resolved_dropin("20-something.conf"));
        // Empty.
        assert!(!is_proteus_resolved_dropin(""));
    }

    #[test]
    fn external_dropins_includes_dispatcher_hook() {
        // The NM dispatcher hook ships with install.sh; revert must remove it
        // alongside the sysctl and timesyncd drop-ins or the system keeps
        // calling Proteus on every link change.
        assert!(EXTERNAL_DROPINS.contains(&"/etc/NetworkManager/dispatcher.d/01-proteus"));
        assert!(EXTERNAL_DROPINS.contains(&"/etc/sysctl.d/95-proteus.conf"));
        assert!(EXTERNAL_DROPINS.contains(&"/etc/systemd/timesyncd.conf.d/10-proteus.conf"));
    }

    #[test]
    fn remove_file_opt_is_idempotent_on_missing_path() {
        // The whole revert flow is required to be idempotent; the file-removal
        // helper must not error on a re-run when the target is already gone.
        let dir = std::env::temp_dir().join("proteus-revert-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("does-not-exist.conf");
        let res = remove_file_opt(&path).expect("missing file is Ok(false), not Err");
        assert!(!res, "expected Ok(false) for a path that never existed");
    }

    #[test]
    fn remove_file_opt_reports_existence_then_absence() {
        let dir = std::env::temp_dir().join("proteus-revert-test-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scratch.conf");
        std::fs::write(&path, b"x").unwrap();
        // First call: file existed, removed.
        assert!(remove_file_opt(&path).unwrap());
        // Second call: gone, but no error — proves idempotency.
        assert!(!remove_file_opt(&path).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
