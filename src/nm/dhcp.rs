// SPDX-License-Identifier: GPL-3.0-or-later

//! DHCP-suppression-related NetworkManager DBus helpers.
//!
//! NM stores DHCP knobs under the `ipv4` and `ipv6` settings sections. The
//! keys we touch are listed in `wiki/dhcp.md`. Per-connection writes go via
//! `Settings.Connection.Update`; we always read the full setting dict first
//! and merge our changes in so unrelated keys are left untouched.

use std::collections::HashMap;

use anyhow::{Context, Result};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::{ConnectionProxy, ConnectionSettings, DeviceProxy, NetworkManagerProxy};
use crate::state::DhcpSettingsSnapshot;

pub const SECTION_IPV4: &str = "ipv4";
pub const SECTION_IPV6: &str = "ipv6";
pub const KEY_DHCP_SEND_HOSTNAME: &str = "dhcp-send-hostname";
pub const KEY_DHCP_HOSTNAME: &str = "dhcp-hostname";
pub const KEY_DHCP_FQDN: &str = "dhcp-fqdn";
pub const KEY_DHCP_VENDOR_CLASS_IDENTIFIER: &str = "dhcp-vendor-class-identifier";
pub const KEY_DHCP_CLIENT_ID: &str = "dhcp-client-id";
pub const KEY_DHCP_DUID: &str = "dhcp-duid";
pub const KEY_DHCP_IAID: &str = "dhcp-iaid";

pub const SECTION_CONNECTION: &str = "connection";
pub const KEY_USER_DATA: &str = "user-data";

/// Snapshot the DHCP-related keys we care about from a full settings dict.
pub fn snapshot_dhcp(settings: &ConnectionSettings) -> DhcpSettingsSnapshot {
    DhcpSettingsSnapshot {
        ipv4_dhcp_send_hostname: extract_bool(settings, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME),
        ipv4_dhcp_hostname: extract_str(settings, SECTION_IPV4, KEY_DHCP_HOSTNAME),
        ipv4_dhcp_fqdn: extract_str(settings, SECTION_IPV4, KEY_DHCP_FQDN),
        ipv4_dhcp_vendor_class_identifier: extract_str(
            settings,
            SECTION_IPV4,
            KEY_DHCP_VENDOR_CLASS_IDENTIFIER,
        ),
        ipv4_dhcp_client_id: extract_str(settings, SECTION_IPV4, KEY_DHCP_CLIENT_ID),
        ipv6_dhcp_duid: extract_str(settings, SECTION_IPV6, KEY_DHCP_DUID),
        ipv6_dhcp_iaid: extract_str(settings, SECTION_IPV6, KEY_DHCP_IAID),
    }
}

