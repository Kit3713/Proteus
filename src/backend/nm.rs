// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::nm` — wraps the existing `crate::nm::*` zbus calls behind
//! the [`NetworkBackend`] trait. No behaviour change vs the pre-trait
//! call sites; the rest of Milestone 1 routes
//! `src/commands/{rotate,dhcp,ipv6,enterprise_wifi}.rs` through this
//! impl so non-NM backends can drop in.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use super::{
    BackendDevice, BackendKind, BoxFuture, ConnectionRef, NetworkBackend, RenewOutcome,
    RotateOutcome,
};
use crate::ipv6::nm::Ipv6NmSettings;
use crate::mac::{Mac, factory};
use crate::nm::{self, ConnectionSettings, DeviceKind};
use crate::state::DhcpSettingsSnapshot;

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
                    // `identifier` is the NM Device object path — used
                    // by `renew_lease`. The connection-keyed mutators
                    // (set_cloned_mac, set_dhcp_settings, ...) iterate
                    // `connections` instead.
                    identifier: d.path.as_str().to_string(),
                    connections: d
                        .connections
                        .iter()
                        .map(|p| ConnectionRef::new(p.as_str()))
                        .collect(),
                    managed: d.managed,
                })
                .collect();
            Ok(out)
        })
    }

    fn list_connections<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Vec<ConnectionRef>>> {
        // The NM impl has the connections cached on the device value
        // already (populated in `list_devices`); just hand them back.
        let out = device.connections.clone();
        Box::pin(async move { Ok(out) })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
        mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let kind = kind_to_nm(device.kind).ok_or_else(|| {
                anyhow!(
                    "backend::nm: device kind {:?} has no cloned-MAC setting",
                    device.kind
                )
            })?;
            if device.connections.is_empty() {
                return Err(anyhow!(
                    "backend::nm: device {} has no NM connection profile",
                    device.iface
                ));
            }
            let conn = Self::connect().await?;
            // Issue #122: write to every profile, not just the first.
            for cref in &device.connections {
                let path = parse_connection_ref(cref)?;
                nm::apply::set_cloned_mac(&conn, &path, kind, mac).await?;
            }
            Ok(())
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let Some(cref) = device.connections.first() else {
                return Ok(None);
            };
            let kind = match kind_to_nm(device.kind) {
                Some(k) => k,
                None => return Ok(None),
            };
            let conn = Self::connect().await?;
            let path = parse_connection_ref(cref)?;
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
        iface: &'a str,
        cooldown: Duration,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        // Issue #206-C: structured entry point used by the NM
        // dispatcher in place of the previous `proteus current --json | sed`
        // grep. The dispatcher path stays read-mostly for backwards
        // compatibility — the cooldown decision lives here, the actual
        // mutation is delegated to `commands::rotate::run` (no DBus
        // spelling out).
        Box::pin(async move {
            let state_path = std::path::PathBuf::from(crate::commands::DEFAULT_STATE_PATH);
            let state = match crate::state::State::load_or_default(&state_path) {
                Ok(s) => s,
                Err(_) => return Ok(RotateOutcome::BackendUnavailable),
            };
            // Cooldown check: read the per-iface `last_rotated` and bail
            // structured if the elapsed time hasn't met the budget yet.
            if let Some(rec) = state.managed.interfaces.get(iface)
                && let Some(stamp) = rec.last_rotated.as_deref()
                && let Some(remaining) = remaining_cooldown(stamp, cooldown)
            {
                return Ok(RotateOutcome::SkippedCooldown { remaining });
            }
            // Factory MAC must be on file before we ever rotate; the
            // sacred-originals invariant in `commands::rotate` saves it
            // mid-run, but `rotate-if-needed` is meant to be called by
            // the dispatcher BEFORE the first rotation, so we check
            // here too. Returning `NoFactoryMac` lets the dispatcher
            // log a clear "this driver doesn't expose a factory MAC,
            // skipping" rather than a generic NM error.
            if factory::permanent_address(iface).is_none()
                && !state.original_macs.contains_key(iface)
            {
                return Ok(RotateOutcome::NoFactoryMac);
            }
            // Delegate to the existing rotate path. We don't pass the
            // result through Result<u8> because that's the CLI exit
            // code; the typed outcome here is "rotated" iff the call
            // succeeded.
            let res = crate::commands::rotate::run(
                Some(iface),
                true,
                false,
                Some(state_path.as_path()),
                None,
            );
            match res {
                Ok(c) if c == crate::exit::SUCCESS => {
                    // Read back the new MAC the rotation just wrote.
                    let new_state = crate::state::State::load_or_default(&state_path)
                        .unwrap_or_default();
                    let new_mac = new_state
                        .managed
                        .interfaces
                        .get(iface)
                        .and_then(|r| r.current_mac.as_deref())
                        .and_then(|s| s.parse::<Mac>().ok())
                        .unwrap_or(Mac::new([0; 6]));
                    Ok(RotateOutcome::Rotated { new_mac })
                }
                Ok(_) | Err(_) => Ok(RotateOutcome::BackendUnavailable),
            }
        })
    }

    fn read_connection_id<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = Self::connect().await?;
            nm::apply::read_connection_id(&conn, &path).await
        })
    }

    fn read_connection_uuid<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = Self::connect().await?;
            nm::apply::read_connection_uuid(&conn, &path).await
        })
    }

    fn set_dhcp_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        snapshot: DhcpSettingsSnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = Self::connect().await?;
            // Read current settings, write the snapshot's keys, push
            // back via the secrets-aware updater. `revert_dhcp_settings`
            // is the same routine the per-command revert path uses;
            // here it does the apply direction by writing the desired
            // snapshot directly onto the live settings.
            let mut settings: ConnectionSettings = nm::dhcp::get_settings(&conn, &path).await?;
            nm::dhcp::revert_dhcp_settings(&mut settings, &snapshot)?;
            nm::dhcp::update_connection(&conn, &path, settings).await
        })
    }

    fn set_ipv6_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        settings: Ipv6NmSettings,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = Self::connect().await?;
            crate::ipv6::nm::apply_settings(&conn, &path, &settings).await
        })
    }

    fn renew_lease<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<RenewOutcome>> {
        Box::pin(async move {
            if device.identifier.is_empty() {
                return Err(anyhow!(
                    "backend::nm: device {} has no NM device path",
                    device.iface
                ));
            }
            let conn = Self::connect().await?;
            let dev_path = ObjectPath::try_from(device.identifier.as_str())
                .with_context(|| format!("parsing NM device path '{}'", device.identifier))?;
            let owned: OwnedObjectPath = dev_path.into();
            let outcome = nm::dhcp::renew_lease(&conn, &owned).await?;
            Ok(match outcome {
                nm::dhcp::RenewOutcome::Reapplied => RenewOutcome::Reapplied,
                nm::dhcp::RenewOutcome::DisconnectActivated => RenewOutcome::DisconnectActivated,
                nm::dhcp::RenewOutcome::NoActiveConnection => RenewOutcome::NoActiveConnection,
            })
        })
    }

    fn write_anonymous_identity<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        value: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = Self::connect().await?;
            crate::enterprise_wifi::nm::write_anonymous_identity(&conn, &path, value).await
        })
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

