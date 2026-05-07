// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod dhcp;

use anyhow::{Context, Result, anyhow, bail};
use zbus::proxy;

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
