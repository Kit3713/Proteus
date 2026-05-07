// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;

use crate::exit;
use crate::state::State;

pub fn run(target: &str, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    let mut changed = false;
    if let Some(rec) = state.managed.interfaces.get_mut(target)
        && rec.pinned.is_some()
    {
        rec.pinned = None;
        changed = true;
    }
    if let Some(rec) = state.managed.connections.get_mut(target)
        && rec.pinned.is_some()
    {
        rec.pinned = None;
        changed = true;
    }

    if !changed {
        eprintln!("proteus: no pin found for '{target}'");
        return Ok(exit::GENERIC_ERROR);
    }

    state.save(&state_path)?;
    println!("unpinned {target}");
    Ok(exit::SUCCESS)
}
