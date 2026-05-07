// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;

#[derive(Serialize)]
struct MissingReport<'a> {
    config_present: bool,
    path: String,
    note: &'a str,
}

pub fn run(json: bool, override_path: Option<&Path>) -> Result<u8> {
    let path = super::config_path(override_path);

    if !path.exists() {
        if json {
            super::print_json(&MissingReport {
                config_present: false,
                path: path.display().to_string(),
                note: "no config file; defaults are in effect — see `proteus show-defaults`",
            })?;
        } else {
            println!(
                "no config file at {}; using built-in defaults — see `proteus show-defaults`",
                path.display()
            );
        }
        return Ok(exit::SUCCESS);
    }
    if let Err(e) = std::fs::metadata(&path) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            tracing::warn!("skip {}: permission denied", path.display());
            return Ok(exit::PERMISSION_ERROR);
        }
        return Err(e).with_context(|| format!("stat {}", path.display()));
    }

    let cfg = Config::default_or_loaded(&path)?;
    super::render_config(&cfg, json)?;
    Ok(exit::SUCCESS)
}
