// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus logs` — tail journald across every Proteus systemd unit + the
//! NM dispatcher syslog tag.
//!
//! Thin shell over `journalctl` so operators have one command to read every
//! line Proteus emits — boot, rotate, check, resume, events, and the
//! NetworkManager dispatcher script (`logger -t proteus-dispatcher` in
//! `dist/networkmanager/dispatcher.d/01-proteus`). Read-only; degrades
//! cleanly when `journalctl` isn't on PATH or systemd isn't running.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::exit;

/// Marker that systemd creates when running. Same probe `timer` uses.
const SYSTEMD_MARKER: &str = "/run/systemd/system";

/// Every systemd unit Proteus ships under `dist/systemd/`. Kept as a flat
/// `&[&str]` so adding a unit means appending one line; the journalctl
/// shell-out turns each entry into a `-u <unit>` argument.
const PROTEUS_UNITS: &[&str] = &[
    "proteus-boot.service",
    "proteus-check.service",
    "proteus-check.timer",
    "proteus-events.service",
    "proteus-resume.service",
    "proteus-rotate.service",
    "proteus-rotate.timer",
];

/// Syslog tags emitted by Proteus components that don't run as systemd
/// units. The NM dispatcher script tags `logger -t proteus-dispatcher` so
/// its lines land in journald under SYSLOG_IDENTIFIER even though the
/// dispatcher itself runs as a NetworkManager child process.
const PROTEUS_SYSLOG_TAGS: &[&str] = &["proteus-dispatcher"];

pub fn run(lines: u32, follow: bool, since: Option<&str>, json: bool) -> Result<u8> {
    if let Some(code) = require_systemd() {
        return Ok(code);
    }
    if !journalctl_available() {
        eprintln!("proteus: journalctl not found on PATH; install systemd or skip this command");
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }

    let mut cmd = Command::new("journalctl");
    cmd.arg("--no-pager");

    for unit in PROTEUS_UNITS {
        cmd.args(["-u", unit]);
    }
    for tag in PROTEUS_SYSLOG_TAGS {
        cmd.args(["-t", tag]);
    }

    let lines_str = lines.to_string();
    cmd.args(["-n", &lines_str]);

    if follow {
        cmd.arg("-f");
    }
    if let Some(s) = since {
        cmd.args(["--since", s]);
    }
    if json {
        cmd.args(["--output", "json"]);
    }

    let status = cmd.status().context("invoking journalctl")?;
    if !status.success() {
        return Ok(exit::GENERIC_ERROR);
    }
    Ok(exit::SUCCESS)
}

fn require_systemd() -> Option<u8> {
    if Path::new(SYSTEMD_MARKER).is_dir() {
        None
    } else {
        eprintln!("proteus: systemd not detected (missing {SYSTEMD_MARKER})");
        Some(exit::SYSTEM_NOT_SUPPORTED)
    }
}

/// Check `journalctl --version` exit status. Cheap, side-effect free, and
/// the canonical "is this binary callable" probe other Proteus subcommands
/// use for their external tools.
fn journalctl_available() -> bool {
    Command::new("journalctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Mirror the per-subcommand `Logs` variant args here so `--help` and
    /// flag parsing can be exercised without going through the full
    /// top-level `Cli`. Keeping the wrap small means the test only covers
    /// the surface this module owns.
    #[derive(Parser, Debug)]
    struct Wrap {
        #[arg(long, short = 'f')]
        follow: bool,
        #[arg(long, short = 'n', default_value_t = 50)]
        lines: u32,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
    }

    #[test]
    fn help_exits_zero() {
        // clap renders `--help` as a `DisplayHelp` error kind, which the
        // binary's `try_parse_from` path lifts into exit code 0. Confirm
        // the kind is the help-display variant so the dispatch in
        // `cli::run` does the same.
        let err = Wrap::try_parse_from(["proteus-logs", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn parses_short_flags() {
        let w = Wrap::try_parse_from(["proteus-logs", "-n", "10", "-f"]).unwrap();
        assert_eq!(w.lines, 10);
        assert!(w.follow);
        assert!(!w.json);
        assert!(w.since.is_none());
    }

    #[test]
    fn parses_since_and_json() {
        let w = Wrap::try_parse_from(["proteus-logs", "--since", "1h ago", "--json"]).unwrap();
        assert_eq!(w.since.as_deref(), Some("1h ago"));
        assert!(w.json);
    }

    /// Without systemd present, `run` short-circuits with
    /// `SYSTEM_NOT_SUPPORTED`. The probe lives on real filesystems
    /// (containerized CI lacks `/run/systemd/system`) so this is the
    /// "no journald" smoke check.
    #[test]
    fn run_without_systemd_returns_system_not_supported() {
        if Path::new(SYSTEMD_MARKER).is_dir() {
            // Real systemd box — the call would try to shell out. Skip.
            return;
        }
        let code = run(5, false, None, false).unwrap();
        assert_eq!(code, exit::SYSTEM_NOT_SUPPORTED);
    }
}
