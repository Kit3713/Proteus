// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus uninstall` — full removal hatch.
//!
//! Best-effort revert + systemd unit teardown + binary removal, with
//! `--purge` to also wipe `/etc/proteus/` and `/var/lib/proteus/`. Tolerant
//! of partial removal: every step records a warning rather than aborting
//! so the system ends up in the most-removed state we can reach.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::exit;

const DEFAULT_CONFIG_DIR: &str = "/etc/proteus";
const DEFAULT_STATE_DIR: &str = "/var/lib/proteus";
const DEFAULT_SYSTEMD_DIR: &str = "/etc/systemd/system";

/// Systemd units `install.sh` creates. Timers first so they stop firing
/// before we tear down the services they trigger. `pub(crate)` so the
/// dry-run preview can iterate the same list without drift.
pub(crate) const UNITS: &[&str] = &[
    "proteus-rotate.timer",
    "proteus-check.timer",
    "proteus-resume.timer",
    "proteus-rotate.service",
    "proteus-check.service",
    "proteus-resume.service",
    "proteus-boot.service",
];

/// Drop-ins Proteus writes outside `/etc/proteus/`. Mirrors `wiki/uninstall.md`.
pub(crate) const EXTERNAL_DROPINS: &[&str] = &[
    "/etc/sysctl.d/95-proteus.conf",
    "/etc/sysctl.d/96-proteus-ipv6.conf",
    "/etc/systemd/timesyncd.conf.d/10-proteus.conf",
];

const RESOLVED_DROPIN_DIR: &str = "/etc/systemd/resolved.conf.d";
const RESOLVED_DROPIN_PREFIX: &str = "10-proteus-";

/// Public entry point invoked from `cli::run`.
pub fn run(purge: bool, yes: bool) -> Result<u8> {
    if let Err(code) = super::require_yes(yes, "uninstall is destructive", "proteus help uninstall")
    {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }

    let layout = Layout::from_env();
    let mut warns: Vec<String> = Vec::new();

    // Issue #126: hold the state lock during revert so a concurrent
    // mutating run can't race us. Released before the (optional) purge step
    // since purge wipes the lock-file directory itself.
    {
        let _lock = match super::acquire_state_lock_or_print(None) {
            Ok(g) => g,
            Err(code) => return Ok(code),
        };
        revert_best_effort(&mut warns);
    }
    teardown_units(&layout, &mut warns);
    let _ = run_quiet("systemctl", &["daemon-reload"]);

    let binary = current_binary_path();
    note(&binary, remove_file_opt(&binary), &mut warns);
    let _ = run_quiet("semanage", &["fcontext", "-d", &binary.to_string_lossy()]);

    if purge {
        for dir in [&layout.config_dir, &layout.state_dir] {
            note(dir, remove_dir_opt(dir), &mut warns);
        }
    } else {
        println!(
            "kept {} and {}",
            layout.config_dir.display(),
            layout.state_dir.display()
        );
    }

    if warns.is_empty() {
        println!("proteus uninstall: complete");
    } else {
        eprintln!("proteus uninstall: {} warning(s):", warns.len());
        for w in &warns {
            eprintln!("  {w}");
        }
        eprintln!("see `proteus wiki uninstall` for manual cleanup");
    }
    Ok(exit::SUCCESS)
}

/// Install-layout paths. Environment-overridable for sandboxed tests.
struct Layout {
    config_dir: PathBuf,
    state_dir: PathBuf,
    systemd_dir: PathBuf,
}

impl Layout {
    fn from_env() -> Self {
        Self {
            config_dir: env_path("PROTEUS_CONFIG_DIR", DEFAULT_CONFIG_DIR),
            state_dir: env_path("PROTEUS_STATE_DIR", DEFAULT_STATE_DIR),
            systemd_dir: env_path("PROTEUS_SYSTEMD_DIR", DEFAULT_SYSTEMD_DIR),
        }
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    match std::env::var_os(key) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(default),
    }
}

/// Where the running binary lives. Falls back to `/usr/local/bin/proteus` if
/// `current_exe` returns a dev/test path we shouldn't delete.
pub(crate) fn current_binary_path() -> PathBuf {
    match std::env::current_exe() {
        Ok(p) => resolve_binary_path(&p),
        Err(_) => PathBuf::from("/usr/local/bin/proteus"),
    }
}