fn parse_connection_ref(cref: &ConnectionRef) -> Result<OwnedObjectPath> {
    let s = cref.as_str();
    if s.is_empty() {
        return Err(anyhow!(
            "backend::nm: connection has no NM dbus path (empty identifier)"
        ));
    }
    let p = ObjectPath::try_from(s)
        .with_context(|| format!("parsing NM connection path '{s}'"))?;
    Ok(p.into())
}

/// Compute remaining cooldown given an ISO-8601 `last_rotated` stamp
/// and a budget. Returns `None` if the cooldown has expired (or the
/// stamp couldn't be parsed) — both cases mean "go ahead and rotate".
fn remaining_cooldown(stamp: &str, cooldown: Duration) -> Option<Duration> {
    let last = parse_iso8601_z(stamp)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let elapsed = now.saturating_sub(last);
    if elapsed >= cooldown.as_secs() {
        None
    } else {
        Some(Duration::from_secs(cooldown.as_secs() - elapsed))
    }
}

/// Hand-rolled inverse of `commands::now_iso8601` — accepts the
/// exact `YYYY-MM-DDTHH:MM:SSZ` shape that helper writes. Returns the
/// Unix epoch second on success, `None` on any parse failure (callers
/// treat that as "no cooldown known").
fn parse_iso8601_z(stamp: &str) -> Option<u64> {
    if stamp.len() != 20 || !stamp.ends_with('Z') {
        return None;
    }
    let y: u32 = stamp[0..4].parse().ok()?;
    let mo: u32 = stamp[5..7].parse().ok()?;
    let d: u32 = stamp[8..10].parse().ok()?;
    let h: u32 = stamp[11..13].parse().ok()?;
    let mi: u32 = stamp[14..16].parse().ok()?;
    let s: u32 = stamp[17..19].parse().ok()?;
    Some(ymdhms_to_unix(y, mo, d, h, mi, s))
}

/// Civil-from-days inverse, mirroring `commands::unix_to_ymdhms`.
fn ymdhms_to_unix(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    let y = y as i64 - if mo <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let secs = days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64;
    secs.max(0) as u64
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
    fn parse_connection_ref_rejects_empty() {
        assert!(parse_connection_ref(&ConnectionRef::new("")).is_err());
    }

    #[test]
    fn parse_connection_ref_accepts_well_formed_path() {
        let p = parse_connection_ref(&ConnectionRef::new(
            "/org/freedesktop/NetworkManager/Settings/3",
        ))
        .expect("valid dbus path");
        assert_eq!(
            p.as_str(),
            "/org/freedesktop/NetworkManager/Settings/3"
        );
    }

    #[test]
    fn name_is_stable_token() {
        assert_eq!(NmBackend::new().name(), "nm");
    }

    #[test]
    fn parse_iso8601_z_rejects_garbage() {
        assert!(parse_iso8601_z("").is_none());
        assert!(parse_iso8601_z("nope").is_none());
        // Wrong length / missing Z.
        assert!(parse_iso8601_z("2026-05-07T12:00:00").is_none());
        assert!(parse_iso8601_z("2026-05-07T12:00:00X").is_none());
    }

    #[test]
    fn parse_iso8601_z_round_trips_unix_to_ymdhms() {
        // Use the existing forward helper to build a stamp at a known
        // epoch, then round trip back through the inverse.
        let secs: u64 = 1_710_000_000; // 2024-03-09 16:00 UTC
        let (y, mo, d, h, mi, s) = crate::commands::unix_to_ymdhms(secs);
        let stamp = format!(
            "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"
        );
        let parsed = parse_iso8601_z(&stamp).unwrap();
        assert_eq!(parsed, secs);
    }

    #[test]
    fn remaining_cooldown_none_when_stamp_in_past() {
        // A stamp from year 2000 is far past any reasonable cooldown.
        let r = remaining_cooldown("2000-01-01T00:00:00Z", Duration::from_secs(60));
        assert!(r.is_none());
    }
}
