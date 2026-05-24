// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus-types` — the pure `--json` output DTOs emitted by the Proteus
//! CLI.
//!
//! Roadmap milestone 1.1.1 carved these serde `Serialize` types out of the
//! `proteus` binary so the wire contract lives in one dependency-light place.
//! The binary re-exports every type from here (e.g. `pub use
//! proteus_types::state::Originals as Originals`), so existing
//! `use crate::state::Originals` paths keep compiling and the emitted JSON is
//! byte-identical to 1.0.x.
//!
//! # Invariants
//!
//! - Every field name, `#[serde(...)]` attribute, and default behaviour here
//!   is part of the public `--json` contract that GUIs and CI wrappers parse.
//!   Do not rename or reorder fields without treating it as a breaking change.
//! - `Serialize`/`Display` always carry the **real** value (the JSON
//!   contract). Redacting belongs to `Debug` in the main crate's
//!   identifier types, never here — these are pure DTOs holding already-
//!   serialized `String`/scalar forms.
//!
//! # The `schema` feature (milestone 1.1.2)
//!
//! With the default-on `schema` feature, every DTO additionally derives
//! [`schemars::JsonSchema`] so the `proteus schema` subcommand can emit a
//! JSON Schema describing the `--json` outputs. The feature is removable via
//! `--no-default-features` for size-constrained builds; the binary keeps a
//! graceful fallback when it is off.

pub mod apply;
pub mod state;
pub mod version;
