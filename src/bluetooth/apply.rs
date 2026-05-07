// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use serde::Serialize;

use super::{Adapter1Proxy, AdapterInfo};
use crate::config::BluetoothConfig;
use crate::state::State;

#[derive(Debug, Serialize)]
pub struct ApplyOutcome {
    pub hci: String,
    pub alias_before: Option<String>,
    pub alias_after: Option<String>,
    pub discoverable_after: Option<bool>,
    pub rpa_action: RpaAction,
    pub notes: Vec<String>,
    /// True when the adapter was powered off and we deliberately skipped
    /// the alias write — BlueZ rejects `Adapter1.Alias` writes on a
    /// `Powered=false` adapter, which used to fail the whole `bluetooth
    /// apply` run (issues #143/#152/#154).
    #[serde(default, skip_serializing_if = "is_false")]
    pub skipped_powered_off: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RpaAction {
    Already,
    Skipped,
    NotSupported,
}

pub async fn apply_one(
    conn: &zbus::Connection,
    info: &AdapterInfo,
    cfg: &BluetoothConfig,
    new_alias: &str,
    state: &mut State,
) -> Result<ApplyOutcome> {
    let proxy = Adapter1Proxy::builder(conn)
        .path(info.path.clone())?
        .build()
        .await
        .context("connecting to BlueZ Adapter1")?;

    capture_original_alias(state, &info.hci, info.alias.as_deref());

    let mut notes = Vec::new();
    let alias_before = proxy.alias().await.ok();

    // BlueZ refuses property writes on a powered-off adapter, surfacing the
    // failure as `org.bluez.Error.NotReady`. That used to abort the whole
    // `bluetooth apply` run when one of N adapters happened to be off — an
    // unsightly failure for what is supposed to be a best-effort sweep.
    // Skip the writes (alias + discoverable + RPA poke) and surface a note.
    if should_skip_powered_off(info) {
        notes.push("adapter is powered off; skipping alias/discoverable writes".into());
        return Ok(ApplyOutcome {
            hci: info.hci.clone(),
            alias_before: alias_before.clone(),
            alias_after: alias_before,
            discoverable_after: info.discoverable,
            rpa_action: RpaAction::Skipped,
            notes,
            skipped_powered_off: true,
        });
    }

    if cfg.generic_alias {
        proxy
            .set_alias(new_alias)
            .await
            .context("setting Adapter1.Alias")?;
    } else {
        notes.push("alias unchanged (generic_alias=false)".into());
    }

    proxy
        .set_discoverable(cfg.discoverable)
        .await
        .context("setting Adapter1.Discoverable")?;

    let rpa_action = if !cfg.ble_rpa {
        RpaAction::Skipped
    } else if !info.privacy_capable {
        notes.push("skipped (no controller privacy support)".into());
        RpaAction::NotSupported
    } else {
        // BlueZ does not expose a stable Privacy property over DBus across
        // versions; the controller manages RPA rotation when AddressType is
        // operating in random mode. We surface the current state and leave
        // explicit privacy enablement to bluetoothctl/Mgmt API.
        if info.privacy_active {
            RpaAction::Already
        } else {
            notes.push("RPA not active; controller currently advertises a public address".into());
            RpaAction::Skipped
        }
    };

    let alias_after = proxy.alias().await.ok();
    let discoverable_after = proxy.discoverable().await.ok();

    Ok(ApplyOutcome {
        hci: info.hci.clone(),
        alias_before,
        alias_after,
        discoverable_after,
        rpa_action,
        notes,
        skipped_powered_off: false,
    })
}

/// Pure helper: returns true when `apply_one` should skip mutating an
/// adapter because BlueZ would reject the writes. Split out so tests can
/// assert the policy without wiring a real DBus connection.
pub(crate) fn should_skip_powered_off(info: &AdapterInfo) -> bool {
    info.powered == Some(false)
}

pub async fn revert_one(
    conn: &zbus::Connection,
    info: &AdapterInfo,
    state: &State,
) -> Result<RevertOutcome> {
    let proxy = Adapter1Proxy::builder(conn)
        .path(info.path.clone())?
        .build()
        .await
        .context("connecting to BlueZ Adapter1")?;
    let original = state.originals.bluetooth_aliases.get(&info.hci).cloned();
    let alias_before = proxy.alias().await.ok();
    let restored = match &original {
        Some(orig) => {
            proxy
                .set_alias(orig)
                .await
                .context("restoring Adapter1.Alias")?;
            true
        }
        None => false,
    };
    Ok(RevertOutcome {
        hci: info.hci.clone(),
        alias_before,
        original,
        restored,
    })
}

#[derive(Debug, Serialize)]
pub struct RevertOutcome {
    pub hci: String,
    pub alias_before: Option<String>,
    pub original: Option<String>,
    pub restored: bool,
}

fn capture_original_alias(state: &mut State, hci: &str, alias: Option<&str>) {
    if state.originals.bluetooth_aliases.contains_key(hci) {
        return;
    }
    if let Some(a) = alias {
        state
            .originals
            .bluetooth_aliases
            .insert(hci.to_string(), a.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::OwnedObjectPath;

    fn info_with_powered(powered: Option<bool>) -> AdapterInfo {
        AdapterInfo {
            hci: "hci0".into(),
            path: OwnedObjectPath::try_from("/org/bluez/hci0").unwrap(),
            address: None,
            address_type: None,
            alias: None,
            name: None,
            discoverable: None,
            pairable: None,
            powered,
            privacy_capable: false,
            privacy_active: false,
        }
    }

    #[test]
    fn skip_when_explicitly_powered_off() {
        assert!(should_skip_powered_off(&info_with_powered(Some(false))));
    }

    #[test]
    fn proceed_when_powered_on_or_unknown() {
        // BlueZ exposes Powered on every modern controller; if it ever
        // doesn't (older daemon, weird adapter), default to attempting the
        // write rather than silently skipping every adapter.
        assert!(!should_skip_powered_off(&info_with_powered(Some(true))));
        assert!(!should_skip_powered_off(&info_with_powered(None)));
    }
}
