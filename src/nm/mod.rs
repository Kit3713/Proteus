// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod dhcp;

use anyhow::{Context, Result, anyhow, bail};
use zbus::proxy;
use zbus::zvariant::OwnedObjectPath;

use crate::mac::Mac;

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
    fn get_device_by_ip_iface(&self, iface: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    #[zbus(property, name = "Managed")]
    fn managed(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "AvailableConnections")]
    fn available_connections(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
    fn get_connection_by_uuid(&self, uuid: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

pub type ConnectionSettings = std::collections::HashMap<
    String,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
>;

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Connection {
    fn get_settings(&self) -> zbus::Result<ConnectionSettings>;
    /// Fetch the secrets dict for one setting (e.g. `"802-1x"`). NM returns
    /// the secrets keyed by setting name — the keys inside (e.g. `password`,
    /// `private-key-password`) are exactly what NM accepts back through
    /// `Update`, so the result can be merged straight into the settings
    /// dict before calling `Update` to avoid clobbering the secrets store.
    fn get_secrets(&self, setting_name: &str) -> zbus::Result<ConnectionSettings>;
    fn update(&self, settings: ConnectionSettings) -> zbus::Result<()>;
}

// NetworkManager device-type integer constants (subset).
pub const DEVICE_TYPE_ETHERNET: u32 = 1;
pub const DEVICE_TYPE_WIFI: u32 = 2;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub interface: String,
    pub kind: DeviceKind,
    pub hw_address: Option<String>,
    pub path: zbus::zvariant::OwnedObjectPath,
    pub managed: bool,
    pub connections: Vec<zbus::zvariant::OwnedObjectPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Wifi,
    Ethernet,
    Other(u32),
}

impl DeviceKind {
    pub fn from_nm(i: u32) -> Self {
        match i {
            DEVICE_TYPE_ETHERNET => Self::Ethernet,
            DEVICE_TYPE_WIFI => Self::Wifi,
            other => Self::Other(other),
        }
    }

    pub fn setting_key(&self) -> Option<&'static str> {
        match self {
            Self::Wifi => Some("802-11-wireless"),
            Self::Ethernet => Some("802-3-ethernet"),
            Self::Other(_) => None,
        }
    }
}

pub async fn list_devices(conn: &zbus::Connection) -> Result<Vec<DeviceInfo>> {
    let nm = NetworkManagerProxy::new(conn)
        .await
        .context("connecting to NetworkManager DBus")?;
    let paths = nm
        .get_devices()
        .await
        .context("calling NetworkManager.GetDevices")?;
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let dev = DeviceProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        let iface = dev.interface().await.unwrap_or_default();
        let dt = dev.device_type().await.unwrap_or(0);
        let hw = dev.hw_address().await.ok();
        let managed = dev.managed().await.unwrap_or(false);
        let conns = dev.available_connections().await.unwrap_or_default();
        out.push(DeviceInfo {
            interface: iface,
            kind: DeviceKind::from_nm(dt),
            hw_address: hw,
            path,
            managed,
            connections: conns,
        });
    }
    Ok(out)
}

pub async fn find_device_by_iface(conn: &zbus::Connection, iface: &str) -> Result<DeviceInfo> {
    let devs = list_devices(conn).await?;
    devs.into_iter()
        .find(|d| d.interface == iface)
        .ok_or_else(|| anyhow!("no NetworkManager device for interface '{iface}'"))
}

/// Parse a colon/dash/bare-hex MAC string into the 6-byte vector NM expects on
/// the wire. NM's `cloned-mac-address` (and equivalent on `802-3-ethernet`) is
/// declared as `ay` in the DBus introspection XML; older NM (1.20–1.36) hard
/// rejects a string. We feed the result through `Mac::from_str` so callers get
/// the same parse behaviour and error messages they already have for
/// rotation/pin paths.
pub fn mac_string_to_bytes(s: &str) -> Result<Vec<u8>> {
    let mac: Mac = s
        .parse()
        .with_context(|| format!("parsing MAC '{s}' for NM cloned-mac-address (ay)"))?;
    Ok(mac.octets().to_vec())
}

/// Connection-setting sections that may carry NM-stored secrets. Whenever we
/// `Settings.Connection.Update` a profile, we must merge each of these
/// sections' `GetSecrets` results back in or NM will interpret the absence of
/// the keys as "user cleared their password" and wipe its secrets store.
///
/// Issue #207 (and original #114 fix): four call sites mutate connection
/// settings via `Update` — `nm::apply::set_cloned_mac`, `nm::dhcp::update_connection`,
/// `ipv6::nm::apply_settings`, and `enterprise_wifi::nm::write_anonymous_identity`.
/// Each one must round-trip through this list, not just the section it
/// directly touches: rotating a Wi-Fi MAC must preserve the WPA-PSK; updating
/// 802.1X anonymous-identity must preserve PEAP/EAP-TLS passwords; updating
/// IPv6 keys on an enterprise Wi-Fi connection must preserve both. The list
/// is the union of every secret-bearing section NM exposes that Proteus
/// could plausibly touch.
pub const SECRET_SECTIONS: &[&str] = &[
    "802-11-wireless-security",
    "802-1x",
    "vpn",
    "wireguard",
    "gsm",
    "cdma",
    "pppoe",
    "macsec",
];

