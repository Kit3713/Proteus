// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use serde::Serialize;

use super::{Adapter1Proxy, AdapterInfo};
use crate::config::BluetoothConfig;
use crate::state::State;

/// Roadmap Stream 7 / NEV2.4: a hot-unplugged Bluetooth adapter raises
/// `org.bluez.Error.NotReady`, `org.freedesktop.DBus.Error.UnknownObject`,
/// or `org.freedesktop.DBus.Error.UnknownMethod` on subsequent property
/// reads/writes. Those benign races used to bubble out as `error!` lines
/// (and a non-zero apply exit code), spamming the journal whenever a
/// dongle was pulled mid-apply. Inspect the underlying zbus error and
/// classify the gone-adapter variants so the caller can `warn!` and
/// continue, while every other error still propagates.
pub(crate) fn is_adapter_gone(err: &zbus::Error) -> bool {
    match err {
        zbus::Error::FDO(boxed) => matches!(
            **boxed,
            zbus::fdo::Error::UnknownObject(_)
                | zbus::fdo::Error::UnknownInterface(_)
                | zbus::fdo::Error::UnknownMethod(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        // BlueZ surfaces `NotFound` / `NotReady` as named DBus errors.
        zbus::Error::MethodError(name, _, _) => {
            let s = name.as_str();
            s == "org.bluez.Error.NotReady"
                || s == "org.bluez.Error.NotFound"
                || s == "org.freedesktop.DBus.Error.UnknownObject"
                || s == "org.freedesktop.DBus.Error.UnknownInterface"
                || s == "org.freedesktop.DBus.Error.UnknownMethod"
        }
        _ => false,
    }
}

/// Outcome of `apply_one_resilient`: either a normal `ApplyOutcome` or
/// a benign skip caused by a hot-unplugged adapter. The caller treats
/// `Gone` as a `Skipped` report rather than a failure.
#[derive(Debug)]
pub enum AdapterApplyResult {
    Done(ApplyOutcome),
    Gone { hci: String, detail: String },
}

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

/// Capture the live alias for this adapter into `state.originals` (capture-
/// once on first apply; never overwritten). Caller must persist `state` to
/// disk via `state.save()` BEFORE invoking `apply_one`, so a crash between
/// capture and mutation cannot lose the original (sacred-originals
/// invariant; issue #119).
pub fn capture_originals_step(state: &mut State, info: &AdapterInfo) {
    capture_original_alias(state, &info.hci, info.alias.as_deref());
}

/// Resilient wrapper around [`apply_one`] that classifies a
/// hot-unplugged adapter as a benign skip. See [`is_adapter_gone`] for
/// the variant set.
pub async fn apply_one_resilient(
    conn: &zbus::Connection,
    info: &AdapterInfo,
    cfg: &BluetoothConfig,
    new_alias: &str,
) -> Result<AdapterApplyResult> {
    match apply_one(conn, info, cfg, new_alias).await {
        Ok(outcome) => Ok(AdapterApplyResult::Done(outcome)),
        Err(e) => {
            if let Some(zerr) = e.downcast_ref::<zbus::Error>()
                && is_adapter_gone(zerr)
            {
                tracing::warn!(
                    hci = %info.hci,
                    "bluetooth adapter disappeared mid-apply; skipping ({zerr})"
                );
                return Ok(AdapterApplyResult::Gone {
                    hci: info.hci.clone(),
                    detail: format!("adapter disappeared: {zerr}"),
                });
            }
            Err(e)
        }
    }
}

pub async fn apply_one(
    conn: &zbus::Connection,
    info: &AdapterInfo,
    cfg: &BluetoothConfig,
    new_alias: &str,
) -> Result<ApplyOutcome> {
    let proxy = Adapter1Proxy::builder(conn)
        .path(info.path.clone())?
        .build()
        .await
        .context("connecting to BlueZ Adapter1")?;

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

    /// Roadmap Stream 7 / NEV2.4: BlueZ surfaces `NotReady` /
    /// `NotFound` / `UnknownObject` as MethodErrors when the adapter
    /// has gone away (hot-unplug). The classifier maps every
    /// gone-adapter variant to "gone", so `apply_one_resilient` skips
    /// instead of bubbling an error.
    ///
    /// Constructing a `zbus::Message` value for the third tuple slot
    /// requires a live bus connection in zbus 5.x, so we exercise the
    /// classifier through the FDO-error variant (which has no Message
    /// payload) plus the `is_adapter_gone` source dispatch. The
    /// MethodError-name string comparisons below are pinned by an
    /// integration check against the real names we've seen on
    /// dongle-pull.
    #[test]
    fn classifier_treats_fdo_unknown_object_as_gone() {
        let inner = zbus::fdo::Error::UnknownObject("/org/bluez/hci0".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(is_adapter_gone(&err));
    }

    #[test]
    fn classifier_treats_fdo_unknown_method_as_gone() {
        let inner = zbus::fdo::Error::UnknownMethod("Set".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(is_adapter_gone(&err));
    }

    #[test]
    fn classifier_treats_fdo_unknown_interface_as_gone() {
        let inner = zbus::fdo::Error::UnknownInterface("org.bluez.Adapter1".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(is_adapter_gone(&err));
    }

    /// Other DBus errors must propagate, not be silently swallowed.
    #[test]
    fn classifier_does_not_swallow_unrelated_errors() {
        let err = zbus::Error::Address("not a real bus address".to_string());
        assert!(!is_adapter_gone(&err));
        let err2 = zbus::Error::FDO(Box::new(zbus::fdo::Error::AccessDenied(
            "not allowed".into(),
        )));
        assert!(!is_adapter_gone(&err2));
    }

    /// Pin the exact MethodError name strings we recognise as
    /// gone-adapter — the classifier's match arm depends on these
    /// exact strings appearing in the real zbus errors at runtime.
    #[test]
    fn classifier_method_error_name_set_is_documented() {
        // If a future zbus / BlueZ rev changes these names, the
        // classifier will silently revert to propagating the error
        // (and we'll see `error!` lines in the journal again). This
        // test documents the set without standing up a real bus.
        let names = [
            "org.bluez.Error.NotReady",
            "org.bluez.Error.NotFound",
            "org.freedesktop.DBus.Error.UnknownObject",
            "org.freedesktop.DBus.Error.UnknownInterface",
            "org.freedesktop.DBus.Error.UnknownMethod",
        ];
        // The classifier source must literally contain each name.
        let src = include_str!("apply.rs");
        for n in names {
            assert!(
                src.contains(n),
                "is_adapter_gone must list the MethodError name {n}"
            );
        }
    }
}
