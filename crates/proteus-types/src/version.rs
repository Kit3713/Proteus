// SPDX-License-Identifier: GPL-3.0-or-later

//! DTO for `proteus version --json` / `proteus about`.

use serde::Serialize;

/// Build-provenance report emitted by `proteus version --json`.
///
/// Moved out of `src/commands/version.rs` (roadmap 1.1.1). The binary fills
/// every field from `env!(...)` stamps that `build.rs` writes plus the
/// in-tree state schema constant, then serialises this struct verbatim — the
/// `--json` shape is unchanged from 1.0.x.
///
/// Fields use `&'static str` because the binary's values are all compile-time
/// constants; that keeps the struct allocation-free on the hot path.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VersionReport {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub rustc: &'static str,
    pub target: &'static str,
    pub build_time: &'static str,
    /// `state.json` schema version this binary writes. Wrappers use it to
    /// decide whether to migrate before invoking a mutator.
    pub state_schema_version: u32,
}
