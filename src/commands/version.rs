// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus version` / `proteus about` — print build provenance.
//!
//! Issue #376 / roadmap Milestone 6 follow-up: CI and GUI wrappers need a
//! stable, machine-readable way to ask the binary which build it is. The
//! values are all stamped at build time by `build.rs` via `cargo:rustc-env`
//! lines, so this command is a read of `env!(...)` constants plus the in-tree
//! schema-version constants that consumers use for forward-compatibility
//! checks.
//!
//! `about` is a thin alias for `version` without `--json` — same surface,
//! friendlier name for first-time users browsing `--help`.

use anyhow::Result;
use serde::Serialize;

use crate::exit;
use crate::state::CURRENT_SCHEMA_VERSION as STATE_SCHEMA_VERSION;
use crate::version::VERSION;

/// Short git SHA at build time, or the literal `"unknown"` when `.git/` is
/// absent (source tarballs, reproducible-build sandboxes) and
/// `PROTEUS_GIT_SHA` wasn't pre-set. See `build.rs::emit_build_metadata`.
const GIT_SHA: &str = env!("PROTEUS_GIT_SHA");

/// ISO-8601 UTC timestamp of the build. Honours `SOURCE_DATE_EPOCH` so
/// reproducible-build pipelines get a deterministic value.
const BUILD_TIME: &str = env!("PROTEUS_BUILD_TIME");

/// `rustc -V` output (single line), or `"unknown"` when the probe failed.
const RUSTC_VERSION: &str = env!("PROTEUS_RUSTC_VERSION");

/// Cargo target triple, e.g. `"x86_64-unknown-linux-gnu"`.
const TARGET: &str = env!("PROTEUS_TARGET");

#[derive(Debug, Serialize)]
struct VersionReport {
    version: &'static str,
    git_sha: &'static str,
    rustc: &'static str,
    target: &'static str,
    build_time: &'static str,
    /// `state.json` schema version this binary writes. Wrappers use it to
    /// decide whether to migrate before invoking a mutator.
    state_schema_version: u32,
}

fn build_report() -> VersionReport {
    VersionReport {
        version: VERSION,
        git_sha: GIT_SHA,
        rustc: RUSTC_VERSION,
        target: TARGET,
        build_time: BUILD_TIME,
        state_schema_version: STATE_SCHEMA_VERSION,
    }
}

pub fn run(json: bool) -> Result<u8> {
    let report = build_report();
    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn print_human(r: &VersionReport) {
    println!("proteus {}", r.version);
    println!("  git sha:      {}", r.git_sha);
    println!("  build time:   {}", r.build_time);
    println!("  rustc:        {}", r.rustc);
    println!("  target:       {}", r.target);
    println!("  state schema: v{}", r.state_schema_version);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The package version constant is the `Cargo.toml` value; bumping the
    /// version shouldn't require touching this test, but the prefix is a
    /// useful sanity check that the env-stamp didn't get lost.
    #[test]
    fn report_carries_cargo_pkg_version() {
        let r = build_report();
        assert_eq!(r.version, env!("CARGO_PKG_VERSION"));
    }

    /// Every stamped value must be a non-empty string. `"unknown"` is a
    /// legitimate fallback emitted by `build.rs`; a bare empty string would
    /// be a packaging regression worth catching.
    #[test]
    fn stamped_fields_are_non_empty() {
        let r = build_report();
        assert!(!r.git_sha.is_empty(), "git sha must be non-empty");
        assert!(!r.rustc.is_empty(), "rustc must be non-empty");
        assert!(!r.target.is_empty(), "target must be non-empty");
        assert!(!r.build_time.is_empty(), "build time must be non-empty");
    }

    /// JSON serialisation must produce a flat object with the documented
    /// keys. Wrappers parse this — drift in field naming is a breaking
    /// change.
    #[test]
    fn json_carries_documented_keys() {
        let r = build_report();
        let v = serde_json::to_value(&r).unwrap();
        for key in [
            "version",
            "git_sha",
            "rustc",
            "target",
            "build_time",
            "state_schema_version",
        ] {
            assert!(v.get(key).is_some(), "missing JSON key: {key}");
        }
        assert_eq!(
            v["state_schema_version"].as_u64(),
            Some(STATE_SCHEMA_VERSION as u64)
        );
    }

    /// The build timestamp must look like ISO-8601 UTC even when the
    /// `SOURCE_DATE_EPOCH` reproducible-build path is taken. Spot-check the
    /// shape (`YYYY-MM-DDTHH:MM:SSZ`).
    #[test]
    fn build_time_is_iso8601_utc_shape() {
        let r = build_report();
        // 20 chars: 4-2-2 T 2:2:2 Z.
        assert_eq!(r.build_time.len(), 20, "got: {}", r.build_time);
        assert!(r.build_time.ends_with('Z'), "got: {}", r.build_time);
        assert_eq!(&r.build_time[4..5], "-");
        assert_eq!(&r.build_time[7..8], "-");
        assert_eq!(&r.build_time[10..11], "T");
        assert_eq!(&r.build_time[13..14], ":");
        assert_eq!(&r.build_time[16..17], ":");
    }
}