/// Merge a `GetSecrets` result into a settings dict in place.
///
/// NM's `GetSecrets(setting_name)` returns a dict shaped like
/// `{ "802-1x": { "password": ..., "private-key-password": ... } }` — only
/// the secret-typed keys, keyed by section. We graft each section's secrets
/// onto the matching section in `settings`, preserving any settings already
/// in place (so the caller's freshly-modified key survives) and overwriting
/// only on key collisions inside a section.
///
/// Issue #114 / #207: without this merge, `Update` would be called with a
/// dict that lacks the secret keys NM already has stored, and NM interprets
/// that as "the user removed their password".
pub fn merge_secrets(settings: &mut ConnectionSettings, secrets: &ConnectionSettings) {
    for (section_name, section_secrets) in secrets {
        let target = settings.entry(section_name.clone()).or_default();
        for (key, value) in section_secrets {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// Push a `Settings.Connection.Update` after grafting every relevant secrets
/// section back onto `settings`. Issue #207: every NM `Update` site must go
/// through this so a connection's stored PSK / EAP passwords / VPN secrets
/// survive the round trip.
///
/// `GetSettings` strips secret-typed keys, so calling `Update` with the
/// stripped dict tells NM "the user cleared this password" and wipes the
/// secrets store. We pull `GetSecrets` for each section in [`SECRET_SECTIONS`]
/// (best-effort — sections that don't exist on this connection or that
/// are not flagged as "system-owned" return errors NM emits as
/// `NoSecrets`/`PermissionDenied`, which we deliberately swallow) and merge
/// them back in.
pub async fn update_with_secrets(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    mut settings: ConnectionSettings,
) -> Result<()> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    for section in SECRET_SECTIONS {
        // GetSecrets failure modes we tolerate:
        //
        // - NM returns NoSecrets when the section exists but stores no secrets.
        // - NM returns InvalidProperty / generic when the section isn't on this
        //   profile (e.g. asking for `802-1x` on a plain WPA-PSK Wi-Fi).
        // - PermissionDenied if the connection is user-owned and the agent
        //   refuses to surface secrets to a non-interactive caller.
        //
        // None of these is a reason to abort the update — they just mean
        // there's nothing to merge for that section. A real DBus failure on
        // the subsequent `update` call still surfaces.
        match proxy.get_secrets(section).await {
            Ok(s) => merge_secrets(&mut settings, &s),
            Err(e) => {
                tracing::debug!(
                    section = section,
                    "GetSecrets returned no secrets to merge: {e}"
                );
            }
        }
    }
    proxy
        .update(settings)
        .await
        .context("calling Settings.Connection.Update")?;
    Ok(())
}

/// Map an `ipv6.addr-gen-mode` token (as it appears in our config and on the
/// wire in NM keyfile/nmcli) to the integer DBus expects. Per NM's
/// `NMSettingIP6ConfigAddrGenMode` enum:
///
/// - `default`           → `0`
/// - `eui64`             → `1`
/// - `stable-privacy`    → `2`
/// - `default-or-eui64`  → `3`
///
/// The DBus property is signature `i` (i32). NM 1.37+ tolerates a string and
/// coerces, but 1.20–1.36 rejects it, leaving the connection inconsistent.
pub fn addr_gen_mode_to_int(s: &str) -> Result<i32> {
    match s {
        "default" => Ok(0),
        "eui64" => Ok(1),
        "stable-privacy" => Ok(2),
        "default-or-eui64" => Ok(3),
        other => bail!(
            "unknown ipv6.addr-gen-mode '{other}'; expected one of \
             default, eui64, stable-privacy, default-or-eui64"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_string_to_bytes_parses_uppercase_colon_form() {
        let bytes = mac_string_to_bytes("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(bytes, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn mac_string_to_bytes_parses_lowercase_dash_form() {
        let bytes = mac_string_to_bytes("aa-bb-cc-dd-ee-ff").unwrap();
        assert_eq!(bytes, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn mac_string_to_bytes_rejects_garbage() {
        assert!(mac_string_to_bytes("not-a-mac").is_err());
        assert!(mac_string_to_bytes("AA:BB:CC:DD:EE").is_err());
        assert!(mac_string_to_bytes("").is_err());
    }

    #[test]
    fn addr_gen_mode_to_int_known_modes() {
        assert_eq!(addr_gen_mode_to_int("default").unwrap(), 0);
        assert_eq!(addr_gen_mode_to_int("eui64").unwrap(), 1);
        assert_eq!(addr_gen_mode_to_int("stable-privacy").unwrap(), 2);
        assert_eq!(addr_gen_mode_to_int("default-or-eui64").unwrap(), 3);
    }

    #[test]
    fn addr_gen_mode_to_int_rejects_unknown() {
        assert!(addr_gen_mode_to_int("garbage").is_err());
        assert!(addr_gen_mode_to_int("").is_err());
        assert!(addr_gen_mode_to_int("STABLE-PRIVACY").is_err());
    }
}
