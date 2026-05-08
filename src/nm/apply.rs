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

/// Roadmap Milestone 4b: per-scan MAC randomization + probe-request hygiene
/// for one Wi-Fi connection. Writes `wifi.scan-rand-mac-address = "random"`
/// (random source MAC for every scan request the supplicant emits) and
/// `wifi.mac-address-randomization = 2` (NM's "always randomize" enum)
/// into the `802-11-wireless` setting block. The two keys together tell
/// NM + wpa_supplicant to emit `mac_addr=2` plus to never broadcast the
/// saved-SSID list as a byproduct of probing — see
/// `wiki/wpa-supplicant-hardening.md`.
///
/// Issue #207: routed through `update_with_secrets` so writing this key
/// preserves the connection's stored PSK / 802.1X password — the same
/// secrets-merge invariant the cloned-MAC path holds.
pub async fn set_scan_rand_mac(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
) -> Result<()> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let mut settings: ConnectionSettings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    if !apply_scan_rand_mac(&mut settings)? {
        // Already set — skip the round trip entirely so a no-op apply
        // doesn't trigger NM's connection-changed signal storm.
        tracing::debug!("scan-rand-mac already set; skipping update");
        return Ok(());
    }
    super::update_with_secrets(conn, connection_path, settings).await
}

/// Set the two NM scan-randomization keys on the `802-11-wireless` section
/// of `settings`. Returns `Ok(true)` iff at least one key changed — the
/// caller can short-circuit the round trip on a no-op. Lifted out of
/// `set_scan_rand_mac` so the round-trip behaviour is unit-testable
/// without DBus.
pub fn apply_scan_rand_mac(settings: &mut ConnectionSettings) -> Result<bool> {
    let section = settings.entry("802-11-wireless".to_string()).or_default();
    let mut changed = false;

    let want_mac = "random";
    let cur_mac = section.get("scan-rand-mac-address").and_then(|v| {
        let val: &Value = v;
        if let Value::Str(s) = val {
            Some(s.as_str().to_string())
        } else {
            None
        }
    });
    if cur_mac.as_deref() != Some(want_mac) {
        section.insert(
            "scan-rand-mac-address".to_string(),
            Value::from(want_mac.to_string()).try_into()?,
        );
        changed = true;
    }

    // `mac-address-randomization` is an `i` (i32) on NM's DBus surface.
    // 0 = default (driver picks), 1 = never, 2 = always. We always want
    // 2 here; some older NM tolerates a string but 1.20–1.36 reject it
    // (same shape as `addr_gen_mode_to_int` in `nm::mod.rs`).
    let want_rand: i32 = 2;
    let cur_rand = section.get("mac-address-randomization").and_then(|v| {
        let val: &Value = v;
        if let Value::I32(i) = val {
            Some(*i)
        } else {
            None
        }
    });
    if cur_rand != Some(want_rand) {
        section.insert(
            "mac-address-randomization".to_string(),
            Value::from(want_rand).try_into()?,
        );
        changed = true;
    }

    Ok(changed)
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

    // ---- Milestone 4b: per-scan MAC randomization round-trip ----

    fn read_str(settings: &ConnectionSettings, section: &str, key: &str) -> Option<String> {
        let sec = settings.get(section)?;
        let v: &Value = sec.get(key)?;
        if let Value::Str(s) = v {
            Some(s.as_str().to_string())
        } else {
            None
        }
    }

    fn read_i32(settings: &ConnectionSettings, section: &str, key: &str) -> Option<i32> {
        let sec = settings.get(section)?;
        let v: &Value = sec.get(key)?;
        if let Value::I32(i) = v {
            Some(*i)
        } else {
            None
        }
    }

    #[test]
    fn apply_scan_rand_mac_sets_both_keys_on_empty_settings() {
        let mut settings: ConnectionSettings = HashMap::new();
        let changed = apply_scan_rand_mac(&mut settings).unwrap();
        assert!(changed);
        assert_eq!(
            read_str(&settings, "802-11-wireless", "scan-rand-mac-address").as_deref(),
            Some("random")
        );
        assert_eq!(
            read_i32(&settings, "802-11-wireless", "mac-address-randomization"),
            Some(2)
        );
    }

    #[test]
    fn apply_scan_rand_mac_is_idempotent() {
        let mut settings: ConnectionSettings = HashMap::new();
        assert!(apply_scan_rand_mac(&mut settings).unwrap());
        let again = apply_scan_rand_mac(&mut settings).unwrap();
        assert!(!again, "second call must report no change");
    }

    #[test]
    fn apply_scan_rand_mac_overwrites_conflicting_value() {
        // A connection with `scan-rand-mac-address = "permanent"` should be
        // upgraded to "random" — Proteus owns the key once configured.
        let mut settings: ConnectionSettings = HashMap::new();
        let section = settings.entry("802-11-wireless".to_string()).or_default();
        section.insert(
            "scan-rand-mac-address".to_string(),
            Value::from("permanent".to_string()).try_into().unwrap(),
        );
        let changed = apply_scan_rand_mac(&mut settings).unwrap();
        assert!(changed);
        assert_eq!(
            read_str(&settings, "802-11-wireless", "scan-rand-mac-address").as_deref(),
            Some("random")
        );
    }

    #[test]
    fn apply_scan_rand_mac_preserves_unrelated_keys_in_section() {
        // Connections frequently carry `ssid`, `mode`, `band` etc. in the
        // 802-11-wireless section; the scan-rand write must not clobber
        // those (it lives in the same `update_with_secrets` round trip
        // as `set_cloned_mac`, which has the same invariant).
        let mut settings: ConnectionSettings = HashMap::new();
        let section = settings.entry("802-11-wireless".to_string()).or_default();
        section.insert(
            "ssid".to_string(),
            Value::from("home-net".to_string()).try_into().unwrap(),
        );
        let _ = apply_scan_rand_mac(&mut settings).unwrap();
        assert_eq!(
            read_str(&settings, "802-11-wireless", "ssid").as_deref(),
            Some("home-net")
        );
    }

    /// Issue #207: the secrets-merge round trip must not clobber the WPA-PSK
    /// when the only thing changing is the scan-rand-mac knob. We can't
    /// drive `update_with_secrets` from a unit test (it needs a live NM
    /// DBus), but we can prove the helper that landings under it leaves
    /// the secrets section alone.
    #[test]
    fn apply_scan_rand_mac_leaves_secrets_section_untouched() {
        let mut settings: ConnectionSettings = HashMap::new();
        let secrets = settings
            .entry("802-11-wireless-security".to_string())
            .or_default();
        secrets.insert(
            "psk".to_string(),
            Value::from("hunter2-the-original".to_string())
                .try_into()
                .unwrap(),
        );
        let _ = apply_scan_rand_mac(&mut settings).unwrap();
        assert_eq!(
            read_str(&settings, "802-11-wireless-security", "psk").as_deref(),
            Some("hunter2-the-original"),
            "the PSK must survive the wifi-key write — secrets merge is what `update_with_secrets` does on top"
        );
    }

    #[test]
    fn apply_scan_rand_mac_creates_section_if_missing() {
        // A profile with no `802-11-wireless` section yet (rare — every
        // wifi profile has one) shouldn't crash; the helper materialises
        // it and the writer downstream sees the right shape.
        let mut settings: ConnectionSettings = HashMap::new();
        let _ = apply_scan_rand_mac(&mut settings).unwrap();
        assert!(settings.contains_key("802-11-wireless"));
    }
}