/// Roadmap M2 "Integration": shape DHCP options from a persona's
/// `dhcp_fingerprint`. Persona values fill slots that are unset by the
/// user's `[dhcp]` knobs — per-knob overrides always win, persona only
/// claims what would otherwise be suppressed or left at default.
///
/// Concrete precedence per slot:
///
/// - `vendor-class-identifier` (option 60): when `suppress_vendor_class`
///   is `true` the suppression wins (the slot is empty on the wire).
///   When the user left `suppress_vendor_class = false`, persona's
///   `vendor_class_identifier` (if non-empty) is written.
/// - `hostname` / `fqdn` (options 12 / 81): `suppress_hostname = true`
///   wins. Otherwise persona's `host_name` / `fqdn` are honoured when
///   non-empty.
///
/// The parameter-request-list (option 55) and DUID/IAID slots are
/// downstream of NM's own option handling — NM's `ipv4.dhcp-iaid` /
/// `ipv4.dhcp-client-id` only accept a small grammar so the persona's
/// raw byte list is logged at `tracing::debug!` for now and the
/// `rotate_client_id` knob's `mac` / `ll` values stay authoritative.
pub fn apply_persona_fingerprint(
    settings: &mut ConnectionSettings,
    persona: &crate::persona::Persona,
    suppress_hostname: bool,
    suppress_vendor_class: bool,
) -> Result<bool> {
    let fp = &persona.dhcp_fingerprint;
    let mut changed = false;
    if !suppress_vendor_class && !fp.vendor_class_identifier.is_empty() {
        changed |= set_str(
            settings,
            SECTION_IPV4,
            KEY_DHCP_VENDOR_CLASS_IDENTIFIER,
            &fp.vendor_class_identifier,
        )?;
    }
    if !suppress_hostname {
        if !fp.host_name.is_empty() {
            // Tell NM "send hostname" then write the persona's host_name
            // — without flipping send_hostname back on, the previous
            // suppress_hostname=true sticky setting would still force
            // option 12 off. The user's per-connection knobs override
            // this if they later explicitly toggle suppress_hostname.
            changed |= set_bool(settings, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME, true)?;
            changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_HOSTNAME, &fp.host_name)?;
        }
        if !fp.fqdn.is_empty() {
            changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_FQDN, &fp.fqdn)?;
        }
    }
    if !fp.parameter_request_list.is_empty() {
        // NM doesn't expose option 55 directly; surface the persona's
        // intent in the log so an operator running with `-v` can see it
        // (a future backend-trait extension can pipe this through to
        // dhclient.conf / networkd's `[DHCPv4] RequestOptions=`).
        tracing::debug!(
            persona = %persona.id,
            options = ?fp.parameter_request_list,
            "persona parameter-request-list captured but NM has no direct ipv4.dhcp-* slot \
             (followup: pipe through backend trait)"
        );
    }
    // Issue #305: mDNS records aren't yet wired into avahi but log them
    // so an operator running `-v` sees the cross-layer surface the
    // persona intends. Avahi drop-in emission is the integration
    // follow-up tracked alongside this issue.
    if !persona.mdns.services.is_empty() {
        tracing::debug!(
            persona = %persona.id,
            services = ?persona.mdns.services,
            advertise = persona.mdns_advertise,
            "persona mdns service set captured (avahi/Bonjour wiring is the integration follow-up)"
        );
    }
    Ok(changed)
}

/// Mutate `settings` in place to apply Proteus DHCP suppression, honoring
/// which knobs are enabled in config. Returns true if anything changed.
pub fn apply_dhcp_settings(
    settings: &mut ConnectionSettings,
    suppress_hostname: bool,
    suppress_vendor_class: bool,
    rotate_client_id: bool,
) -> Result<bool> {
    let mut changed = false;
    if suppress_hostname {
        // ipv4.dhcp-send-hostname=no → no option 12.
        changed |= set_bool(settings, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME, false)?;
        // Belt-and-braces: clear ipv4.dhcp-hostname so an old explicit value
        // doesn't ride along if NM ever gains a "send anyway" toggle.
        changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_HOSTNAME, "")?;
        // ipv4.dhcp-fqdn cleared → no option 81.
        changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_FQDN, "")?;
    }
    if suppress_vendor_class {
        changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_VENDOR_CLASS_IDENTIFIER, "")?;
    }
    if rotate_client_id {
        changed |= set_str(settings, SECTION_IPV4, KEY_DHCP_CLIENT_ID, "mac")?;
        changed |= set_str(settings, SECTION_IPV6, KEY_DHCP_DUID, "ll")?;
        changed |= set_str(settings, SECTION_IPV6, KEY_DHCP_IAID, "mac")?;
    }
    Ok(changed)
}

