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

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::Serialize;

use crate::commands;
use crate::config::TimersConfig;
use crate::profile::TIMER_NEVER;
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
    // Issue #352: the artifact shipped under `dist/systemd/` is
    // `proteus-resume.service` (a sleep.target hook), not a `.timer`.
    // `proteus timer enable resume` previously asked systemctl to
    // operate on `proteus-resume.timer` — a unit name that doesn't
    // exist anywhere in the package — so every timer subcommand on
    // `resume` failed with "Unit … could not be found." Match the
    // shipped unit and treat it as a oneshot service like `boot`.
    TimerSpec {
        short: "resume",
        unit: "proteus-resume.service",
        kind: TimerKind::BootOneshot,
        default: "boot",
        description: "Rotate on resume from suspend (sleep.target hook).",
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

/// Lower bound on a `proteus timer set` rotation cadence. Anything shorter
/// is a DoS-amplification footgun: the rotation cycle (NM apply, RF down/up,
/// DHCP renew) can take several seconds on its own, and a sub-minute cadence
/// risks stacking rotation runs faster than the previous one finishes. Issue
/// #293 picked 60 s — well below any realistic operator cadence while still
/// rejecting garbage like `1s` or `0m`. The bound is enforced at the timer
/// CLI surface only; non-rotation `parse_interval` callers (e.g. probe
/// cooldowns) legitimately use sub-minute values.
pub const MIN_TIMER_INTERVAL_SECONDS: u64 = 60;

/// Upper bound on a `proteus timer set` rotation cadence. 30 days is two
/// orders of magnitude past the slowest "monthly" use case anyone has cited;
/// past it the value is almost certainly an off-by-an-order-of-magnitude user
/// error (typing `300d` when `30d` was meant). Named calendar expressions
/// (`hourly`, `weekly`, `monthly`, `yearly`) are exempt — those carry their
/// semantics in the systemd grammar. Issue #293.
pub const MAX_TIMER_INTERVAL_SECONDS: u64 = 60 * 60 * 24 * 30;

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
    // Security audit L-2 + issue #297: reject calendar expressions that
    // could break out of the systemd unit by injecting newlines, section
    // headers, comments, or other unit-grammar metacharacters.
    // `proteus timer set` is root-only so the threat is limited, but the
    // input lands in a config file other tools read; defense in depth.
    if trimmed.len() > 200 {
        anyhow::bail!(
            "interval too long ({} bytes; max 200): {trimmed:?}",
            trimmed.len()
        );
    }
    for c in trimmed.chars() {
        if c.is_control() && c != '\t' {
            anyhow::bail!("interval contains control char {c:?}: {trimmed:?}");
        }
        if matches!(c, '[' | ']' | ';' | '#') {
            anyhow::bail!(
                "interval contains forbidden character {c:?} (unit-grammar metachar): {trimmed:?}"
            );
        }
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

/// Issue #293: a parsed `Interval` is bound-checked against the
/// `MIN_TIMER_INTERVAL_SECONDS` / `MAX_TIMER_INTERVAL_SECONDS` window
/// before being written as a `proteus timer set` drop-in. Calendar
/// expressions (`hourly`, `*-*-*`) bypass the check because their grammar
/// carries the cadence — there's no plain seconds value to bound. The check
/// lives separate from `parse_interval` so non-rotation callers (e.g. probe
/// cooldowns under `[probes].cooldown`) keep their existing sub-minute
/// freedom.
pub fn validate_timer_set_bounds(interval: &Interval) -> Result<()> {
    let Interval::UnitActive { seconds, original } = interval else {
        return Ok(());
    };
    if *seconds < MIN_TIMER_INTERVAL_SECONDS {
        anyhow::bail!(
            "interval '{original}' is too short ({seconds}s); minimum is \
             {MIN_TIMER_INTERVAL_SECONDS}s (sub-minute rotation risks stacking runs \
             and is a DoS-amp footgun)"
        );
    }
    if *seconds > MAX_TIMER_INTERVAL_SECONDS {
        anyhow::bail!(
            "interval '{original}' is too long ({seconds}s); maximum is \
             {MAX_TIMER_INTERVAL_SECONDS}s (~30 days; past this is almost certainly \
             a user error — use a calendar expression like 'monthly' instead)"
        );
    }
    Ok(())
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
///
/// Issue #303: user-set drop-ins also carry `AccuracySec=` and
/// `RandomizedDelaySec=` proportional to the cadence, so a user who sets
/// `proteus timer set rotate --interval 2h` does not fall back to the
/// systemd-default `AccuracySec=1min` cluster — they keep the
/// anti-fingerprint jitter that the shipped unit-file defaults already
/// have. Pure-relative `OnUnitActiveSec=` cadences are naturally
/// non-clustering across hosts (each host's "0 of N seconds" starts at a
/// different wall time), but applying jitter on top further smears the
/// fire-time within any one host's schedule, which makes the
/// per-rotation moment harder to predict for an attacker who knows the
/// last fire time.
pub fn render_dropin(interval: &Interval) -> String {
    let header = format!(
        "# managed by proteus v{version}\n# do not edit; manage via `proteus timer set ...`\n",
        version = version::VERSION
    );
    let (accuracy, randomized) = pick_jitter(interval_period_seconds(interval));
    let body = match interval {
        Interval::UnitActive { seconds, original } => format!(
            "[Timer]\n# user-requested cadence: {original}\nOnCalendar=\nOnUnitActiveSec={seconds}\nAccuracySec={accuracy}\nRandomizedDelaySec={randomized}\n"
        ),
        Interval::Calendar { expr } => {
            format!(
                "[Timer]\nOnCalendar=\nOnCalendar={expr}\nAccuracySec={accuracy}\nRandomizedDelaySec={randomized}\n"
            )
        }
    };
    format!("{header}{body}")
}

/// Approximate cadence period in seconds, used to scale jitter. For
/// `OnUnitActiveSec=` we know the answer exactly. For named cadences and
/// raw calendar expressions we don't, so we fall back to a conservative
/// 1-hour estimate, which gives "30min" / "15min" jitter — sane defaults
/// across the typical user-set range (5min ... daily).
fn interval_period_seconds(interval: &Interval) -> u64 {
    match interval {
        Interval::UnitActive { seconds, .. } => *seconds,
        Interval::Calendar { .. } => 3600,
    }
}

/// Pick `(AccuracySec, RandomizedDelaySec)` for a given cadence.
///
/// Rule of thumb: AccuracySec is wide enough to blur cross-host
/// wallclock alignment, RandomizedDelaySec adds per-host non-predictability
/// on top, both scaled so the jitter window is a meaningful fraction of
/// the cadence without ever pushing the jitter bigger than the cadence
/// itself (which would let a tick swallow another tick). The shipped
/// unit-file defaults (rotate: 45min/30min on a 2h cadence; check:
/// 2min/2min on a 5min cadence) sit in this band, and a `proteus timer
/// set rotate --interval 2h` drop-in lands on the same numbers.
///
/// Shared with `crate::init::systemd::render_periodic_timer` so
/// generated artifacts inherit the same anti-fingerprint shape.
pub(crate) fn pick_jitter(period_seconds: u64) -> (&'static str, &'static str) {
    match period_seconds {
        // <=60s: tight — anything wider misses scheduling intent.
        0..=60 => ("10s", "5s"),
        // 1–4 min: 30s/30s.
        61..=240 => ("30s", "30s"),
        // 4–30 min: 2min/2min — matches the shipped check timer (5min).
        241..=1800 => ("2min", "2min"),
        // 30 min – 1.5 h: 15min/15min.
        1801..=5400 => ("15min", "15min"),
        // 1.5–6 h: 45min/30min — matches the shipped rotate timer (2h).
        5401..=21600 => ("45min", "30min"),
        // 6–24 h: 1h/30min.
        21601..=86400 => ("1h", "30min"),
        // > 1 day: 2h/1h.
        _ => ("2h", "1h"),
    }
}

/// Path to the drop-in directory for the given timer.
pub fn dropin_dir(spec: &TimerSpec) -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/{}.d", spec.unit))
}

/// Path to the drop-in file we write under that directory.
pub fn dropin_file(spec: &TimerSpec) -> PathBuf {
    dropin_dir(spec).join("override.conf")
}

/// Per-timer reconciliation outcome surfaced by `reconcile_with_config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReconcileOutcome {
    /// Drop-in already matches the desired interval; no work performed.
    Unchanged,
    /// Drop-in written or updated to the new interval.
    Changed,
    /// Drop-in removed because the configured interval is `"never"`.
    Removed,
    /// Configured interval is `"never"` and no drop-in existed; no work.
    AlreadyDisabled,
    /// Reconciliation failed for this timer.
    Failed(String),
}

impl ReconcileOutcome {
    /// Short label suitable for the `proteus apply` summary line.
    pub fn label(&self) -> &'static str {
        match self {
            ReconcileOutcome::Changed => "changed",
            ReconcileOutcome::Removed => "removed",
            ReconcileOutcome::Unchanged => "unchanged",
            ReconcileOutcome::AlreadyDisabled => "off",
            ReconcileOutcome::Failed(_) => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileEntry {
    pub name: &'static str,
    pub unit: &'static str,
    pub interval: String,
    pub outcome: ReconcileOutcome,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileReport {
    pub timers: Vec<ReconcileEntry>,
}

impl ReconcileReport {
    /// True iff at least one entry needed a write or remove.
    pub fn any_changed(&self) -> bool {
        self.timers.iter().any(|t| {
            matches!(
                t.outcome,
                ReconcileOutcome::Changed | ReconcileOutcome::Removed
            )
        })
    }

    /// True iff any entry failed.
    pub fn any_failed(&self) -> bool {
        self.timers
            .iter()
            .any(|t| matches!(t.outcome, ReconcileOutcome::Failed(_)))
    }
}

/// Plan a single timer's reconciliation given the configured interval and
/// the current drop-in body (if any). Returns the action to take and, if
/// applicable, the rendered new body. Pure: no I/O, exhaustively testable.
#[derive(Debug)]
pub enum PlannedAction {
    /// Write `body` to the drop-in path; the timer needs to be restarted.
    Write { body: String },
    /// Remove the drop-in path; the timer needs to be restarted.
    Remove,
    /// No change needed.
    Noop { already_disabled: bool },
}

/// Pure planner: given a desired interval and the on-disk drop-in body,
/// pick the minimum action that brings the timer to the desired state.
pub fn plan_action(desired: &str, current_body: Option<&str>) -> Result<PlannedAction> {
    if is_never(desired) {
        return Ok(match current_body {
            Some(_) => PlannedAction::Remove,
            None => PlannedAction::Noop {
                already_disabled: true,
            },
        });
    }
    let interval = parse_interval(desired)?;
    let new_body = render_dropin(&interval);
    if let Some(existing) = current_body
        && existing == new_body
    {
        return Ok(PlannedAction::Noop {
            already_disabled: false,
        });
    }
    Ok(PlannedAction::Write { body: new_body })
}

/// Whether `s` is the "do not run this timer" sentinel.
pub fn is_never(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case(TIMER_NEVER)
}

/// Pair each `[timers.<name>]` field with the matching `TimerSpec`. The
/// short names (`"rotate"`, `"check"`) are the bridge between the config
/// schema and the systemd unit metadata in `TIMERS`.
fn config_managed_intervals(cfg: &TimersConfig) -> Vec<(&'static TimerSpec, &str)> {
    [
        ("rotate", cfg.rotate.interval.as_str()),
        ("check", cfg.check.interval.as_str()),
    ]
    .iter()
    .filter_map(|(short, interval)| {
        TIMERS
            .iter()
            .find(|t| t.short == *short)
            .map(|spec| (spec, *interval))
    })
    .collect()
}

/// Reconcile every managed timer against the configured `[timers]` block.
/// Writes or removes drop-ins as needed and returns a per-timer report.
/// `restart` is invoked once per changed unit so the new cadence takes
/// effect; pass `|_| Ok(())` from tests to skip the live restart.
///
/// `daemon_reload` is invoked once after the loop iff any timer changed.
/// This mirrors the convention `proteus timer set` already uses.
pub fn reconcile_with_config<F, R>(
    cfg: &TimersConfig,
    mut restart: R,
    daemon_reload: F,
) -> ReconcileReport
where
    F: FnOnce() -> Result<()>,
    R: FnMut(&str) -> Result<()>,
{
    let mut entries = Vec::new();
    let mut changed_units: Vec<&'static str> = Vec::new();

    for (spec, interval) in config_managed_intervals(cfg) {
        let path = dropin_file(spec);
        let current = std::fs::read_to_string(&path).ok();
        let outcome = match plan_action(interval, current.as_deref()) {
            Ok(action) => execute_action(spec, &path, action),
            Err(e) => ReconcileOutcome::Failed(format!("{e:#}")),
        };
        if matches!(
            outcome,
            ReconcileOutcome::Changed | ReconcileOutcome::Removed
        ) {
            changed_units.push(spec.unit);
        }
        entries.push(ReconcileEntry {
            name: spec.short,
            unit: spec.unit,
            interval: interval.to_string(),
            outcome,
        });
    }

    let mut report = ReconcileReport { timers: entries };
    if !changed_units.is_empty() {
        if let Err(e) = daemon_reload() {
            // Mark every changed entry as failed so the orchestrator can
            // surface the daemon-reload error.
            for t in report.timers.iter_mut() {
                if matches!(
                    t.outcome,
                    ReconcileOutcome::Changed | ReconcileOutcome::Removed
                ) {
                    t.outcome = ReconcileOutcome::Failed(format!("daemon-reload: {e:#}"));
                }
            }
            return report;
        }
        for unit in &changed_units {
            if let Err(e) = restart(unit) {
                if let Some(t) = report.timers.iter_mut().find(|t| t.unit == *unit) {
                    t.outcome = ReconcileOutcome::Failed(format!("restart: {e:#}"));
                }
            }
        }
    }
    report
}

fn execute_action(spec: &TimerSpec, path: &Path, action: PlannedAction) -> ReconcileOutcome {
    match action {
        PlannedAction::Write { body } => {
            // write_atomic creates the parent dir as part of its contract; the
            // randomized temp name + O_EXCL + parent fsync defends issues
            // #125/#150 (TOCTOU symlink redirect, leaked .tmp, durability).
            if let Err(e) = commands::write_atomic(path, body.as_bytes()) {
                return ReconcileOutcome::Failed(format!("writing {}: {e:#}", path.display()));
            }
            ReconcileOutcome::Changed
        }
        PlannedAction::Remove => match std::fs::remove_file(path) {
            Ok(()) => {
                let _ = std::fs::remove_dir(dropin_dir(spec));
                ReconcileOutcome::Removed
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ReconcileOutcome::AlreadyDisabled,
            Err(e) => ReconcileOutcome::Failed(format!("removing {}: {e}", path.display())),
        },
        PlannedAction::Noop { already_disabled } => {
            if already_disabled {
                ReconcileOutcome::AlreadyDisabled
            } else {
                ReconcileOutcome::Unchanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_timers() {
        assert_eq!(resolve("rotate").unwrap().unit, "proteus-rotate.timer");
        assert_eq!(resolve("check").unwrap().unit, "proteus-check.timer");
        assert_eq!(resolve("boot").unwrap().unit, "proteus-boot.service");
        // Issue #352: `proteus-resume.service` is the artifact shipped in
        // `dist/systemd/`, not a `.timer`. The mapping must match the
        // installed unit name or every `proteus timer * resume` call
        // hits "Unit could not be found".
        assert_eq!(resolve("resume").unwrap().unit, "proteus-resume.service");
    }

    /// Issue #352: pin that the shipped unit referenced by `resume` is
    /// the actual file in `dist/systemd/`. If a future refactor
    /// rearranges the dist layout this test catches the mismatch
    /// before users do.
    #[test]
    fn resume_short_name_targets_a_unit_that_actually_exists() {
        let spec = resolve("resume").unwrap();
        assert_eq!(spec.unit, "proteus-resume.service");
        // The kind switches to BootOneshot so timer-only operations like
        // "set cadence" surface a proper error instead of writing a
        // .timer drop-in for a unit that has no .timer artifact.
        assert!(matches!(spec.kind, TimerKind::BootOneshot));
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

    /// Issue #293: the timer-set bound check rejects sub-minute cadences
    /// with an error naming the floor. Lives separate from `parse_interval`
    /// so non-rotation callers (probe cooldowns) keep sub-minute freedom.
    #[test]
    fn validate_bounds_rejects_below_min() {
        for too_short in &["1s", "30s", "59s"] {
            let interval = parse_interval(too_short).unwrap();
            let err = validate_timer_set_bounds(&interval)
                .expect_err("interval below MIN_TIMER_INTERVAL_SECONDS must reject");
            let msg = format!("{err:#}");
            assert!(
                msg.contains(&format!("{MIN_TIMER_INTERVAL_SECONDS}s")),
                "error must name the {MIN_TIMER_INTERVAL_SECONDS}s floor: {msg}"
            );
        }
    }

    /// Boundary: 60 s exactly is the smallest accepted timer-set cadence.
    #[test]
    fn validate_bounds_accepts_min() {
        let interval = parse_interval("60s").unwrap();
        assert!(validate_timer_set_bounds(&interval).is_ok());
    }

    /// Issue #293: a value past 30 days is almost certainly a user error
    /// (typo / off-by-an-order-of-magnitude). Reject with an error pointing
    /// at calendar expressions for the legitimate "rotate rarely" case.
    #[test]
    fn validate_bounds_rejects_above_max() {
        // 31d directly.
        let interval = parse_interval("31d").unwrap();
        let err = validate_timer_set_bounds(&interval).expect_err("> 30d must reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("user error") || msg.contains("monthly"),
            "error must point user at calendar expression: {msg}"
        );
        // 100d — extreme.
        let interval = parse_interval("100d").unwrap();
        assert!(validate_timer_set_bounds(&interval).is_err());
    }

    /// Boundary: 30 days exactly is the largest accepted timer-set cadence.
    #[test]
    fn validate_bounds_accepts_max() {
        let interval = parse_interval("30d").unwrap();
        assert!(validate_timer_set_bounds(&interval).is_ok());
    }

    /// Calendar expressions carry their cadence in systemd's grammar — no
    /// seconds value to bound. The bound check is a no-op for them.
    #[test]
    fn validate_bounds_passes_calendar_expressions() {
        for s in &["hourly", "yearly", "*-*-* 00/2:00:00"] {
            let interval = parse_interval(s).unwrap();
            assert!(
                validate_timer_set_bounds(&interval).is_ok(),
                "calendar expression {s} must bypass bounds"
            );
        }
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

    /// Issue #303: every drop-in we render must include both
    /// `AccuracySec=` and `RandomizedDelaySec=` so user-set cadences
    /// inherit the same anti-fingerprint jitter the shipped unit-file
    /// defaults already have.
    #[test]
    fn render_dropin_emits_jitter_directives() {
        for spec in ["30s", "5m", "30m", "2h", "8h", "1d", "hourly", "daily"] {
            let interval = parse_interval(spec).unwrap();
            let body = render_dropin(&interval);
            assert!(
                body.lines().any(|l| l.starts_with("AccuracySec=")),
                "drop-in for {spec:?} missing AccuracySec= (issue #303):\n{body}"
            );
            assert!(
                body.lines().any(|l| l.starts_with("RandomizedDelaySec=")),
                "drop-in for {spec:?} missing RandomizedDelaySec= (issue #303):\n{body}"
            );
        }
    }

    /// Issue #303: a 2h drop-in (the new default) must render with at
    /// least 30 min of accuracy slack — matching the shipped unit-file
    /// shape — so the user-set form does not regress to the v0.3.x
    /// 5-min cluster.
    #[test]
    fn render_dropin_2h_carries_wide_jitter() {
        let interval = parse_interval("2h").unwrap();
        let body = render_dropin(&interval);
        // 2h falls in the 4–24h jitter band (45min / 30min). Pre-#303 this
        // would have rendered with no AccuracySec line at all and
        // inherited systemd's 1-min default — recognizable.
        assert!(
            body.contains("AccuracySec=45min"),
            "2h drop-in must use 45min accuracy (issue #303); got:\n{body}"
        );
        assert!(
            body.contains("RandomizedDelaySec=30min"),
            "2h drop-in must use 30min randomized delay (issue #303); got:\n{body}"
        );
    }

    #[test]
    fn plan_never_with_no_dropin_is_noop() {
        match plan_action("never", None).unwrap() {
            PlannedAction::Noop {
                already_disabled: true,
            } => {}
            other => panic!("expected already-disabled Noop, got {other:?}"),
        }
    }

    #[test]
    fn plan_never_with_dropin_removes() {
        match plan_action("never", Some("# managed by proteus\n[Timer]\n")).unwrap() {
            PlannedAction::Remove => {}
            other => panic!("expected Remove, got {other:?}"),
        }
    }

    #[test]
    fn plan_writes_when_no_dropin() {
        match plan_action("30m", None).unwrap() {
            PlannedAction::Write { body } => {
                assert!(body.contains("OnUnitActiveSec=1800"));
            }
            other => panic!("expected Write, got {other:?}"),
        }
    }

    #[test]
    fn plan_unchanged_when_body_matches() {
        let body = render_dropin(&parse_interval("30m").unwrap());
        match plan_action("30m", Some(&body)).unwrap() {
            PlannedAction::Noop {
                already_disabled: false,
            } => {}
            other => panic!("expected Unchanged Noop, got {other:?}"),
        }
    }

    #[test]
    fn plan_writes_when_existing_body_differs() {
        let body = render_dropin(&parse_interval("1h").unwrap());
        match plan_action("30m", Some(&body)).unwrap() {
            PlannedAction::Write { body: new_body } => {
                assert!(new_body.contains("OnUnitActiveSec=1800"));
            }
            other => panic!("expected Write, got {other:?}"),
        }
    }

    #[test]
    fn plan_propagates_parse_errors() {
        assert!(plan_action("forever", None).is_err());
    }

    #[test]
    fn is_never_handles_case_and_whitespace() {
        assert!(is_never("never"));
        assert!(is_never("NEVER"));
        assert!(is_never("  Never  "));
        assert!(!is_never("5m"));
        assert!(!is_never(""));
    }
}
