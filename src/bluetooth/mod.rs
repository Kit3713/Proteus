// SPDX-License-Identifier: GPL-3.0-or-later

pub mod alias;
pub mod apply;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use zbus::proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[proxy(
    interface = "org.bluez.Adapter1",
    default_service = "org.bluez",
    gen_blocking = false
)]
pub trait Adapter1 {
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_alias(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn address_type(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn discoverable(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_discoverable(&self, value: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn pairable(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;
}

// ObjectManager.GetManagedObjects returns the entire BlueZ tree. We walk it to
// find Adapter1 objects rather than guessing /org/bluez/hciN paths, which keeps
// us forward-compatible with multi-adapter setups and renames.
#[proxy(
    interface = "org.freedesktop.DBus.ObjectManager",
    default_service = "org.bluez",
    default_path = "/",
    gen_blocking = false
)]
pub trait ObjectManager {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub hci: String,
    pub path: OwnedObjectPath,
    pub address: Option<String>,
    pub address_type: Option<String>,
    pub alias: Option<String>,
    pub name: Option<String>,
    pub discoverable: Option<bool>,
    pub pairable: Option<bool>,
    pub powered: Option<bool>,
    // Approximation: BlueZ exposes AddressType for any controller that
    // surfaces the standard privacy property set. Deeper "this chipset can
    // actually rotate RPAs" requires controller-specific HCI introspection
    // and is out of scope per docs/PLAN.md.
    pub privacy_capable: bool,
    // Currently advertising a random address — usually the result of privacy
    // mode being on. Reads as `false` until the controller is in random mode.
    pub privacy_active: bool,
}

pub fn detect_runtime() -> bool {
    if Path::new("/run/bluetooth").exists() || Path::new("/var/run/bluetooth").exists() {
        return true;
    }
    // Fallback for distros (Fedora 43+) that no longer populate /run/bluetooth
    // by default. /sys/class/bluetooth contains an hciN entry per powered or
    // power-cycled controller.
    has_sysfs_adapters()
}

fn has_sysfs_adapters() -> bool {
    let read = match std::fs::read_dir("/sys/class/bluetooth") {
        Ok(r) => r,
        Err(_) => return false,
    };
    for entry in read.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("hci") {
            return true;
        }
    }
    false
}

pub async fn detect_service(conn: &zbus::Connection) -> bool {
    let proxy = match zbus::fdo::DBusProxy::new(conn).await {
        Ok(p) => p,
        Err(_) => return false,
    };
    let bus = match "org.bluez".try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    proxy.name_has_owner(bus).await.unwrap_or(false)
}

/// Connect to the system bus and list adapters, returning `Ok(None)` when
/// BlueZ is not running. Mutating commands and read commands share this so
/// the "BlueZ not detected" exit path is uniform.
pub async fn connect_and_list() -> Result<Option<(zbus::Connection, Vec<AdapterInfo>)>> {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    if !detect_service(&conn).await {
        return Ok(None);
    }
    let adapters = list_adapters(&conn).await?;
    Ok(Some((conn, adapters)))
}

pub async fn list_adapters(conn: &zbus::Connection) -> Result<Vec<AdapterInfo>> {
    let om = ObjectManagerProxy::new(conn)
        .await
        .context("connecting to BlueZ ObjectManager")?;
    let objects = om
        .get_managed_objects()
        .await
        .context("calling BlueZ ObjectManager.GetManagedObjects")?;
    let mut out = Vec::new();
    for (path, ifaces) in objects {
        let Some(adapter_props) = ifaces.get("org.bluez.Adapter1") else {
            continue;
        };
        out.push(build_adapter_info(path, adapter_props));
    }
    out.sort_by(|a, b| a.hci.cmp(&b.hci));
    Ok(out)
}

fn build_adapter_info(path: OwnedObjectPath, props: &HashMap<String, OwnedValue>) -> AdapterInfo {
    let hci = path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or("hci?")
        .to_string();
    let address_type = prop_string(props, "AddressType");
    let privacy_active = address_type.as_deref() == Some("random");
    // Capability is approximated from BlueZ surface: if AddressType is
    // reported the controller exposes the standard kernel/BlueZ privacy
    // surface. Deeper "controller actually supports RPA rotation" is
    // chipset-specific and out of scope.
    let privacy_capable = address_type.is_some();
    AdapterInfo {
        hci,
        path,
        address: prop_string(props, "Address"),
        address_type,
        alias: prop_string(props, "Alias"),
        name: prop_string(props, "Name"),
        discoverable: prop_bool(props, "Discoverable"),
        pairable: prop_bool(props, "Pairable"),
        powered: prop_bool(props, "Powered"),
        privacy_capable,
        privacy_active,
    }
}

fn prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = props.get(key)?;
    let val: &zbus::zvariant::Value = v;
    if let zbus::zvariant::Value::Str(s) = val {
        Some(s.as_str().to_string())
    } else {
        None
    }
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    let v = props.get(key)?;
    let val: &zbus::zvariant::Value = v;
    if let zbus::zvariant::Value::Bool(b) = val {
        Some(*b)
    } else {
        None
    }
}
