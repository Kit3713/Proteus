// SPDX-License-Identifier: GPL-3.0-or-later

//! Timer management: short-name -> systemd unit mapping, duration parsing,
//! and drop-in writing for the `proteus timer` subcommand.
//!
//! The CLI exposes user-friendly timer names (`rotate`, `check`, `resume`,
//! `boot`); we translate to the corresponding systemd unit name, parse
//! intervals like `30m`/`1h`/`hourly`, and emit a "managed by proteus"
//! drop-in at `/etc/systemd/system/proteus-<name>.timer.d/override.conf`.
//!
//! Mutating bits (drop-in write, daemon-reload, restart) live in
//! `crate::commands::timer`. This module is pure logic + filesystem layout
//! so it stays unit-testable.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::version;

/// Canonical Proteus timer types.
///
/// `Boot` is a oneshot service rather than a timer; we still expose it under
/// `proteus timer` so users have a single surface for "scheduled / boot
/// units Proteus owns".
pub const TIMERS: &[TimerSpec] = &[
    TimerSpec {
        short: "rotate",
        unit: "proteus-rotate.timer",
        kind: TimerKind::Timer,
        default: "2h",
        description: "Scheduled MAC rotation cadence.",
    },
    TimerSpec {
        short: "check",
        unit: "proteus-check.timer",
        kind: TimerKind::Timer,
        default: "5m",
        description: "Probe-driven rotation check interval.",
    },
    TimerSpec {
        short: "resume",
        unit: "proteus-resume.timer",
        kind: TimerKind::Timer,
        default: "off",
        description: "Rotate on resume from suspend (lands phase C).",
    },
    TimerSpec {
        short: "boot",
        unit: "proteus-boot.service",
        kind: TimerKind::BootOneshot,
        default: "boot",
        description: "Apply Proteus config + first rotation at boot.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Timer,
    BootOneshot,
}

#[derive(Debug, Clone, Copy)]
pub struct TimerSpec {
    pub short: &'static str,
    pub unit: &'static str,
    pub kind: TimerKind,
    pub default: &'static str,
    pub description: &'static str,
}

/// Resolve a short name (e.g. `rotate`) to its `TimerSpec`.
pub fn resolve(short: &str) -> Result<&'static TimerSpec> {
    TIMERS
        .iter()
        .find(|t| t.short.eq_ignore_ascii_case(short))
        .ok_or_else(|| {
            anyhow!(
                "unknown timer '{short}'; valid: {}",
                TIMERS
                    .iter()
                    .map(|t| t.short)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Parsed interval ready to render into a systemd drop-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interval {
    /// `OnUnitActiveSec=<seconds>` — fires every N seconds after the unit was
    /// last active. Right knob for "every N time".
    UnitActive { seconds: u64, original: String },
    /// `OnCalendar=<expr>` — passed through verbatim. For named cadences like
    /// `hourly`, `daily`, or full systemd calendar expressions.
    Calendar { expr: String },
}

/// Parse a user-friendly duration string into an `Interval`.
///
/// Accepted shapes:
/// - Plain duration: `30s`, `5m`, `2h`, `1d`. Becomes `OnUnitActiveSec`.
/// - Named systemd cadence: `hourly`, `daily`, `weekly`, `monthly`, `yearly`,
///   `minutely`. Passed through to `OnCalendar` unchanged.
/// - Anything else containing whitespace, a `*`, or `:` is assumed to be a
///   raw systemd calendar expression and passed through to `OnCalendar`.
pub fn parse_interval(s: &str) -> Result<Interval> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        anyhow::bail!("interval is empty");
    }
    if is_named_cadence(trimmed) {
        return Ok(Interval::Calendar {
            expr: trimmed.to_lowercase(),
        });
    }
    if looks_like_calendar_expr(trimmed) {
        return Ok(Interval::Calendar {
            expr: trimmed.to_string(),
        });
    }
    let seconds = parse_duration_seconds(trimmed)?;
    if seconds == 0 {
        anyhow::bail!("interval must be > 0 (got '{trimmed}')");
    }
    Ok(Interval::UnitActive {
        seconds,
        original: trimmed.to_string(),
    })
}

fn is_named_cadence(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "minutely"
            | "hourly"
            | "daily"
            | "weekly"
            | "monthly"
            | "quarterly"
            | "semiannually"
            | "yearly"
            | "annually"
    )
}

fn looks_like_calendar_expr(s: &str) -> bool {
    s.contains(' ') || s.contains('*') || s.contains(':')
}

fn parse_duration_seconds(s: &str) -> Result<u64> {
    // Single-suffix compact form: <n><unit>. Whitespace already trimmed.
    let bytes = s.as_bytes();
    let split_at = bytes
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(bytes.len());
    if split_at == 0 {
        anyhow::bail!("duration must start with digits (got '{s}')");
    }
    let (num, suffix) = s.split_at(split_at);
    let value: u64 = num
        .parse()
        .map_err(|e| anyhow!("invalid duration '{s}': {e}"))?;
    let mult: u64 = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60 * 60,
        "d" | "day" | "days" => 60 * 60 * 24,
        "w" | "wk" | "wks" | "week" | "weeks" => 60 * 60 * 24 * 7,
        other => anyhow::bail!(
            "unknown duration unit '{other}' in '{s}' (use s/m/h/d/w or a named cadence like 'hourly')"
        ),
    };
    value
        .checked_mul(mult)
        .ok_or_else(|| anyhow!("duration '{s}' overflows u64 seconds"))
}

