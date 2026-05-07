// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::nm` — wraps the existing `crate::nm::*` zbus calls behind
//! the [`NetworkBackend`] trait. No behaviour change vs the pre-trait
//! call sites; the migration of `src/commands/*.rs` away from direct
//! `crate::nm::*` imports is the rest of Milestone 1.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use super::{BackendDevice, BackendKind, BoxFuture, NetworkBackend, RotateOutcome};
use crate::mac::{Mac, factory};
use crate::nm::{self, DeviceKind};

/// NetworkManager-backed implementation. Holds nothing — the system
/// DBus connection is opened lazily per call so the backend value is
/// cheap to construct and cheap to clone for trait-object dispatch.
#[derive(Debug, Default)]
pub struct NmBackend;

impl NmBackend {
    pub fn new() -> Self {
        Self
    }

    async fn connect() -> Result<zbus::Connection> {
        zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager)")
    }
}

impl NetworkBackend for NmBackend {
    fn name(&self) -> &'static str {
        "nm"
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, bool> {
        // /run/NetworkManager and /var/run/NetworkManager are the same
        // signal `commands::status::detect_system` uses; cheap and
        // mirrors what the user already sees in `proteus status`.
        Box::pin(async {
            Path::new("/run/NetworkManager").exists()
                || Path::new("/var/run/NetworkManager").exists()
        })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async move {
            let conn = Self::connect().await?;
            let devs = nm::list_devices(&conn).await?;
            let out = devs
                .into_iter()
                .map(|d| BackendDevice {
                    iface: d.interface,
                    kind: kind_from_nm(d.kind),
                    hw_address: d.hw_address,
                    // We key NM mutations off the connection object
                    // path, not the device path: `set_cloned_mac` and
                    // friends call `Settings.Connection.Update`. When
                    // the device has no profile yet we leave the
                    // identifier empty so the trait caller surfaces a
                    // clear "no NM connection profile available"
                    // error rather than silently mutating a wrong
                    // path. (Mirrors `commands::rotate::rotate_one`.)
                    identifier: d
                        .connections
                        .first()
                        .map(|p| p.as_str().to_string())
                        .unwrap_or_default(),
                })
                .collect();
            Ok(out)
        })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
        mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_identifier(&device.identifier)?;
            let conn = Self::connect().await?;
            let kind = kind_to_nm(device.kind).ok_or_else(|| {
                anyhow!(
                    "backend::nm: device kind {:?} has no cloned-MAC setting",
                    device.kind
                )
            })?;
            nm::apply::set_cloned_mac(&conn, &path, kind, mac).await
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            if device.identifier.is_empty() {
                return Ok(None);
            }
            let path = parse_identifier(&device.identifier)?;
            let conn = Self::connect().await?;
            let kind = match kind_to_nm(device.kind) {
                Some(k) => k,
                None => return Ok(None),
            };
            nm::apply::read_cloned_mac(&conn, &path, kind).await
        })
    }

    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        // Same source `commands::rotate` and `commands::status` already
        // use; the trait method just routes through it so non-NM
        // backends can share the read path verbatim.
        Box::pin(async move { Ok(factory::permanent_address(iface)) })
    }

    fn rotate_if_needed<'a>(
        &'a self,
        _iface: &'a str,
        _cooldown: Duration,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        // Issue #206-C tracks the dispatcher migration onto this
        // typed entry point. The full implementation lifts the cooldown
        // arithmetic out of `commands::rotate::run`; for the trait-
        // scaffolding PR we surface `BackendUnavailable` so callers
        // know the structured path is not yet wired.
        Box::pin(async move { Ok(RotateOutcome::BackendUnavailable) })
    }
}

fn kind_from_nm(k: DeviceKind) -> BackendKind {
    match k {
        DeviceKind::Wifi => BackendKind::Wifi,
        DeviceKind::Ethernet => BackendKind::Ethernet,
        DeviceKind::Other(_) => BackendKind::Other,
    }
}

fn kind_to_nm(k: BackendKind) -> Option<DeviceKind> {
    match k {
        BackendKind::Wifi => Some(DeviceKind::Wifi),
        BackendKind::Ethernet => Some(DeviceKind::Ethernet),
        BackendKind::Other => None,
    }
}

fn parse_identifier(s: &str) -> Result<OwnedObjectPath> {
    if s.is_empty() {
        return Err(anyhow!(
            "backend::nm: device has no NM connection profile (identifier empty)"
        ));
    }
    let p = ObjectPath::try_from(s)
        .with_context(|| format!("parsing NM connection path '{s}'"))?;
    Ok(p.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trip_wifi_and_ethernet() {
        assert_eq!(kind_from_nm(DeviceKind::Wifi), BackendKind::Wifi);
        assert_eq!(kind_from_nm(DeviceKind::Ethernet), BackendKind::Ethernet);
        assert_eq!(kind_from_nm(DeviceKind::Other(7)), BackendKind::Other);

        assert_eq!(kind_to_nm(BackendKind::Wifi), Some(DeviceKind::Wifi));
        assert_eq!(
            kind_to_nm(BackendKind::Ethernet),
            Some(DeviceKind::Ethernet)
        );
        assert_eq!(kind_to_nm(BackendKind::Other), None);
    }

    #[test]
    fn parse_identifier_rejects_empty() {
        assert!(parse_identifier("").is_err());
    }

    #[test]
    fn parse_identifier_accepts_well_formed_path() {
        let p =
            parse_identifier("/org/freedesktop/NetworkManager/Settings/3").expect("valid dbus path");
        assert_eq!(
            p.as_str(),
            "/org/freedesktop/NetworkManager/Settings/3"
        );
    }

    #[test]
    fn name_is_stable_token() {
        assert_eq!(NmBackend::new().name(), "nm");
    }
}
