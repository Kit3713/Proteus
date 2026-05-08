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
/// and the install script. `pub(crate)` so the dry-run preview iterates the
/// same list as the real teardown (issue #265).
pub(crate) const EXTERNAL_DROPINS: &[&str] = &[
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
    // Issue #126: hold the state lock while iterating through every revert
    // step so a concurrent `apply`/`rotate`/etc. can't race us.
    let _lock = match super::acquire_state_lock_or_print(None) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

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

/// Tracks which on-disk artifacts the file-removal pass actually deleted.
/// Used to decide whether the matching daemon needs a reload — issue
/// #153/#155: previously `revert_best_effort` would `systemctl restart`
/// `systemd-resolved` and `systemd-timesyncd` on every invocation, even
/// when nothing on disk had changed.
#[derive(Debug, Default)]
pub(crate) struct RevertChanged {
    pub sysctl_dropin_removed: bool,
    pub timesyncd_dropin_removed: bool,
    pub resolved_dropin_removed: bool,
    pub nft_table_removed: bool,
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
///
/// Issue #242: each per-feature revert below is passed `yes=true` because
/// the parent (`commands::revert::run` or `commands::uninstall::run`) has
/// already cleared its own `--yes` gate.
pub(crate) fn revert_best_effort(warns: &mut Vec<String>) {
    let mut changed = RevertChanged::default();
    if let Err(e) = super::hostname::revert(true, None) {
        warns.push(format!("hostname: {e:#}"));
    }
    if let Err(e) = super::bluetooth_cmd::revert(true, None) {
        warns.push(format!("bluetooth: {e:#}"));
    }
    if let Err(e) = super::ipv6::revert(true, None) {
        warns.push(format!("ipv6: {e:#}"));
    }
    if let Err(e) = super::dhcp::revert(true, None) {
        warns.push(format!("dhcp: {e:#}"));
    }
    // Issue #298: enterprise-wifi was missing from the revert fan-out,
    // so `proteus revert` left `802-1x.anonymous-identity` on every
    // managed connection. Restore the cached originals here so the
    // 802.1X profile goes back to the pre-Proteus state alongside
    // every other feature.
    if let Err(e) = super::enterprise_wifi::revert(true, None) {
        warns.push(format!("enterprise-wifi: {e:#}"));
    }
    if let Err(e) = super::rf::revert(true, None) {
        warns.push(format!("rf: {e:#}"));
    }
    for p in EXTERNAL_DROPINS {
        let path = Path::new(p);
        let outcome = remove_file_opt(path);
        if matches!(outcome, Ok(true)) {
            // Track which kind of drop-in was actually removed so we can
            // skip the matching reload when nothing changed.
            if path.starts_with("/etc/sysctl.d/") {
                changed.sysctl_dropin_removed = true;
            } else if path.starts_with("/etc/systemd/timesyncd.conf.d/") {
                changed.timesyncd_dropin_removed = true;
            }
        }
        note(path, outcome, warns);
    }
    if remove_resolved_dropins(warns) {
        changed.resolved_dropin_removed = true;
    }
    // `nft delete table` non-zero usually means the table was already
    // absent (a no-op); record only the success case.
    if run_quiet("nft", &["delete", "table", "inet", "proteus"]).is_ok() {
        changed.nft_table_removed = true;
    }
    if changed.sysctl_dropin_removed {
        let _ = run_quiet("sysctl", &["--system"]);
    }
    // The original code restarted both daemons unconditionally, which
    // could thrash a healthy system on every `proteus revert` re-run.
    // Restart only the ones whose drop-ins we actually pulled.
    let mut to_restart: Vec<&str> = Vec::new();
    if changed.resolved_dropin_removed {
        to_restart.push("systemd-resolved");
    }
    if changed.timesyncd_dropin_removed {
        to_restart.push("systemd-timesyncd");
    }
    if !to_restart.is_empty() {
        let mut args = vec!["restart"];
        args.extend(to_restart);
        let _ = run_quiet("systemctl", &args);
    }
}

/// Resolved drop-ins are name-prefixed so the per-link files Proteus
/// writes can be wiped without scanning every conf file in the directory.
/// Returns `true` if at least one matching file was actually removed —
/// callers use that to decide whether to reload `systemd-resolved`.
fn remove_resolved_dropins(warns: &mut Vec<String>) -> bool {
    let dir = Path::new(RESOLVED_DROPIN_DIR);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => {
            warns.push(format!("{}: {e}", dir.display()));
            return false;
        }
    };
    let mut removed_any = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if is_proteus_resolved_dropin(&s) {
            let path = entry.path();
            let outcome = remove_file_opt(&path);
            if matches!(outcome, Ok(true)) {
                removed_any = true;
            }
            note(&path, outcome, warns);
        }
    }
    removed_any
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
    fn nm_dispatcher_hook_pins_absolute_path_and_resets_path_env() {
        // Issue #121: NM dispatcher runs as root, so resolving `proteus` via
        // $PATH is a privilege-escalation surface. The shipped script must
        // pin to an absolute path and reset $PATH to a root-owned set, so a
        // future regression — e.g. someone "simplifying" back to PATH lookup
        // — fails the build instead of silently rooting the host.
        let script = include_str!("../../dist/networkmanager/dispatcher.d/01-proteus");

        assert!(
            script.contains("/usr/bin/proteus"),
            "dispatcher must reference /usr/bin/proteus by absolute path"
        );
        assert!(
            script.contains("PATH=/usr/sbin:/usr/bin:/sbin:/bin"),
            "dispatcher must reset PATH to a minimal root-owned set"
        );

        // Forbidden tokens that imply $PATH-based resolution. We scan
        // non-comment lines so the header rationale doesn't trip us.
        const FORBIDDEN: &[&str] = &["command -v proteus", "which proteus", "exec proteus "];
        for (lineno, raw) in script.lines().enumerate() {
            let line = raw.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            for token in FORBIDDEN {
                assert!(
                    !line.contains(token),
                    "dispatcher line {} uses PATH lookup ({token:?}): {raw}",
                    lineno + 1
                );
            }
            // A leading `proteus <args>` call (not part of a path/variable).
            assert!(
                !(line.starts_with("proteus ") || line.starts_with("proteus\t")),
                "dispatcher line {} starts with bare `proteus`: {raw}",
                lineno + 1
            );
        }
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
    fn remove_resolved_dropins_returns_false_when_no_matches() {
        // Issue #153/#155 — the return value drives whether we restart
        // systemd-resolved. A directory with no proteus-prefixed files
        // must report `false` so revert stays quiet.
        let dir = std::env::temp_dir().join("proteus-revert-resolved-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("99-third-party.conf"), b"DNS=1.1.1.1\n").unwrap();

        // We can't easily dependency-inject the dir into
        // remove_resolved_dropins (it reads a const), so build a small
        // local mirror that exercises the same loop and helper.
        let mut warns: Vec<String> = Vec::new();
        let mut removed_any = false;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if is_proteus_resolved_dropin(&s) {
                let path = entry.path();
                let outcome = remove_file_opt(&path);
                if matches!(outcome, Ok(true)) {
                    removed_any = true;
                }
                note(&path, outcome, &mut warns);
            }
        }
        assert!(
            !removed_any,
            "no proteus-prefixed file present, removed_any should stay false"
        );
        // The third-party file must still be there afterwards.
        assert!(dir.join("99-third-party.conf").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
