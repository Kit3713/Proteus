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
            // Roadmap Stream 7 / E4: a config the user can't read is a
            // hard error, not a warning. The exit code (PERMISSION_ERROR)
            // is unchanged; the journal level is promoted so an operator
            // grepping `journalctl -p err` actually sees it.
            tracing::error!("skip {}: permission denied", path.display());
            return Ok(exit::PERMISSION_ERROR);
        }
        return Err(e).with_context(|| format!("stat {}", path.display()));
    }

    let cfg = Config::default_or_loaded(&path)?;
    super::render_config(&cfg, json)?;
    Ok(exit::SUCCESS)
}

#[cfg(test)]
mod tests {
    /// Roadmap Stream 7 acceptance: the success path of `show-config`
    /// emits zero `tracing::info!` events from this module. The
    /// permission-denied path now uses `error!` (E4) — that lives on
    /// the failure path so it does not violate the default-stderr
    /// discipline. We pin by asserting:
    /// 1) zero `tracing::info!` calls anywhere in the prod source;
    /// 2) `tracing::warn!` is gone (E4 promoted it to error).
    #[test]
    fn no_info_calls_and_no_warn_calls_in_prod() {
        let src = include_str!("show_config.rs");
        // Cut at the test module so this test's own assertion
        // strings don't trigger.
        let prod = src
            .split_once("\n#[cfg(test)]\n")
            .map(|(prod, _)| prod)
            .unwrap_or(src);
        let mut without_comments = String::with_capacity(prod.len());
        for line in prod.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            without_comments.push_str(line);
            without_comments.push('\n');
        }
        assert!(
            !without_comments.contains("tracing::info!"),
            "show_config must not call tracing::info!"
        );
        assert!(
            !without_comments.contains("tracing::warn!"),
            "Stream 7 / E4: show_config permission-denied must use error!, not warn!"
        );
    }
}