/// Restore the cached pre-Proteus values onto `settings` in place. Keys whose
/// snapshot value is `None` are removed entirely so NM falls back to its
/// own defaults.
pub fn revert_dhcp_settings(
    settings: &mut ConnectionSettings,
    snap: &DhcpSettingsSnapshot,
) -> Result<()> {
    restore_bool(
        settings,
        SECTION_IPV4,
        KEY_DHCP_SEND_HOSTNAME,
        snap.ipv4_dhcp_send_hostname,
    )?;
    restore_str(
        settings,
        SECTION_IPV4,
        KEY_DHCP_HOSTNAME,
        snap.ipv4_dhcp_hostname.as_deref(),
    )?;
    restore_str(
        settings,
        SECTION_IPV4,
        KEY_DHCP_FQDN,
        snap.ipv4_dhcp_fqdn.as_deref(),
    )?;
    restore_str(
        settings,
        SECTION_IPV4,
        KEY_DHCP_VENDOR_CLASS_IDENTIFIER,
        snap.ipv4_dhcp_vendor_class_identifier.as_deref(),
    )?;
    restore_str(
        settings,
        SECTION_IPV4,
        KEY_DHCP_CLIENT_ID,
        snap.ipv4_dhcp_client_id.as_deref(),
    )?;
    restore_str(
        settings,
        SECTION_IPV6,
        KEY_DHCP_DUID,
        snap.ipv6_dhcp_duid.as_deref(),
    )?;
    restore_str(
        settings,
        SECTION_IPV6,
        KEY_DHCP_IAID,
        snap.ipv6_dhcp_iaid.as_deref(),
    )?;
    Ok(())
}

/// Tag a connection with `proteus.managed=true` (and friends) under
/// `connection.user-data`, preserving any third-party entries already there.
pub fn tag_user_data(
    settings: &mut ConnectionSettings,
    applied_version: &str,
    applied_at: &str,
) -> Result<()> {
    let section = settings.entry(SECTION_CONNECTION.to_string()).or_default();
    let mut existing = read_user_data(section);
    existing.insert("proteus.managed".to_string(), "true".to_string());
    existing.insert(
        "proteus.applied-version".to_string(),
        applied_version.to_string(),
    );
    existing.insert("proteus.applied-at".to_string(), applied_at.to_string());
    write_user_data(section, &existing)
}

/// Remove our `proteus.*` entries from `connection.user-data`. Leaves any
/// keys we don't own untouched. Drops the key entirely once empty so NM's
/// settings stay tidy.
pub fn untag_user_data(settings: &mut ConnectionSettings) -> Result<()> {
    let Some(section) = settings.get_mut(SECTION_CONNECTION) else {
        return Ok(());
    };
    let mut existing = read_user_data(section);
    existing.retain(|k, _| !k.starts_with("proteus."));
    if existing.is_empty() {
        section.remove(KEY_USER_DATA);
    } else {
        write_user_data(section, &existing)?;
    }
    Ok(())
}

/// Read the `proteus.managed=true` tag from a settings dict.
pub fn is_proteus_managed(settings: &ConnectionSettings) -> bool {
    let Some(section) = settings.get(SECTION_CONNECTION) else {
        return false;
    };
    let map = read_user_data(section);
    map.get("proteus.managed").map(String::as_str) == Some("true")
}

/// Push a settings dict back to NM via `Settings.Connection.Update`.
///
/// Issue #207: routed through the shared `nm::update_with_secrets` helper
/// so DHCP/IPv6 mutations on a Wi-Fi or 802.1X profile preserve the
/// stored PSK / EAP password. The trailing `_proxy` arg is retained for
/// signature stability with older callers.
pub async fn update_connection(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
    settings: ConnectionSettings,
) -> Result<()> {
    super::update_with_secrets(conn, path, settings).await
}

