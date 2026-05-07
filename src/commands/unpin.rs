// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result};

use crate::exit;
use crate::nm;
use crate::state::State;

pub fn run(target: &str, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    let mut changed = false;
    if let Some(rec) = state.managed.interfaces.get_mut(target)
        && rec.pinned.is_some()
    {
        rec.pinned = None;
        changed = true;
    }
    // Issue #124: managed.connections is uuid-keyed; accept the uuid
    // directly, otherwise look it up by id via NM.
    if let Some(uuid) = resolve_connection_uuid(target)?
        && let Some(rec) = state.managed.connections.get_mut(&uuid)
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

/// Map `target` (uuid or id) to the canonical uuid. A uuid is returned
/// verbatim; an id is resolved via NM. Returns `Ok(None)` if NM is
/// unavailable or no profile matches — the caller treats that as "no
/// connection-keyed pin to clear" rather than a hard failure.
fn resolve_connection_uuid(target: &str) -> Result<Option<String>> {
    if super::looks_like_uuid(target) {
        return Ok(Some(target.to_string()));
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")
    {
        Ok(rt) => rt,
        Err(_) => return Ok(None),
    };
    rt.block_on(async {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(_) => return Ok::<Option<String>, anyhow::Error>(None),
        };
        match nm::apply::find_connection_by_id(&conn, target).await {
            Ok((_, s)) => Ok(nm::dhcp::connection_uuid(&s)),
            Err(_) => Ok(None),
        }
    })
}
