// SPDX-License-Identifier: GPL-3.0-or-later

//! `MockBackend` — in-memory backend used by per-backend integration
//! tests (Milestone 1 acceptance). Lets a test pre-populate devices,
//! observe `set_cloned_mac` calls, and assert outcomes without
//! standing up a DBus daemon or a containerised distro.
//!
//! Gated behind `cfg(test)` so it never ships in the release
//! binary. If out-of-tree integration tests need this later, lift
//! the gate to a `test-mock` feature; the constraint today is "no
//! new dependencies / features", so we keep it test-only.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

use super::{BackendDevice, BoxFuture, NetworkBackend, RotateOutcome};
use crate::mac::Mac;

/// Single observation in [`MockBackend::call_log`]. Test code asserts
/// against this rather than poking at internal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockCall {
    ListDevices,
    SetClonedMac { iface: String, mac: Mac },
    ReadClonedMac { iface: String },
    ReadFactoryMac { iface: String },
    RotateIfNeeded { iface: String, cooldown: Duration },
}

/// Per-iface state the backend tracks. Tests seed the initial values
/// via [`MockBackend::insert_device`].
#[derive(Debug, Clone)]
struct DeviceState {
    device: BackendDevice,
    cloned_mac: Option<String>,
    factory_mac: Option<String>,
    rotate_outcome: RotateOutcome,
}

#[derive(Default)]
struct Inner {
    devices: Vec<DeviceState>,
    calls: Vec<MockCall>,
    available: bool,
}

#[derive(Default)]
pub struct MockBackend {
    inner: Mutex<Inner>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                available: true,
                ..Inner::default()
            }),
        }
    }

    pub fn set_available(&self, available: bool) {
        self.inner.lock().unwrap().available = available;
    }

    /// Seed a device. The default `rotate_outcome` is
    /// `BackendUnavailable`; override per-iface via
    /// [`MockBackend::set_rotate_outcome`].
    pub fn insert_device(&self, device: BackendDevice, factory_mac: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.devices.push(DeviceState {
            device,
            cloned_mac: None,
            factory_mac,
            rotate_outcome: RotateOutcome::BackendUnavailable,
        });
    }

    pub fn set_rotate_outcome(&self, iface: &str, outcome: RotateOutcome) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(d) = inner.devices.iter_mut().find(|d| d.device.iface == iface) {
            d.rotate_outcome = outcome;
        }
    }

    /// Snapshot of the call log in the order calls landed. Returns
    /// owned values so the test can drop the borrow before further
    /// trait calls.
    pub fn call_log(&self) -> Vec<MockCall> {
        self.inner.lock().unwrap().calls.clone()
    }

    /// Last cloned MAC written for `iface`, if any. Useful when a
    /// test cares about the final state, not the sequence.
    pub fn cloned_mac_for(&self, iface: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .devices
            .iter()
            .find(|d| d.device.iface == iface)
            .and_then(|d| d.cloned_mac.clone())
    }
}

impl NetworkBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, bool> {
        let v = self.inner.lock().unwrap().available;
        Box::pin(async move { v })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(MockCall::ListDevices);
        let out: Vec<BackendDevice> = inner.devices.iter().map(|d| d.device.clone()).collect();
        Box::pin(async move { Ok(out) })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
        mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(MockCall::SetClonedMac {
            iface: device.iface.clone(),
            mac,
        });
        if let Some(d) = inner
            .devices
            .iter_mut()
            .find(|d| d.device.iface == device.iface)
        {
            d.cloned_mac = Some(mac.to_string());
        }
        Box::pin(async move { Ok(()) })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(MockCall::ReadClonedMac {
            iface: device.iface.clone(),
        });
        let out = inner
            .devices
            .iter()
            .find(|d| d.device.iface == device.iface)
            .and_then(|d| d.cloned_mac.clone());
        Box::pin(async move { Ok(out) })
    }

    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(MockCall::ReadFactoryMac {
            iface: iface.to_string(),
        });
        let out = inner
            .devices
            .iter()
            .find(|d| d.device.iface == iface)
            .and_then(|d| d.factory_mac.clone());
        Box::pin(async move { Ok(out) })
    }

    fn rotate_if_needed<'a>(
        &'a self,
        iface: &'a str,
        cooldown: Duration,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(MockCall::RotateIfNeeded {
            iface: iface.to_string(),
            cooldown,
        });
        let out = inner
            .devices
            .iter()
            .find(|d| d.device.iface == iface)
            .map(|d| d.rotate_outcome.clone())
            .unwrap_or(RotateOutcome::BackendUnavailable);
        Box::pin(async move { Ok(out) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn dev(iface: &str) -> BackendDevice {
        BackendDevice {
            iface: iface.into(),
            kind: BackendKind::Wifi,
            hw_address: Some("aa:bb:cc:dd:ee:ff".into()),
            identifier: format!("mock://{iface}"),
        }
    }

    #[test]
    fn round_trip_set_then_read_cloned_mac() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), Some("aa:bb:cc:dd:ee:ff".into()));
        let mac: Mac = "02:11:22:33:44:55".parse().unwrap();

        rt().block_on(async {
            let devices = backend.list_devices().await.unwrap();
            assert_eq!(devices.len(), 1);
            backend.set_cloned_mac(&devices[0], mac).await.unwrap();
            let read = backend.read_cloned_mac(&devices[0]).await.unwrap();
            assert_eq!(read.as_deref(), Some("02:11:22:33:44:55"));
        });

        let log = backend.call_log();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0], MockCall::ListDevices);
        assert!(matches!(log[1], MockCall::SetClonedMac { .. }));
        assert!(matches!(log[2], MockCall::ReadClonedMac { .. }));
    }

    #[test]
    fn read_factory_mac_returns_seeded_value() {
        let backend = MockBackend::new();
        backend.insert_device(dev("eth0"), Some("11:22:33:44:55:66".into()));
        rt().block_on(async {
            let got = backend.read_factory_mac("eth0").await.unwrap();
            assert_eq!(got.as_deref(), Some("11:22:33:44:55:66"));
        });
    }

    #[test]
    fn rotate_outcome_is_configurable_per_iface() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), None);
        let mac: Mac = "06:aa:bb:cc:dd:ee".parse().unwrap();
        backend.set_rotate_outcome("wlan0", RotateOutcome::Rotated { new_mac: mac });
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", Duration::from_secs(60))
                .await
                .unwrap();
            assert_eq!(outcome, RotateOutcome::Rotated { new_mac: mac });
        });
    }

    #[test]
    fn unknown_iface_rotate_falls_back_to_backend_unavailable() {
        let backend = MockBackend::new();
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("eth9", Duration::from_secs(0))
                .await
                .unwrap();
            assert_eq!(outcome, RotateOutcome::BackendUnavailable);
        });
    }

    #[test]
    fn available_flag_can_be_toggled() {
        let backend = MockBackend::new();
        rt().block_on(async {
            assert!(backend.available().await);
            backend.set_available(false);
            assert!(!backend.available().await);
        });
    }
}
