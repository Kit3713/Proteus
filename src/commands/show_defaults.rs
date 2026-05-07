// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

use crate::config::Config;
use crate::exit;

pub fn run(json: bool) -> Result<u8> {
    super::render_config(&Config::default(), json)?;
    Ok(exit::SUCCESS)
}
