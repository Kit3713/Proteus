// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;

const DEFAULT_CONFIG_PATH: &str = "/etc/proteus/config.toml";

#[derive(Serialize)]
struct MissingReport<'a> {
    config_present: bool,
    path: String,
    note: &'a str,
}

pub fn run(json: bool, override_path: Option<&Path>) -> Result<u8> {
    let path: PathBuf = override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            tracing::warn!("skip {}: permission denied", path.display());
            return Ok(exit::PERMISSION_ERROR);
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    let cfg: Config =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    super::render_config(&cfg, json)?;
    Ok(exit::SUCCESS)
}