/// Render the drop-in body for a parsed interval.
///
/// The leading `OnCalendar=` line is intentional: systemd *appends* timer
/// triggers from drop-ins to the unit-file ones, so we clear the unit-file
/// `OnCalendar=` first and then add our own knob.
pub fn render_dropin(interval: &Interval) -> String {
    let header = format!(
        "# managed by proteus v{version}\n# do not edit; manage via `proteus timer set ...`\n",
        version = version::VERSION
    );
    let body = match interval {
        Interval::UnitActive { seconds, original } => format!(
            "[Timer]\n# user-requested cadence: {original}\nOnCalendar=\nOnUnitActiveSec={seconds}\n"
        ),
        Interval::Calendar { expr } => {
            format!("[Timer]\nOnCalendar=\nOnCalendar={expr}\n")
        }
    };
    format!("{header}{body}")
}

/// Path to the drop-in directory for the given timer.
pub fn dropin_dir(spec: &TimerSpec) -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/{}.d", spec.unit))
}

/// Path to the drop-in file we write under that directory.
pub fn dropin_file(spec: &TimerSpec) -> PathBuf {
    dropin_dir(spec).join("override.conf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_timers() {
        assert_eq!(resolve("rotate").unwrap().unit, "proteus-rotate.timer");
        assert_eq!(resolve("check").unwrap().unit, "proteus-check.timer");
        assert_eq!(resolve("boot").unwrap().unit, "proteus-boot.service");
        assert_eq!(resolve("resume").unwrap().unit, "proteus-resume.timer");
    }

    #[test]
    fn resolve_is_case_insensitive() {
        assert_eq!(resolve("ROTATE").unwrap().short, "rotate");
        assert_eq!(resolve("Check").unwrap().short, "check");
    }

    #[test]
    fn resolve_unknown_errors_with_valid_list() {
        let err = resolve("nope").unwrap_err().to_string();
        assert!(err.contains("rotate"), "error should list rotate: {err}");
        assert!(err.contains("check"), "error should list check: {err}");
    }

    #[test]
    fn parse_seconds_minutes_hours_days() {
        assert!(matches!(
            parse_interval("30s").unwrap(),
            Interval::UnitActive { seconds: 30, .. }
        ));
        assert!(matches!(
            parse_interval("5m").unwrap(),
            Interval::UnitActive { seconds: 300, .. }
        ));
        assert!(matches!(
            parse_interval("2h").unwrap(),
            Interval::UnitActive { seconds: 7200, .. }
        ));
        assert!(matches!(
            parse_interval("1d").unwrap(),
            Interval::UnitActive { seconds: 86400, .. }
        ));
    }

    #[test]
    fn parse_named_cadence_passes_through() {
        match parse_interval("hourly").unwrap() {
            Interval::Calendar { expr } => assert_eq!(expr, "hourly"),
            other => panic!("expected Calendar, got {other:?}"),
        }
        match parse_interval("Daily").unwrap() {
            Interval::Calendar { expr } => assert_eq!(expr, "daily"),
            other => panic!("expected Calendar, got {other:?}"),
        }
    }

    #[test]
    fn parse_calendar_expr_passes_through() {
        match parse_interval("*-*-* 00/2:00:00").unwrap() {
            Interval::Calendar { expr } => assert_eq!(expr, "*-*-* 00/2:00:00"),
            other => panic!("expected Calendar, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_zero_and_garbage() {
        assert!(parse_interval("0s").is_err());
        assert!(parse_interval("").is_err());
        assert!(parse_interval("forever").is_err());
        assert!(parse_interval("5q").is_err());
    }

    #[test]
    fn render_dropin_clears_oncalendar_for_unit_active() {
        let interval = parse_interval("30m").unwrap();
        let body = render_dropin(&interval);
        assert!(body.contains("# managed by proteus"));
        assert!(body.contains("OnCalendar="));
        assert!(body.contains("OnUnitActiveSec=1800"));
    }

    #[test]
    fn render_dropin_uses_calendar_for_named() {
        let interval = parse_interval("hourly").unwrap();
        let body = render_dropin(&interval);
        assert!(body.contains("OnCalendar=\nOnCalendar=hourly\n"));
    }
}