/// Read the current settings dict for a connection.
pub async fn get_settings(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Result<ConnectionSettings> {
    let proxy = ConnectionProxy::builder(conn)
        .path(path.clone())?
        .build()
        .await?;
    proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")
}

/// Pull the `connection.id` field out of a settings dict.
pub fn connection_id(settings: &ConnectionSettings) -> Option<String> {
    extract_str(settings, SECTION_CONNECTION, "id")
}

/// Pull the `connection.uuid` field out of a settings dict. Issue #124:
/// state must be keyed by `uuid` (NM's only uniqueness guarantee), not by
/// `id` (a display string that can collide between profiles).
pub fn connection_uuid(settings: &ConnectionSettings) -> Option<String> {
    extract_str(settings, SECTION_CONNECTION, "uuid")
}

/// Pull the `connection.type` field, mapping NM type strings to friendly
/// labels for display.
pub fn connection_kind(settings: &ConnectionSettings) -> String {
    match extract_str(settings, SECTION_CONNECTION, "type").as_deref() {
        Some("802-11-wireless") => "wifi".into(),
        Some("802-3-ethernet") => "ethernet".into(),
        Some(other) => other.to_string(),
        None => "other".into(),
    }
}

fn extract_str(settings: &ConnectionSettings, section: &str, key: &str) -> Option<String> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &Value = val;
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

fn extract_bool(settings: &ConnectionSettings, section: &str, key: &str) -> Option<bool> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &Value = val;
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn set_bool(
    settings: &mut ConnectionSettings,
    section: &str,
    key: &str,
    value: bool,
) -> Result<bool> {
    let sec = settings.entry(section.to_string()).or_default();
    let cur = sec.get(key).and_then(|v| {
        let val: &Value = v;
        if let Value::Bool(b) = val {
            Some(*b)
        } else {
            None
        }
    });
    if cur == Some(value) {
        return Ok(false);
    }
    sec.insert(key.to_string(), Value::from(value).try_into()?);
    Ok(true)
}

fn set_str(
    settings: &mut ConnectionSettings,
    section: &str,
    key: &str,
    value: &str,
) -> Result<bool> {
    let sec = settings.entry(section.to_string()).or_default();
    let cur = sec.get(key).and_then(|v| {
        let val: &Value = v;
        if let Value::Str(s) = val {
            Some(s.as_str().to_string())
        } else {
            None
        }
    });
    if cur.as_deref() == Some(value) {
        return Ok(false);
    }
    sec.insert(key.to_string(), Value::from(value.to_string()).try_into()?);
    Ok(true)
}

fn restore_bool(
    settings: &mut ConnectionSettings,
    section: &str,
    key: &str,
    value: Option<bool>,
) -> Result<()> {
    match value {
        Some(b) => {
            let sec = settings.entry(section.to_string()).or_default();
            sec.insert(key.to_string(), Value::from(b).try_into()?);
        }
        None => {
            // No cached value to restore. Only touch the section if it
            // already exists — issue #151: previously we would `or_default`
            // the entry and materialize an empty `[ipv6]` section even when
            // the originals dict had nothing IPv6-shaped at all.
            if let Some(sec) = settings.get_mut(section) {
                sec.remove(key);
                if sec.is_empty() {
                    settings.remove(section);
                }
            }
        }
    }
    Ok(())
}

fn restore_str(
    settings: &mut ConnectionSettings,
    section: &str,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    match value {
        Some(s) => {
            let sec = settings.entry(section.to_string()).or_default();
            sec.insert(key.to_string(), Value::from(s.to_string()).try_into()?);
        }
        None => {
            if let Some(sec) = settings.get_mut(section) {
                sec.remove(key);
                if sec.is_empty() {
                    settings.remove(section);
                }
            }
        }
    }
    Ok(())
}

// connection.user-data is a Dict<String, String> in NM's API. zvariant gives
// us back an OwnedValue holding a Dict; we copy entries into a HashMap, mutate
// it, then build a new Dict on the way out.
fn read_user_data(section: &HashMap<String, OwnedValue>) -> HashMap<String, String> {
    let Some(val) = section.get(KEY_USER_DATA) else {
        return HashMap::new();
    };
    let v: &Value = val;
    let mut out = HashMap::new();
    if let Value::Dict(d) = v {
        for (k, v) in d.iter() {
            if let (Value::Str(ks), Value::Str(vs)) = (k, v) {
                out.insert(ks.as_str().to_string(), vs.as_str().to_string());
            }
        }
    }
    out
}

fn write_user_data(
    section: &mut HashMap<String, OwnedValue>,
    map: &HashMap<String, String>,
) -> Result<()> {
    let mut dict = zbus::zvariant::Dict::new(
        &zbus::zvariant::Signature::Str,
        &zbus::zvariant::Signature::Str,
    );
    for (k, v) in map {
        dict.add(k.clone(), v.clone())
            .context("building user-data dict entry")?;
    }
    section.insert(
        KEY_USER_DATA.to_string(),
        Value::Dict(dict)
            .try_to_owned()
            .context("converting user-data dict")?,
    );
    Ok(())
}

/// Outcome of a single per-device DHCP lease renew. The IP/lease number
/// is reported as a placeholder string since NM doesn't surface a
/// monotonic lease counter — the visible signal is "the device picked
/// up a fresh lease without losing the L2 association". Roadmap
/// Milestone 4c.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewOutcome {
    /// `Device.Reapply` succeeded — NM kept the link up and re-ran
    /// DHCP against the existing settings. This is the cheap path.
    Reapplied,
    /// `Device.Reapply` was rejected (typically `NotSupported` on NM
    /// ≤1.0, or `InvalidArguments` if the connection has unsynced
    /// changes) and we fell back to `Disconnect` +
    /// `NetworkManager.ActivateConnection`.
    DisconnectActivated,
    /// No active connection on the device — the renew is a no-op,
    /// not an error.
    NoActiveConnection,
}

