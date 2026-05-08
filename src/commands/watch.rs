// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared `--watch` loop for read-only commands.
//!
//! Roadmap Milestone 6 (CLI ergonomics): re-runs a read-only command on a
//! fixed interval and clears the screen between renders. The wiring on
//! `proteus status` / `current` / `session` is the integration follow-up;
//! this module ships the helper plus the duration parser independently
//! so a future PR can flip the flag without touching this file.
//!
//! The loop is deliberately simple — no diff rendering, no curses. Two
//! ANSI escapes (`ESC[2J ESC[H`) clear the terminal between iterations;
//! under `NO_COLOR` or a piped stdout the escapes are skipped.

use std::io::{self, Write};
use std::time::Duration;

/// Default refresh cadence. Picked to match the smallest meaningful
/// rotation cadence the timer module exposes (60s = the tightest
/// systemd-timer interval Proteus surfaces). Slow enough to read,
/// fast enough that a 30-second-cooldown rotation is visible inside
/// two refreshes.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);

/// Run `body` repeatedly with a `delay` sleep between calls. Clears the
/// screen between iterations when stdout is a TTY and `no_color` is
/// false.
///
/// The body runs every tick. If it returns non-zero or errors, the
/// watcher reports it and continues — operators usually want to keep
/// watching even when the underlying check is briefly unhappy (a daemon
/// restart, a transient DBus blip).
pub fn run<F>(delay: Duration, no_color: bool, mut body: F) -> anyhow::Result<u8>
where
    F: FnMut() -> anyhow::Result<u8>,
{
    let interactive = !no_color && stdout_is_tty();
    loop {
        if interactive {
            // ESC[2J clears the screen, ESC[H homes the cursor.
            print!("\x1b[2J\x1b[H");
            io::stdout().flush().ok();
        }
        // The body's exit code is observed for the side effect (its own
        // print/eprint output) but not propagated — the watch loop keeps
        // running even when a tick is briefly unhappy. Ctrl-C is the
        // only loop terminator on the happy path.
        match body() {
            Ok(_) => {}
            Err(e) => eprintln!("proteus: watch tick error: {e:#}"),
        }
        std::thread::sleep(delay);
    }
}

fn stdout_is_tty() -> bool {
    // SAFETY: `libc::isatty` reads only the descriptor's terminal flag;
    // no side effects.
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
}

/// Smallest cadence accepted by `--watch`. Issue #349 / CL1+CL7: a
/// zero-second interval (or a sub-millisecond one) busy-loops the
/// CPU at the rate of clock-poll. We reject below this floor at
/// parse time so the loop never sees a zero-sleep tick.
const MIN_INTERVAL: Duration = Duration::from_millis(1);

/// Parse a `--watch --interval` value (e.g. `2s`, `500ms`, `1m`) into a
/// Duration. Returns the default when the input is empty. Errors on
/// garbage and on intervals below [`MIN_INTERVAL`].
///
/// Roadmap Milestone 6: keeps the surface small (s / ms / m). Anything
/// beyond `m` belongs in cron, not in `--watch`.
///
/// Issue #349 / CL1: `0s` previously parsed to `Duration::ZERO` and the
/// run-loop happily called `thread::sleep(0)` every tick, pegging a core
/// for the lifetime of the watch. CL7 mirrors the same fix at the
/// sub-millisecond floor — `Duration::from_micros(...)` is technically
/// representable but is a CPU-burn footgun for the same reason. We reject
/// both with a single sentinel below.
pub fn parse_interval(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(DEFAULT_INTERVAL);
    }
    let parsed = if let Some(num) = s.strip_suffix("ms") {
        Duration::from_millis(num.parse()?)
    } else if let Some(num) = s.strip_suffix('s') {
        Duration::from_secs(num.parse()?)
    } else if let Some(num) = s.strip_suffix('m') {
        let mins: u64 = num.parse()?;
        Duration::from_secs(mins * 60)
    } else {
        anyhow::bail!("expected a duration like `2s`, `500ms`, or `1m`; got '{s}'");
    };
    if parsed < MIN_INTERVAL {
        anyhow::bail!(
            "--interval must be >= 1ms (got '{s}'); zero / sub-ms sleeps would peg a CPU core"
        );
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interval_accepts_seconds() {
        assert_eq!(parse_interval("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_interval("60s").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn parse_interval_accepts_milliseconds() {
        assert_eq!(parse_interval("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_interval("1ms").unwrap(), Duration::from_millis(1));
    }

    /// Issue #349 / CL1: zero-second interval would peg a CPU core via
    /// `thread::sleep(0)`. The parser must reject it loudly so the loop
    /// never gets to call `body()` on a hot loop.
    #[test]
    fn parse_interval_rejects_zero_seconds() {
        let e = parse_interval("0s").expect_err("0s must be rejected");
        let msg = format!("{e:#}");
        assert!(
            msg.contains(">= 1ms"),
            "diagnostic should mention the floor; got {msg:?}"
        );
    }

    /// CL7: same rationale at the millisecond floor — `0ms` is a busy
    /// loop in disguise.
    #[test]
    fn parse_interval_rejects_zero_milliseconds() {
        assert!(parse_interval("0ms").is_err());
    }

    /// CL7: minutes resolve to seconds resolve to >= 1ms, so any non-zero
    /// minute count is fine. Zero minutes is the trap-case.
    #[test]
    fn parse_interval_rejects_zero_minutes() {
        assert!(parse_interval("0m").is_err());
    }

    #[test]
    fn parse_interval_accepts_minutes() {
        assert_eq!(parse_interval("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_interval("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn parse_interval_empty_yields_default() {
        assert_eq!(parse_interval("").unwrap(), DEFAULT_INTERVAL);
        assert_eq!(parse_interval("  ").unwrap(), DEFAULT_INTERVAL);
    }

    #[test]
    fn parse_interval_rejects_garbage() {
        assert!(parse_interval("garbage").is_err());
        assert!(parse_interval("2h").is_err()); // hours not supported — use 120m
        assert!(parse_interval("2").is_err()); // no unit
    }

    #[test]
    fn default_interval_is_two_seconds() {
        // Roadmap notes the cadence should outpace a 30-second-cooldown
        // rotation; 2s gives ~15 frames before the rotation window
        // closes. Pin so a future tweak past 5s gets caught.
        assert!(DEFAULT_INTERVAL <= Duration::from_secs(5));
        assert!(DEFAULT_INTERVAL >= Duration::from_secs(1));
    }
}
