// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::raw` — scaffold for Milestone 1; full impl pending. Will
//! drive `ip` + `iw` + `wpa_supplicant`/`iwd` directly for the any-distro
//! fallback. Today every method bails so the trait object compiles
//! and `proteus doctor` can advertise availability.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::{BackendDevice, BoxFuture, NetworkBackend, RotateOutcome};
use crate::mac::{Mac, factory};

#[derive(Debug, Default)]
pub struct RawBackend;

impl RawBackend {
    pub fn new() -> Self {
        Self
    }
}

impl NetworkBackend for RawBackend {
    fn name(&self) -> &'static str {
        "raw"
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, bool> {
        // The full impl will need `iw` and one of
        // `wpa_supplicant`/`iwd` for Wi-Fi, but `ip` alone is enough
        // for the trait scaffolding to advertise availability — the
        // ethernet / wired path needs nothing else, and the Wi-Fi
        // paths surface their own missing-binary errors when invoked.
        Box::pin(async {
            Path::new("/sbin/ip").exists() || Path::new("/usr/bin/ip").exists()
        })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async { bail!("backend::raw: list_devices not yet implemented (Milestone 1)") })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
        _mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!("backend::raw: set_cloned_mac not yet implemented (Milestone 1)")
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async {
            bail!("backend::raw: read_cloned_mac not yet implemented (Milestone 1)")
        })
    }

    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
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
        assert_eq!(RawBackend::new().name(), "raw");
    }

    #[test]
    fn list_devices_bails_for_now() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let backend = RawBackend::new();
            assert!(backend.list_devices().await.is_err());
        });
    }
}