/// Release + renew the DHCP lease on `device_path` without touching the
/// cloned MAC. Tries the cheap NM `Device.Reapply` path first
/// (link stays up, DHCP client just re-DISCOVERs), and falls back to
/// `Disconnect` + `ActivateConnection` for older NM that doesn't
/// support `Reapply` or for connections where `Reapply` is rejected.
///
/// The fallback is more disruptive: the link drops momentarily, the
/// supplicant re-associates, and 802.1X (if any) re-authenticates.
/// That's still cheaper than a full rotation because the cloned MAC
/// is left alone — no `Settings.Connection.Update` round trip, no
/// state.json mutation. From the upstream DHCP server's perspective
/// it's a fresh client showing the same MAC, which yields a fresh
/// lease (or, on a sticky pool, the same one).
pub async fn renew_lease(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
) -> Result<RenewOutcome> {
    let dev = DeviceProxy::builder(conn)
        .path(device_path.clone())?
        .build()
        .await?;

    // The "/" object path is NM's signal for "no active connection."
    // Don't try to renew an idle device — surface that to the caller
    // so the report can render "no active connection" rather than a
    // confusing DBus error.
    let active_path: OwnedObjectPath = match dev.active_connection().await {
        Ok(p) if p.as_str() == "/" => return Ok(RenewOutcome::NoActiveConnection),
        Ok(p) => p,
        Err(_) => return Ok(RenewOutcome::NoActiveConnection),
    };

    // Empty settings dict + version=0 + flags=0 tells NM to use the
    // currently-stored connection settings as-is. NM's documented
    // contract for that combination is "kick DHCP without changing
    // anything" — no settings round trip, no L2 disturb.
    let empty: ConnectionSettings = HashMap::new();
    match dev.reapply(empty, 0, 0).await {
        Ok(()) => Ok(RenewOutcome::Reapplied),
        Err(e) => {
            tracing::debug!(
                "Device.Reapply rejected ({e:#}); falling back to Disconnect+ActivateConnection"
            );
            renew_via_disconnect_activate(conn, device_path, &active_path).await
        }
    }
}

