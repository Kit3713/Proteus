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
        .with_ansi(!no_color)
        .without_time()
        .with_target(false);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init();
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
    // `Targets::from_str` already understands `target=level` directives. Bare
    // levels (`debug`) parse as a `=debug` directive against the empty target,
    // which `Targets` does not treat as a global default — so we strip a
    // leading bare-level token and feed it through `with_default` instead.
    let mut default = fallback;
    let mut directives: Vec<&str> = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !part.contains('=')
            && let Ok(level) = LevelFilter::from_str(part)
        {
            default = level;
            continue;
        }
        directives.push(part);
    }
    let targets = if directives.is_empty() {
        Targets::new()
    } else {
        Targets::from_str(&directives.join(",")).unwrap_or_else(|_| Targets::new())
    };
    targets.with_default(default)
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
}