/// Decide whether the given `current_exe` is a real install or a dev/test
/// path we should ignore. Pulled out so the policy is unit-testable.
pub(crate) fn resolve_binary_path(current_exe: &Path) -> PathBuf {
    let s = current_exe.to_string_lossy();
    let is_dev = s.contains("/target/")
        || s.starts_with("/tmp/")
        || s.starts_with("/var/tmp/")
        || s.contains("/.cargo/");
    if is_dev {
        PathBuf::from("/usr/local/bin/proteus")
    } else {
        current_exe.to_path_buf()
    }
}

type Outcome = std::io::Result<bool>;

fn remove_file_opt(p: &Path) -> Outcome {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn remove_dir_opt(p: &Path) -> Outcome {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn note(p: &Path, outcome: Outcome, warns: &mut Vec<String>) {
    let d = p.display();
    match outcome {
        Ok(true) => println!("removed {d}"),
        Ok(false) => {}
        Err(e) => warns.push(format!("{d}: {e}")),
    }
}

fn revert_best_effort(warns: &mut Vec<String>) {
    if let Err(e) = super::hostname::revert(None) {
        warns.push(format!("hostname: {e:#}"));
    }
    if let Err(e) = super::bluetooth_cmd::revert(None) {
        warns.push(format!("bluetooth: {e:#}"));
    }
    if let Err(e) = super::ipv6::revert(true, None) {
        warns.push(format!("ipv6: {e:#}"));
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
        if s.starts_with(RESOLVED_DROPIN_PREFIX) && s.ends_with(".conf") {
            let path = entry.path();
            note(&path, remove_file_opt(&path), warns);
        }
    }
}

fn teardown_units(layout: &Layout, warns: &mut Vec<String>) {
    for unit in UNITS {
        // disable --now stops + disables in one call. Missing units exit
        // nonzero — not a warning; we tolerate partial installs.
        let _ = run_quiet("systemctl", &["disable", "--now", unit]);

        let path = layout.systemd_dir.join(unit);
        note(&path, remove_file_opt(&path), warns);

        let dropin_dir = layout.systemd_dir.join(format!("{unit}.d"));
        note(&dropin_dir, remove_dir_opt(&dropin_dir), warns);
    }
}

/// Run a command quietly. Returns Err with a single-line message on
/// non-zero exit so callers can collect warnings without aborting.
fn run_quiet(program: &str, args: &[&str]) -> Result<(), String> {
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
    fn dev_target_path_falls_back_to_default_install() {
        // `cargo run` and `cargo test` exec from target/. We must never
        // delete those — fall back to the canonical install path.
        let dev = Path::new("/home/dev/proteus/target/release/proteus");
        assert_eq!(
            resolve_binary_path(dev),
            PathBuf::from("/usr/local/bin/proteus")
        );

        let tmp = Path::new("/tmp/cargo-target/proteus");
        assert_eq!(
            resolve_binary_path(tmp),
            PathBuf::from("/usr/local/bin/proteus")
        );
    }

    #[test]
    fn install_path_passes_through() {
        let installed = Path::new("/usr/local/bin/proteus");
        assert_eq!(resolve_binary_path(installed), installed.to_path_buf());
        let alt = Path::new("/usr/bin/proteus");
        assert_eq!(resolve_binary_path(alt), alt.to_path_buf());
    }

    #[test]
    fn layout_honors_env_overrides() {
        // Safety: env mutation needs unsafe in 2024 edition; the keys here
        // are unique to this test so cross-test bleed is unlikely.
        unsafe {
            std::env::set_var("PROTEUS_CONFIG_DIR", "/sandbox/etc/proteus");
            std::env::set_var("PROTEUS_STATE_DIR", "/sandbox/var/lib/proteus");
            std::env::set_var("PROTEUS_SYSTEMD_DIR", "/sandbox/etc/systemd/system");
        }
        let layout = Layout::from_env();
        assert_eq!(layout.config_dir, PathBuf::from("/sandbox/etc/proteus"));
        assert_eq!(layout.state_dir, PathBuf::from("/sandbox/var/lib/proteus"));
        assert_eq!(
            layout.systemd_dir,
            PathBuf::from("/sandbox/etc/systemd/system")
        );

        unsafe {
            std::env::remove_var("PROTEUS_CONFIG_DIR");
            std::env::remove_var("PROTEUS_STATE_DIR");
            std::env::remove_var("PROTEUS_SYSTEMD_DIR");
        }
        let defaulted = Layout::from_env();
        assert_eq!(defaulted.config_dir, PathBuf::from(DEFAULT_CONFIG_DIR));
        assert_eq!(defaulted.state_dir, PathBuf::from(DEFAULT_STATE_DIR));
        assert_eq!(defaulted.systemd_dir, PathBuf::from(DEFAULT_SYSTEMD_DIR));
    }
}