/// Fallback path: `Device.Disconnect` brings the link down, then
/// `NetworkManager.ActivateConnection(Connection, Device, "/")` brings
/// it back up against the same connection profile. We pull the
/// connection-path-of-the-active-connection by reading the
/// ActiveConnection object's `Connection` property.
async fn renew_via_disconnect_activate(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
) -> Result<RenewOutcome> {
    // Resolve the `Connection` (Settings.Connection object path) that
    // backs this ActiveConnection. We do a generic Properties.Get so
    // we don't have to declare a whole NM.Connection.Active proxy
    // just for this one read.
    let connection_path = read_active_connection_path(conn, active_path)
        .await
        .context("reading ActiveConnection.Connection")?;

    let dev = DeviceProxy::builder(conn)
        .path(device_path.clone())?
        .build()
        .await?;
    dev.disconnect()
        .await
        .context("calling Device.Disconnect")?;

    let nm = NetworkManagerProxy::new(conn).await?;
    let conn_obj: ObjectPath<'_> = connection_path.as_ref();
    let dev_obj: ObjectPath<'_> = device_path.as_ref();
    let root = ObjectPath::try_from("/").context("constructing root object path")?;
    nm.activate_connection(&conn_obj, &dev_obj, &root)
        .await
        .context("calling NetworkManager.ActivateConnection")?;
    Ok(RenewOutcome::DisconnectActivated)
}

