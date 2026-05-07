// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::networkd` — scaffold for Milestone 1; full impl pending.
//! Will speak `org.freedesktop.network1` DBus + write drop-ins under
//! `/etc/systemd/network/`. Today every method bails so the trait
//! object compiles and `proteus doctor` can advertise availability,
//! but no command actually routes through it yet.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::{BackendDevice, BoxFuture, NetworkBackend, RotateOutcome};
use crate::mac::{Mac, factory};

#[derive(Debug, Default)]
pub struct NetworkdBackend;

impl NetworkdBackend {
    pub fn new() -> Self {
        Self
    }
}

impl NetworkBackend for NetworkdBackend {
    fn name(&self) -> &'static str {
        "networkd"
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, bool> {
        // Per task brief: probe the runtime path rather than shelling
        // out to `systemctl is-active`. systemd creates
        // /run/systemd/network only when networkd is started, so
        // presence is a strict positive signal.
        Box::pin(async { Path::new("/run/systemd/network").is_dir() })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async {
            bail!("backend::networkd: list_devices not yet implemented (Milestone 1)")
        })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
        _mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!("backend::networkd: set_cloned_mac not yet implemented (Milestone 1)")
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async {
            bail!("backend::networkd: read_cloned_mac not yet implemented (Milestone 1)")
        })
    }

    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        // factory::permanent_address is a sysfs/ethtool read with no
        // backend coupling; reusing it keeps the answer consistent
        // across every backend.
        Box::pin(async move { Ok(factory::permanent_address(iface)) })
    }

    fn rotate_if_needed<'a>(
        &'a self,
        _iface: &'a str,
        _cooldown: Duration,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        Box::pin(async { Ok(RotateOutcome::BackendUnavailable) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_stable_token() {
        assert_eq!(NetworkdBackend::new().name(), "networkd");
    }

    #[test]
    fn unimplemented_methods_return_err_not_panic() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let backend = NetworkdBackend::new();
        rt.block_on(async {
            assert!(backend.list_devices().await.is_err());
        });
    }

    #[test]
    fn rotate_returns_backend_unavailable_for_now() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let backend = NetworkdBackend::new();
        rt.block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", Duration::from_secs(60))
                .await
                .unwrap();
            assert_eq!(outcome, RotateOutcome::BackendUnavailable);
        });
    }
}
