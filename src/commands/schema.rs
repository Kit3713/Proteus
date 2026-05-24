// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus schema` — emit a JSON Schema for the `--json` outputs.
//!
//! Roadmap milestone 1.1.2. GUI/tray clients and CI wrappers parse Proteus's
//! `--json` output; this subcommand hands them a machine-readable contract so
//! they can validate against it and detect drift.
//!
//! The DTOs themselves live in `proteus-types` and derive
//! [`schemars::JsonSchema`] behind that crate's `schema` feature. This
//! command stitches one schema per output type into a single Draft-07
//! document keyed by output name (e.g. `"version"`, `"apply"`), so a single
//! `proteus schema` call documents the whole surface.
//!
//! When the binary is built `--no-default-features` (schemars dropped to save
//! binary size), this command degrades gracefully: it prints a clear note to
//! stderr and exits [`exit::SYSTEM_NOT_SUPPORTED`] rather than silently
//! disappearing from the CLI surface.

use anyhow::Result;

use crate::exit;

#[cfg(feature = "schema")]
pub fn run() -> Result<u8> {
    use anyhow::Context;
    use serde_json::{Map, Value, json};

    // One named entry per `--json` output type. The key is the logical
    // output name a consumer asks about ("which schema validates `proteus
    // version --json`?"); the value is that type's JSON Schema. Keep this
    // list in sync with the DTOs moved into `proteus-types`.
    //
    // `schema_for!` returns a self-contained schema (its own `$schema` +
    // any `definitions`), so nesting them under named keys keeps each one
    // independently valid for a consumer that pulls a single entry.
    let mut outputs: Map<String, Value> = Map::new();
    macro_rules! add {
        ($name:literal, $ty:ty) => {{
            let schema = schemars::schema_for!($ty);
            let value = serde_json::to_value(&schema)
                .with_context(|| format!("serialising schema for `{}`", $name))?;
            outputs.insert($name.to_string(), value);
        }};
    }

    // `proteus version --json` / `proteus about`.
    add!("version", proteus_types::version::VersionReport);
    // `proteus apply --json` / `proteus revert --json` (shared envelope).
    add!("apply", proteus_types::apply::Summary);
    // `proteus original --json` / `proteus state info` (state.json tree).
    add!("state.originals", proteus_types::state::Originals);
    add!("state.managed", proteus_types::state::ManagedState);
    add!(
        "state.portal_check",
        proteus_types::state::PortalCheckRecord
    );
    add!(
        "state.per_ssid_seed",
        proteus_types::state::PerSsidStateSeed
    );

    let doc = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Proteus --json outputs",
        "description": "JSON Schemas for the stable machine-readable outputs \
                        emitted by `proteus <command> --json`. Each key under \
                        `outputs` names a logical output and maps to its schema.",
        "outputs": Value::Object(outputs),
    });

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, &doc).context("writing JSON Schema document")?;
    use std::io::Write;
    handle.write_all(b"\n").context("flushing schema newline")?;
    Ok(exit::SUCCESS)
}

#[cfg(not(feature = "schema"))]
pub fn run() -> Result<u8> {
    eprintln!(
        "proteus: this binary was built without schema support \
         (`--no-default-features`); rebuild with the default `schema` feature \
         to emit the JSON Schema for --json outputs"
    );
    Ok(exit::SYSTEM_NOT_SUPPORTED)
}

#[cfg(all(test, feature = "schema"))]
mod tests {
    /// The emitted document must be valid JSON, carry the Draft-07 `$schema`,
    /// and expose a `version` output schema (the hermetic command CI
    /// validates against). Guards against an accidental shape change that
    /// would break consumers.
    #[test]
    fn schema_document_is_valid_json_with_version_output() {
        // Capture by re-running the generator logic against the same DTOs.
        // We can't easily capture stdout here, so assert the building blocks
        // the `run()` path uses produce the expected shape.
        let schema = schemars::schema_for!(proteus_types::version::VersionReport);
        let value = serde_json::to_value(&schema).expect("version schema serialises");
        // The VersionReport schema must describe the documented `--json`
        // keys so a consumer validating `proteus version --json` succeeds.
        let props = value
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("version schema has object properties");
        for key in [
            "version",
            "git_sha",
            "rustc",
            "target",
            "build_time",
            "state_schema_version",
        ] {
            assert!(props.contains_key(key), "version schema missing `{key}`");
        }
    }
}