/// Read the `Connection` property off an ActiveConnection object via
/// the generic `org.freedesktop.DBus.Properties` interface. Avoids
/// declaring a one-shot NM.Connection.Active proxy.
async fn read_active_connection_path(
    conn: &zbus::Connection,
    active_path: &OwnedObjectPath,
) -> Result<OwnedObjectPath> {
    use zbus::Proxy;
    let proxy = Proxy::new(
        conn,
        "org.freedesktop.NetworkManager",
        active_path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .await?;
    let val: OwnedObjectPath = proxy.get_property("Connection").await?;
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_settings() -> ConnectionSettings {
        HashMap::new()
    }

    #[test]
    fn apply_full_changes_all_keys() {
        let mut s = empty_settings();
        let changed = apply_dhcp_settings(&mut s, true, true, true).unwrap();
        assert!(changed);
        let ipv4 = s.get(SECTION_IPV4).expect("ipv4 section created");
        let send: &Value = ipv4.get(KEY_DHCP_SEND_HOSTNAME).unwrap();
        assert!(matches!(send, Value::Bool(false)));
        for key in [
            KEY_DHCP_FQDN,
            KEY_DHCP_VENDOR_CLASS_IDENTIFIER,
            KEY_DHCP_HOSTNAME,
        ] {
            let v: &Value = ipv4.get(key).unwrap();
            assert!(matches!(v, Value::Str(s) if s.as_str().is_empty()));
        }
        let cid: &Value = ipv4.get(KEY_DHCP_CLIENT_ID).unwrap();
        assert!(matches!(cid, Value::Str(s) if s.as_str() == "mac"));
        let ipv6 = s.get(SECTION_IPV6).expect("ipv6 section created");
        let duid: &Value = ipv6.get(KEY_DHCP_DUID).unwrap();
        assert!(matches!(duid, Value::Str(s) if s.as_str() == "ll"));
        let iaid: &Value = ipv6.get(KEY_DHCP_IAID).unwrap();
        assert!(matches!(iaid, Value::Str(s) if s.as_str() == "mac"));
    }

    #[test]
    fn apply_idempotent_when_already_set() {
        let mut s = empty_settings();
        apply_dhcp_settings(&mut s, true, true, true).unwrap();
        let again = apply_dhcp_settings(&mut s, true, true, true).unwrap();
        assert!(!again, "second apply should be a no-op");
    }

    #[test]
    fn apply_partial_skips_disabled_categories() {
        let mut s = empty_settings();
        apply_dhcp_settings(&mut s, false, true, false).unwrap();
        let ipv4 = s.get(SECTION_IPV4).expect("ipv4 set for vendor-class");
        assert!(ipv4.get(KEY_DHCP_SEND_HOSTNAME).is_none());
        assert!(ipv4.get(KEY_DHCP_CLIENT_ID).is_none());
        assert!(ipv4.contains_key(KEY_DHCP_VENDOR_CLASS_IDENTIFIER));
        assert!(!s.contains_key(SECTION_IPV6));
    }

    #[test]
    fn revert_does_not_materialize_empty_ipv6_section() {
        // Issue #151 — when the snapshot has no IPv6-shaped values, revert
        // would still create an empty `[ipv6]` section because every
        // restore_* helper called `entry().or_default()` unconditionally.
        // After the fix, an absent IPv6 dict stays absent.
        let mut s = empty_settings();
        let snap = snapshot_dhcp(&s);
        revert_dhcp_settings(&mut s, &snap).unwrap();
        assert!(
            !s.contains_key(SECTION_IPV6),
            "revert should not materialize an empty ipv6 section, got {s:?}"
        );
        assert!(
            !s.contains_key(SECTION_IPV4),
            "revert should not materialize an empty ipv4 section either"
        );
    }

    #[test]
    fn snapshot_then_revert_is_a_no_op_round_trip() {
        let mut s = empty_settings();
        let ipv4 = s.entry(SECTION_IPV4.to_string()).or_default();
        ipv4.insert(
            KEY_DHCP_SEND_HOSTNAME.to_string(),
            Value::from(true).try_into().unwrap(),
        );
        ipv4.insert(
            KEY_DHCP_CLIENT_ID.to_string(),
            Value::from("duid".to_string()).try_into().unwrap(),
        );
        let snap = snapshot_dhcp(&s);
        // Apply suppression, then revert with the snapshot, then check that
        // the result snapshot matches the original one.
        apply_dhcp_settings(&mut s, true, true, true).unwrap();
        revert_dhcp_settings(&mut s, &snap).unwrap();
        let after = snapshot_dhcp(&s);
        assert_eq!(after, snap);
    }

    #[test]
    fn user_data_tag_round_trip() {
        let mut s = empty_settings();
        tag_user_data(&mut s, "0.1.0", "2026-05-06T00:00:00Z").unwrap();
        assert!(is_proteus_managed(&s));
        untag_user_data(&mut s).unwrap();
        assert!(!is_proteus_managed(&s));
        // Once untagged, the section should either be missing user-data or
        // gone entirely.
        if let Some(section) = s.get(SECTION_CONNECTION) {
            assert!(section.get(KEY_USER_DATA).is_none());
        }
    }

    // === Roadmap M2 "Integration" — apply_persona_fingerprint ===

    fn persona_with_dhcp(vendor: &str, host_name: &str, fqdn: &str) -> crate::persona::Persona {
        crate::persona::Persona {
            id: "test".into(),
            display_name: "Test".into(),
            kind: crate::persona::PersonaKind::Stealth,
            category: crate::persona::PersonaCategory::Phone,
            oui_pool: vec!["apple".into()],
            mac_byte_pattern: None,
            hostname_template: "h".into(),
            dhcp_fingerprint: crate::persona::DhcpFingerprint {
                vendor_class_identifier: vendor.into(),
                fqdn: fqdn.into(),
                parameter_request_list: vec![1, 3, 6, 15, 119, 252],
                host_name: host_name.into(),
            },
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: true,
            mdns: Default::default(),
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        }
    }

    #[test]
    fn persona_writes_vendor_class_when_not_suppressed() {
        let mut s = empty_settings();
        let p = persona_with_dhcp("iPhone", "", "");
        let changed = apply_persona_fingerprint(&mut s, &p, false, false).unwrap();
        assert!(changed);
        let v = extract_str(&s, SECTION_IPV4, KEY_DHCP_VENDOR_CLASS_IDENTIFIER);
        assert_eq!(v.as_deref(), Some("iPhone"));
    }

    #[test]
    fn persona_skips_vendor_class_when_user_suppressed() {
        // Per-knob override beats persona: when suppress_vendor_class is
        // true, the persona's vendor_class_identifier must NOT land on
        // the wire.
        let mut s = empty_settings();
        let p = persona_with_dhcp("iPhone", "", "");
        let _ = apply_dhcp_settings(&mut s, true, true, true).unwrap();
        // After suppression: vendor class is empty.
        assert_eq!(
            extract_str(&s, SECTION_IPV4, KEY_DHCP_VENDOR_CLASS_IDENTIFIER).as_deref(),
            Some("")
        );
        // Now apply persona with suppress_vendor_class still true —
        // persona must NOT overwrite the empty string.
        apply_persona_fingerprint(&mut s, &p, true, true).unwrap();
        assert_eq!(
            extract_str(&s, SECTION_IPV4, KEY_DHCP_VENDOR_CLASS_IDENTIFIER).as_deref(),
            Some(""),
            "persona must not override user suppression"
        );
    }

    #[test]
    fn persona_writes_hostname_and_flips_send_hostname_back_on() {
        // Suppression first, then persona: persona's host_name should
        // win and `send_hostname` must be flipped back to true so the
        // option actually leaves the box.
        let mut s = empty_settings();
        let p = persona_with_dhcp("iPhone", "alexs-iphone", "");
        let _ = apply_dhcp_settings(&mut s, true, true, true).unwrap();
        // After suppression: send_hostname is false.
        assert_eq!(
            extract_bool(&s, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME),
            Some(false)
        );
        // Now apply persona with suppress_hostname=false (user un-set it):
        let changed = apply_persona_fingerprint(&mut s, &p, false, false).unwrap();
        assert!(changed);
        assert_eq!(
            extract_bool(&s, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME),
            Some(true),
            "persona must flip send_hostname back on"
        );
        assert_eq!(
            extract_str(&s, SECTION_IPV4, KEY_DHCP_HOSTNAME).as_deref(),
            Some("alexs-iphone")
        );
    }

    #[test]
    fn persona_writes_fqdn_when_set() {
        let mut s = empty_settings();
        let p = persona_with_dhcp("", "", "test.local");
        apply_persona_fingerprint(&mut s, &p, false, false).unwrap();
        assert_eq!(
            extract_str(&s, SECTION_IPV4, KEY_DHCP_FQDN).as_deref(),
            Some("test.local")
        );
    }

    #[test]
    fn persona_skips_hostname_when_user_suppressed() {
        let mut s = empty_settings();
        let p = persona_with_dhcp("", "alexs-iphone", "");
        // Suppression first, then persona with suppress_hostname=true.
        let _ = apply_dhcp_settings(&mut s, true, false, false).unwrap();
        apply_persona_fingerprint(&mut s, &p, true, false).unwrap();
        assert_eq!(
            extract_bool(&s, SECTION_IPV4, KEY_DHCP_SEND_HOSTNAME),
            Some(false),
            "persona must not undo user suppression"
        );
    }

    #[test]
    fn persona_with_empty_fingerprint_is_a_noop() {
        let mut s = empty_settings();
        let p = persona_with_dhcp("", "", "");
        let changed = apply_persona_fingerprint(&mut s, &p, false, false).unwrap();
        assert!(!changed, "empty persona fingerprint should not mutate");
    }

    #[test]
    fn user_data_preserves_third_party_entries() {
        let mut s = empty_settings();
        // Pre-existing user-data set by some other tool.
        let section = s.entry(SECTION_CONNECTION.to_string()).or_default();
        let mut dict = zbus::zvariant::Dict::new(
            &zbus::zvariant::Signature::Str,
            &zbus::zvariant::Signature::Str,
        );
        dict.add("other.tool".to_string(), "yes".to_string())
            .unwrap();
        section.insert(
            KEY_USER_DATA.to_string(),
            Value::Dict(dict).try_to_owned().unwrap(),
        );
        tag_user_data(&mut s, "0.1.0", "now").unwrap();
        untag_user_data(&mut s).unwrap();
        // other.tool entry survives the round trip.
        let map = read_user_data(s.get(SECTION_CONNECTION).unwrap());
        assert_eq!(map.get("other.tool").map(String::as_str), Some("yes"));
        assert!(!map.contains_key("proteus.managed"));
    }
}
