// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::{ConnectionProxy, ConnectionSettings, DeviceKind};
use crate::mac::Mac;

pub async fn set_cloned_mac(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    kind: DeviceKind,
    mac: Mac,
) -> Result<()> {
    set_cloned_mac_returning_ids(conn, connection_path, kind, mac)
        .await
        .map(|_| ())
}

/// Same as `set_cloned_mac` but also returns the profile's `id` and `uuid`
/// captured from the same `GetSettings` call. Lets callers like `rotate`
/// avoid two extra DBus round-trips per profile (issue #122 hot path).
pub async fn set_cloned_mac_returning_ids(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    kind: DeviceKind,
    mac: Mac,
) -> Result<(Option<String>, Option<String>)> {
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
    let id = extract_str(&settings, "connection", "id");
    let uuid = extract_str(&settings, "connection", "uuid");
    let mac_str = mac.to_string();
    let entry = settings.entry(key.to_string()).or_default();
    entry.insert(
        "cloned-mac-address".to_string(),
        Value::from(mac_str.clone()).try_into()?,
    );
    // Belt and braces: also set assigned-mac-address so older NMs honor it.
    entry.insert(
        "assigned-mac-address".to_string(),
        Value::from(mac_str).try_into()?,
    );
    proxy
        .update(settings)
        .await
        .context("calling Settings.Connection.Update")?;
    Ok((id, uuid))
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
    Ok(extract_str(&settings, key, "cloned-mac-address"))
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

/// Read the `connection.uuid` for a single profile. NM-assigned, stable across
/// renames; the canonical state-keying field per issue #124.
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
    let mut matches = find_connections_by_id(conn, id).await?;
    if matches.is_empty() {
        bail!("no NetworkManager connection profile with id '{id}'")
    }
    if matches.len() > 1 {
        bail!(
            "id '{id}' matches {} connection profiles; pass the uuid to disambiguate",
            matches.len()
        )
    }
    Ok(matches.remove(0))
}

/// Return every NM connection whose `connection.id` matches. NM's id field is
/// not unique (issue #124), so callers that want uniqueness should key by
/// uuid; commands that mutate per-id should iterate this list instead of
/// taking just the first match (issue #122).
pub async fn find_connections_by_id(
    conn: &zbus::Connection,
    id: &str,
) -> Result<Vec<(OwnedObjectPath, ConnectionSettings)>> {
    let settings_proxy = super::SettingsProxy::new(conn).await?;
    let conns = settings_proxy.list_connections().await?;
    let mut out = Vec::new();
    for path in conns {
        let proxy = ConnectionProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        if let Ok(s) = proxy.get_settings().await
            && let Some(found_id) = extract_str(&s, "connection", "id")
            && found_id == id
        {
            out.push((path, s));
        }
    }
    Ok(out)
}

/// Look up an NM connection by its stable `connection.uuid` field. Returns
/// the path and full settings dict; errors if no profile carries the uuid.
pub async fn find_connection_by_uuid(
    conn: &zbus::Connection,
    uuid: &str,
) -> Result<(OwnedObjectPath, ConnectionSettings)> {
    let settings_proxy = super::SettingsProxy::new(conn).await?;
    let path = settings_proxy
        .get_connection_by_uuid(uuid)
        .await
        .with_context(|| format!("no NM connection with uuid '{uuid}'"))?;
    let proxy = ConnectionProxy::builder(conn)
        .path(path.clone())?
        .build()
        .await?;
    let settings = proxy.get_settings().await?;
    Ok((path, settings))
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
