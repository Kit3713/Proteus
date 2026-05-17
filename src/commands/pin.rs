// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use crate::display::display_safe;
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
    // Issue #292: validate the user-supplied --mac shape BEFORE any expensive
    // setup (root check, state lock, DBus connection). On a host without
    // NetworkManager, deferring validation past the DBus connect surfaced a
    // confusing "DBus I/O error" instead of the obvious "invalid MAC" the
    // operator needs to see. `Mac::from_str` was hardened in #284 — reuse it.
    let mac = match mac.map(parse_mac).transpose() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("proteus: invalid --mac value: {e:#}");
            return Ok(exit::CONFIG_ERROR);
        }
    };
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
        // Issue #364: stamp the set-at timestamp so `proteus pin list`
        // can surface when each pin was authored. We restamp on every
        // pin (including overwrite) — the freshest mutation wins.
        entry.pinned_at = Some(super::now_iso8601());
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
                            "connection '{target}' is not currently bound to a device; pass --mac; see proteus wiki mac-recipes"
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
        entry.pinned_at = Some(super::now_iso8601());
        return Ok(format!("pinned connection {target} to {pin_mac}"));
    }

    Err(anyhow!(
        "no NetworkManager interface or connection profile named '{target}'; see proteus wiki mac-recipes"
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
        "no current MAC available for {}; pass --mac; see proteus wiki mac-recipes",
        dev.interface
    ))
}

fn parse_mac(s: &str) -> Result<Mac> {
    s.parse::<Mac>()
        .with_context(|| format!("parsing MAC '{s}'"))
}

/// Issue #364: read-only `proteus pin list` — enumerate every interface
/// and connection that currently has a `pinned` MAC in `state.json`.
///
/// Output order: interfaces first, then connections. Within each kind
/// targets sort alphabetically (BTreeMap iteration order). Targets come
/// from NM-controlled state, so the human renderer goes through
/// `display_safe` to keep hostile interface / connection names from
/// painting ANSI through a privileged print site.
pub fn run_list(json: bool, state_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    // Reads are best-effort: a corrupt or absent state.json yields an
    // empty list rather than an error so `pin list` mirrors the other
    // read-only commands' shape.
    let state = State::load_or_default(&state_path).unwrap_or_default();

    let interfaces = state
        .managed
        .interfaces
        .iter()
        .filter_map(|(target, rec)| Some((target, rec.pinned.as_ref()?, rec.pinned_at.as_deref())))
        .map(|(target, mac, set_at)| PinListEntry {
            target: target.clone(),
            kind: "interface",
            mac: mac.clone(),
            set_at: set_at.map(str::to_owned),
        });
    let connections = state
        .managed
        .connections
        .iter()
        .filter_map(|(target, rec)| Some((target, rec.pinned.as_ref()?, rec.pinned_at.as_deref())))
        .map(|(target, mac, set_at)| PinListEntry {
            target: target.clone(),
            kind: "connection",
            mac: mac.clone(),
            set_at: set_at.map(str::to_owned),
        });
    let entries: Vec<PinListEntry> = interfaces.chain(connections).collect();

    if json {
        super::print_json(&entries)?;
    } else if entries.is_empty() {
        println!("(no pinned targets)");
    } else {
        for e in &entries {
            let set_at = e.set_at.as_deref().unwrap_or("unknown");
            println!(
                "{} -> {}  (set: {})",
                display_safe(&e.target),
                e.mac,
                set_at
            );
        }
    }
    Ok(exit::SUCCESS)
}

#[derive(Debug, Serialize)]
struct PinListEntry {
    target: String,
    /// `"interface"` or `"connection"` — surfaced in JSON so wrappers
    /// can distinguish the two pin namespaces (NM interface name vs NM
    /// connection profile id).
    kind: &'static str,
    mac: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    set_at: Option<String>,
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

    /// Issue #292 regression: `pin --yes --mac <garbage>` used to defer MAC
    /// validation past `zbus::Connection::system()`; on a host without
    /// NetworkManager that surfaced as "DBus I/O error" instead of "invalid
    /// MAC". The fix parses up front so malformed input rejects with
    /// `CONFIG_ERROR` before any DBus / NM / state work.
    #[test]
    fn pin_with_malformed_mac_fast_fails_before_dbus() {
        let dir = std::env::temp_dir().join("proteus-pin-bad-mac-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");

        let code = run("wlan0", Some("not-a-mac"), true, Some(&state_path)).unwrap();
        assert_eq!(
            code,
            exit::CONFIG_ERROR,
            "malformed --mac must yield CONFIG_ERROR (65), not GENERIC_ERROR"
        );
        assert!(
            !state_path.exists(),
            "pin with bad --mac must not create state.json"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #364: `pin list` against a fresh / absent state.json yields
    /// `SUCCESS` and no panic — read-only commands must never error on a
    /// cold install.
    #[test]
    fn pin_list_on_missing_state_returns_success() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-pin-list-empty-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");

        let code = run_list(false, Some(&state_path)).unwrap();
        assert_eq!(code, exit::SUCCESS);
        assert_eq!(
            run_list(true, Some(&state_path)).unwrap(),
            exit::SUCCESS,
            "json form must succeed on cold install"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #364: when interfaces and connections both carry pins,
    /// `pin list` reports each of them. We seed `state.json` directly
    /// (the `pin` write path needs a live DBus, which the unit tests
    /// don't have) and exercise the read-side end-to-end.
    #[test]
    fn pin_list_enumerates_both_kinds() {
        use crate::state::{ConnectionRecord, InterfaceRecord, State};

        let dir = std::env::temp_dir().join(format!(
            "proteus-pin-list-mix-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");

        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".to_string(),
            InterfaceRecord {
                pinned: Some("aa:bb:cc:dd:ee:ff".to_string()),
                pinned_at: Some("2026-05-17T00:00:00Z".to_string()),
                ..Default::default()
            },
        );
        // Use an RFC-4122-shaped key so the migration ladder's UUID
        // retain-filter keeps the connection entry after load.
        s.managed.connections.insert(
            "aabbccdd-eeff-1122-3344-556677889900".to_string(),
            ConnectionRecord {
                pinned: Some("11:22:33:44:55:66".to_string()),
                pinned_at: None,
                ..Default::default()
            },
        );
        s.save(&state_path).unwrap();

        // Human renderer succeeds; we don't assert exact text shape but
        // `(no pinned targets)` would be a regression.
        let code = run_list(false, Some(&state_path)).unwrap();
        assert_eq!(code, exit::SUCCESS);
        let code = run_list(true, Some(&state_path)).unwrap();
        assert_eq!(code, exit::SUCCESS);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
