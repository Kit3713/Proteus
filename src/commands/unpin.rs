// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;

use crate::exit;
use crate::state::State;

/// Issue #391 / N12.1: `unpin` clears the persisted pin so a subsequent
/// rotation drops the operator-chosen MAC. That's a mutating change just
/// like `pin`, so we gate on `--yes` for parity with the rest of the
/// confirmation contract — wrapper scripts that depend on the gate were
/// silently no-ops before.
pub fn run(target: &str, yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'unpin' is mutating (clears the operator-chosen pin)",
        "proteus help pin",
    ) {
        return Ok(code);
    }
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
