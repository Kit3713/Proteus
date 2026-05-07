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
    Ok(())
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
