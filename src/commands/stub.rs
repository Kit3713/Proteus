// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

use crate::exit;

pub fn not_implemented(name: &str, phase: char, see: &str) -> Result<u8> {
    eprintln!("proteus: '{name}' is not yet implemented; targets phase {phase}. See: {see}");
    Ok(exit::NOT_IMPLEMENTED)
}
