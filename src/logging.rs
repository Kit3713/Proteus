// SPDX-License-Identifier: GPL-3.0-or-later

//! Logging setup. We deliberately do not pull in `tracing-subscriber`'s
//! `env-filter` feature: it depends on `regex-automata` + `regex-syntax` and
//! pulls roughly 175 KB into the stripped release binary. We support the same
//! `RUST_LOG` directives we document — `RUST_LOG=debug` and
//! `RUST_LOG=proteus=trace` style targets — through `tracing_subscriber::filter::Targets`,
//! which uses a tiny hand-rolled parser and ships with the `fmt` feature for
//! free.
//!
//! See `wiki/cli.md` Logging section for the user-facing surface.

use std::str::FromStr;

use tracing::Level;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn init(verbose: u8, quiet: u8, no_color: bool) {
    let default_level = level_from_counts(verbose, quiet);
    let filter = build_filter(default_level);

    // JOURNAL_STREAM is set by systemd when the process is launched as a unit.
    // Prefer journald there so timer-driven runs are auto-correlated.
    if std::env::var_os("JOURNAL_STREAM").is_some()
        && let Ok(layer) = tracing_journald::layer()
    {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init();
        return;
    }

    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(!no_color && !no_color_env())
        .without_time()
        .with_target(false);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init();
}

/// Honor the POSIX `NO_COLOR` convention (https://no-color.org): any
/// non-empty value disables ANSI styling, regardless of the `--no-color`
/// flag. Empty `NO_COLOR=""` is treated as unset, matching the spec.
fn no_color_env() -> bool {
    no_color_from(std::env::var_os("NO_COLOR").as_deref())
}

fn no_color_from(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Parse `RUST_LOG` into a `Targets` filter. Falls back to the requested
/// `default_level` when the environment variable is unset or unparseable.
///
/// Supported syntax (a strict subset of `EnvFilter`):
///
/// * `RUST_LOG=debug` — global default level
/// * `RUST_LOG=proteus=trace` — single-target override
/// * `RUST_LOG=proteus=debug,zbus=warn` — comma-separated overrides
fn build_filter(default_level: Level) -> Targets {
    let default = LevelFilter::from_level(default_level);
    match std::env::var("RUST_LOG") {
        Ok(s) if !s.is_empty() => parse_rust_log(&s, default),
        _ => Targets::new().with_default(default),
    }
}

fn parse_rust_log(s: &str, fallback: LevelFilter) -> Targets {
    // Each directive parses independently so a malformed `bogus=invalid`
    // does not poison the otherwise-valid `zbus=warn` next to it. Bare
    // levels (`debug`) update the default; `target=level` directives
    // accumulate into the `Targets` filter. Bad directives are warned
    // about and skipped — tracing itself isn't initialized yet here, so
    // we route the warning through stderr.
    let mut default = fallback;
    let mut targets = Targets::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !part.contains('=') {
            match LevelFilter::from_str(part) {
                Ok(level) => default = level,
                Err(_) => warn_bad_directive(part),
            }
            continue;
        }
        match Targets::from_str(part) {
            Ok(t) => targets.extend(t),
            Err(_) => warn_bad_directive(part),
        }
    }
    targets.with_default(default)
}

fn warn_bad_directive(part: &str) {
    eprintln!("proteus: RUST_LOG: ignoring invalid directive '{part}'");
}

fn level_from_counts(verbose: u8, quiet: u8) -> Level {
    let v = i32::from(verbose) - i32::from(quiet);
    match v {
        i32::MIN..=-2 => Level::ERROR,
        -1 => Level::WARN,
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_from_counts_clamps() {
        assert_eq!(level_from_counts(0, 5), Level::ERROR);
        assert_eq!(level_from_counts(0, 1), Level::WARN);
        assert_eq!(level_from_counts(0, 0), Level::INFO);
        assert_eq!(level_from_counts(1, 0), Level::DEBUG);
        assert_eq!(level_from_counts(5, 0), Level::TRACE);
    }

    #[test]
    fn parse_rust_log_bare_level() {
        let t = parse_rust_log("debug", LevelFilter::INFO);
        assert!(t.would_enable("anything", &Level::DEBUG));
        assert!(!t.would_enable("anything", &Level::TRACE));
    }

    #[test]
    fn parse_rust_log_targeted() {
        let t = parse_rust_log("proteus=trace,zbus=warn", LevelFilter::INFO);
        assert!(t.would_enable("proteus", &Level::TRACE));
        assert!(t.would_enable("zbus", &Level::WARN));
        assert!(!t.would_enable("zbus", &Level::INFO));
        assert!(t.would_enable("other", &Level::INFO));
        assert!(!t.would_enable("other", &Level::DEBUG));
    }

    #[test]
    fn parse_rust_log_mixed() {
        // Bare level becomes the default; targeted directives layer on top.
        let t = parse_rust_log("debug,zbus=warn", LevelFilter::INFO);
        assert!(t.would_enable("proteus", &Level::DEBUG));
        assert!(t.would_enable("zbus", &Level::WARN));
        assert!(!t.would_enable("zbus", &Level::INFO));
    }

    #[test]
    fn parse_rust_log_empty_falls_back() {
        let t = parse_rust_log("", LevelFilter::WARN);
        assert!(t.would_enable("x", &Level::WARN));
        assert!(!t.would_enable("x", &Level::INFO));
    }

    #[test]
    fn parse_rust_log_skips_bad_directive_keeps_valid_ones() {
        // The motivating case from issue #132: a single bad directive in
        // the middle of an otherwise-valid list must not nuke the rest.
        let t = parse_rust_log("zbus=warn,bogus=invalid", LevelFilter::INFO);
        assert!(t.would_enable("zbus", &Level::WARN));
        assert!(!t.would_enable("zbus", &Level::INFO));
        // Untouched targets follow the fallback default.
        assert!(t.would_enable("other", &Level::INFO));
        assert!(!t.would_enable("other", &Level::DEBUG));
    }

    #[test]
    fn parse_rust_log_skips_bad_bare_level_keeps_valid_targeted() {
        let t = parse_rust_log("nonsense,zbus=warn", LevelFilter::INFO);
        // Targeted directive survives even though the bare level was junk.
        assert!(t.would_enable("zbus", &Level::WARN));
        assert!(!t.would_enable("zbus", &Level::INFO));
    }

    #[test]
    fn no_color_from_treats_unset_and_empty_as_color_allowed() {
        assert!(!no_color_from(None));
        assert!(!no_color_from(Some(std::ffi::OsStr::new(""))));
    }

    #[test]
    fn no_color_from_treats_any_nonempty_value_as_color_disabled() {
        assert!(no_color_from(Some(std::ffi::OsStr::new("1"))));
        assert!(no_color_from(Some(std::ffi::OsStr::new("yes"))));
        assert!(no_color_from(Some(std::ffi::OsStr::new("0"))));
    }
}
