// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::exit;
use crate::mac::Mac;
use crate::nm::{self, DeviceInfo};
use crate::state::State;

pub fn run(target: &str, mac: Option<&str>, yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'pin' is mutating (writes state.json)",
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

    let mac = match mac {
        Some(m) => Some(parse_mac(m)?),
        None => None,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let outcome = rt.block_on(async { resolve_and_pin(target, mac, &mut state).await });

    match outcome {
        Ok(message) => {
            state.save(&state_path)?;
            println!("{message}");
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: pin failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

async fn resolve_and_pin(target: &str, mac: Option<Mac>, state: &mut State) -> Result<String> {
    let conn = zbus::Connection::system()
        .await
        .context("connecting to system DBus (NetworkManager required)")?;
    let devices = nm::list_devices(&conn).await?;

    if let Some(dev) = devices.iter().find(|d| d.interface == target) {
        let pin_mac = resolve_mac_for_device(&conn, dev, mac).await?;
        let entry = state
            .managed
            .interfaces
            .entry(target.to_string())
            .or_default();
        entry.pinned = Some(pin_mac.to_string());
        if entry.current_mac.is_none() {
            entry.current_mac = dev.hw_address.clone();
        }
        return Ok(format!("pinned interface {target} to {pin_mac}"));
    }

    if let Ok((path, _)) = nm::apply::find_connection_by_id(&conn, target).await {
        let pin_mac = match mac {
            Some(m) => m,
            None => {
                let owner_dev = devices
                    .iter()
                    .find(|d| d.connections.iter().any(|p| p == &path));
                match owner_dev {
                    Some(dev) => resolve_mac_for_device(&conn, dev, None).await?,
                    None => {
                        return Err(anyhow!(
                            "connection '{target}' is not currently bound to a device; pass --mac"
                        ));
                    }
                }
            }
        };
        let entry = state
            .managed
            .connections
            .entry(target.to_string())
            .or_default();
        entry.pinned = Some(pin_mac.to_string());
        return Ok(format!("pinned connection {target} to {pin_mac}"));
    }

    Err(anyhow!(
        "no NetworkManager interface or connection profile named '{target}'"
    ))
}

async fn resolve_mac_for_device(
    conn: &zbus::Connection,
    dev: &DeviceInfo,
    mac: Option<Mac>,
) -> Result<Mac> {
    if let Some(m) = mac {
        return Ok(m);
    }
    if let Some(path) = dev.connections.first()
        && let Some(s) = nm::apply::read_cloned_mac(conn, path, dev.kind).await?
    {
        return parse_mac(&s);
    }
    if let Some(hw) = &dev.hw_address {
        return parse_mac(hw);
    }
    Err(anyhow!(
        "no current MAC available for {}; pass --mac",
        dev.interface
    ))
}

fn parse_mac(s: &str) -> Result<Mac> {
    s.parse::<Mac>()
        .with_context(|| format!("parsing MAC '{s}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #118 regression: `pin` used to ignore `--yes` and always
    /// mutate `state.json`. The fix gates the call before the root check
    /// and any state load so a missing `--yes` is a fast, observable bail.
    /// We assert on the exit code AND that no state file is written
    /// (calling with a path that points into an empty temp dir; the gate
    /// returns before the state code path runs).
    #[test]
    fn pin_without_yes_returns_confirmation_required_and_does_not_touch_state() {
        let dir = std::env::temp_dir().join("proteus-pin-noyes-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");

        let code = run("wlan0", None, false, Some(&state_path)).unwrap();
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
        assert!(
            !state_path.exists(),
            "pin without --yes must not create state.json"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
