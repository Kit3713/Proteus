// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use zbus::zvariant::{Array, OwnedObjectPath, OwnedValue, Value};

use super::{ConnectionProxy, ConnectionSettings, DeviceKind};
use crate::mac::Mac;

pub async fn set_cloned_mac(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    kind: DeviceKind,
    mac: Mac,
) -> Result<()> {
    let key = kind.setting_key().ok_or_else(|| {
        anyhow!(
            "device type {:?} does not expose a cloned-MAC setting key",
            kind
        )
    })?;
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let mut settings: ConnectionSettings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    let entry = settings.entry(key.to_string()).or_default();
    // cloned-mac-address is `ay` per NM's introspection XML; NM 1.20-1.36
    // hard-rejects a string and leaves the connection half-updated.
    entry.insert(
        "cloned-mac-address".to_string(),
        cloned_mac_value(mac).try_into()?,
    );
    // assigned-mac-address stays a string in NM's API; setting both lets
    // older NM honour the rotation when it ignores cloned-mac-address.
    entry.insert(
        "assigned-mac-address".to_string(),
        Value::from(mac.to_string()).try_into()?,
    );
    // Issue #207: rotate must preserve the WPA-PSK / 802.1X password the user
    // already saved on the profile, so we go through `update_with_secrets`
    // which round-trips secrets back through `GetSecrets` before `Update`.
    super::update_with_secrets(conn, connection_path, settings).await
}

fn cloned_mac_value(mac: Mac) -> Value<'static> {
    Value::Array(Array::from(mac.octets().to_vec()))
}

pub async fn read_cloned_mac(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    kind: DeviceKind,
) -> Result<Option<String>> {
    let key = match kind.setting_key() {
        Some(k) => k,
        None => return Ok(None),
    };
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let settings = proxy.get_settings().await?;
    Ok(extract_cloned_mac(&settings, key))
}

/// Read `<key>.cloned-mac-address` from a settings dict, accepting either the
/// modern `ay` byte form (what `set_cloned_mac` now writes) or the legacy
/// `Value::Str` form that older NM versions and pre-fix Proteus releases used.
/// Returns the canonical `aa:bb:cc:dd:ee:ff` rendering either way.
fn extract_cloned_mac(
    settings: &HashMap<String, HashMap<String, OwnedValue>>,
    section: &str,
) -> Option<String> {
    let sec = settings.get(section)?;
    let v: &Value = sec.get("cloned-mac-address")?;
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        Value::Array(arr) => mac_from_byte_array(arr).map(|m| m.to_string()),
        _ => None,
    }
}

fn mac_from_byte_array(arr: &Array<'_>) -> Option<Mac> {
    let mut out = [0u8; 6];
    if arr.len() != 6 {
        return None;
    }
    for (slot, v) in out.iter_mut().zip(arr.iter()) {
        match v {
            Value::U8(b) => *slot = *b,
            _ => return None,
        }
    }
    Some(Mac::new(out))
}

pub async fn read_connection_id(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
) -> Result<Option<String>> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let settings = proxy.get_settings().await?;
    Ok(extract_str(&settings, "connection", "id"))
}

/// Issue #124: read `connection.uuid` for state keying. Returns `Ok(None)`
/// only if NM's `Settings.Connection.GetSettings` succeeded but the dict
/// has no uuid field — every well-formed NM connection has one, so this
/// is treated as a soft skip rather than a hard error in callers.
pub async fn read_connection_uuid(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
) -> Result<Option<String>> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let settings = proxy.get_settings().await?;
    Ok(extract_str(&settings, "connection", "uuid"))
}

pub async fn find_connection_by_id(
    conn: &zbus::Connection,
    id: &str,
) -> Result<(OwnedObjectPath, ConnectionSettings)> {
    let settings_proxy = super::SettingsProxy::new(conn).await?;
    let conns = settings_proxy.list_connections().await?;
    for path in conns {
        let proxy = ConnectionProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        if let Ok(s) = proxy.get_settings().await
            && let Some(found_id) = extract_str(&s, "connection", "id")
            && found_id == id
        {
            return Ok((path, s));
        }
    }
    bail!("no NetworkManager connection profile with id '{id}'")
}

fn extract_str(
    settings: &HashMap<String, HashMap<String, OwnedValue>>,
    section: &str,
    key: &str,
) -> Option<String> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &Value = val;
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_cloned_mac(settings: &mut ConnectionSettings, section: &str, value: Value<'static>) {
        let entry = settings.entry(section.to_string()).or_default();
        entry.insert("cloned-mac-address".to_string(), value.try_into().unwrap());
    }

    #[test]
    fn cloned_mac_value_emits_six_byte_array() {
        let mac: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let v = cloned_mac_value(mac);
        match v {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 6);
                let parsed = mac_from_byte_array(&arr).unwrap();
                assert_eq!(parsed, mac);
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn extract_cloned_mac_reads_byte_form() {
        let mac: Mac = "11:22:33:44:55:66".parse().unwrap();
        let mut settings: ConnectionSettings = HashMap::new();
        put_cloned_mac(&mut settings, "802-3-ethernet", cloned_mac_value(mac));
        let got = extract_cloned_mac(&settings, "802-3-ethernet");
        assert_eq!(got.as_deref(), Some("11:22:33:44:55:66"));
    }

    #[test]
    fn extract_cloned_mac_reads_legacy_string_form() {
        let mut settings: ConnectionSettings = HashMap::new();
        put_cloned_mac(
            &mut settings,
            "802-11-wireless",
            Value::from("AA:BB:CC:DD:EE:FF".to_string()),
        );
        let got = extract_cloned_mac(&settings, "802-11-wireless");
        assert_eq!(got.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    }
}
