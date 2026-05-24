// SPDX-License-Identifier: GPL-3.0-or-later

//! DTOs for the shared `proteus apply --json` / `proteus revert --json`
//! envelope.
//!
//! Moved out of `src/commands/apply.rs` (roadmap 1.1.1). Both the apply and
//! revert orchestrators in the binary build a [`Summary`] of per-component
//! [`ComponentReport`]s and emit it as a single JSON line; the shape is
//! unchanged from 1.0.x. The `emit_summary` writer and the private `Tally`
//! helper stay in the binary — they are behaviour, not wire types.

use serde::Serialize;

/// Per-component apply/revert outcome. Serialised kebab-case
/// (`applied` / `skipped` / `failed`) — that string is the `--json` contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Applied,
    Skipped,
    Failed,
}

/// One row in [`Summary::components`].
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ComponentReport {
    pub name: &'static str,
    pub status: Status,
    pub note: String,
}

/// Issue #343: single-line `--json` envelope shared by `proteus apply
/// --json` and `proteus revert --json`. The `command` field discriminates
/// the two, `components` mirrors the per-feature reports, `exit_code`
/// carries the same exit code the binary returns so CI / Ansible
/// consumers only have to parse stdout (no `$?` lookup needed). An
/// optional `error` field is set when the orchestrator never reached the
/// per-component fan-out (root / `--yes` / config / lock / preflight
/// gates) so wrappers can distinguish "everything failed" from "we never
/// got there."
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Summary {
    pub command: &'static str,
    pub components: Vec<ComponentReport>,
    pub exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Summary {
    pub fn new(command: &'static str, components: Vec<ComponentReport>, exit_code: u8) -> Self {
        Self {
            command,
            components,
            exit_code,
            error: None,
        }
    }

    pub fn with_error(command: &'static str, exit_code: u8, error: impl Into<String>) -> Self {
        Self {
            command,
            components: Vec::new(),
            exit_code,
            error: Some(error.into()),
        }
    }
}
