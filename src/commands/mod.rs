// SPDX-License-Identifier: GPL-3.0-or-later

pub mod current;
pub mod original;
pub mod show_config;
pub mod show_defaults;
pub mod status;
pub mod stub;
pub mod wiki_cmd;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;

pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, value)?;
    println!();
    Ok(())
}

pub(crate) fn render_config(cfg: &Config, json: bool) -> Result<()> {
    if json {
        print_json(cfg)
    } else {
        let rendered = toml::to_string_pretty(cfg).context("serializing config to TOML")?;
        print!("{rendered}");
        Ok(())
    }
}
