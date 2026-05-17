// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::networkd` — scaffold for Milestone 1; full impl pending.
//! Will speak `org.freedesktop.network1` DBus + write drop-ins under
//! `/etc/systemd/network/`. Today every method bails so the trait
//! object compiles and `proteus doctor` can advertise availability,
//! but no command actually routes through it yet.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::{BackendDevice, BoxFuture, ConnectionRef, NetworkBackend, RenewOutcome, RotateOutcome};
use crate::ipv6::nm::Ipv6NmSettings;
use crate::mac::{Mac, factory};
use crate::state::DhcpSettingsSnapshot;

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
        // Issue #247: `/run/systemd/network` is just a tmpfiles.d-managed
        // config directory that exists on every systemd host whether or
        // not `systemd-networkd` is running. Using it as the availability
        // signal made `apply` claim networkd was usable on NM-only hosts,
        // and every backend method then bailed with the milestone-1
        // "not yet implemented" error.
        //
        // The honest signal is `/run/systemd/netif/`, which the
        // networkd daemon creates and populates on startup. When the
        // unit is `inactive` the directory does not exist; when the
        // unit is `active` it holds the per-link state subtree. We also
        // require the config directory to be writeable since every
        // future networkd backend method writes a drop-in there — a
        // backend that can't persist its config can't honestly claim
        // availability either.
        Box::pin(async { Path::new("/run/systemd/netif").is_dir() && config_dir_writeable() })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async {
            bail!(
                "backend::networkd: list_devices not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn list_connections<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Vec<ConnectionRef>>> {
        Box::pin(async {
            bail!(
                "backend::networkd: list_connections not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
        _mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!(
                "backend::networkd: set_cloned_mac not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async {
            bail!(
                "backend::networkd: read_cloned_mac not yet implemented (Milestone 1); see proteus wiki backend"
            )
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
        _state_path: Option<&'a std::path::Path>,
        _reason: Option<&'a str>,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        Box::pin(async { Ok(RotateOutcome::BackendUnavailable) })
    }

    fn read_connection_id<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }

    fn read_connection_uuid<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }

    fn set_dhcp_settings<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
        _snapshot: DhcpSettingsSnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!(
                "backend::networkd: set_dhcp_settings not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn set_ipv6_settings<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
        _settings: Ipv6NmSettings,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!(
                "backend::networkd: set_ipv6_settings not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn renew_lease<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<RenewOutcome>> {
        Box::pin(async {
            bail!(
                "backend::networkd: renew_lease not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }

    fn write_anonymous_identity<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
        _value: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!(
                "backend::networkd: write_anonymous_identity not yet implemented (Milestone 1); see proteus wiki backend"
            )
        })
    }
}

/// Is `/etc/systemd/network/` writeable by the current process? The full
/// networkd impl will land drop-ins there, so a backend that can't write
/// can't honestly call itself available — even if the daemon is running.
///
/// We probe by attempting to create+remove a sentinel file. `access(2)`
/// would lie under capabilities like CAP_DAC_OVERRIDE that change the
/// effective write right; an actual create is the only honest test.
fn config_dir_writeable() -> bool {
    let dir = Path::new("/etc/systemd/network");
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(
        ".proteus-availability-probe-{}",
        std::process::id()
    ));
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&probe);
    match opened {
        Ok(f) => {
            drop(f);
            // Best-effort cleanup; if the unlink fails (rare; permission
            // changed mid-probe), the sentinel name carries our pid so
            // an operator can identify and clean it.
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
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
                .rotate_if_needed("wlan0", Duration::from_secs(60), None, None)
                .await
                .unwrap();
            assert_eq!(outcome, RotateOutcome::BackendUnavailable);
        });
    }

    #[test]
    fn connection_reads_return_none_not_err() {
        // The stub backends should return `Ok(None)` for the
        // optional-string reads so the per-command code can fall
        // through cleanly. Only the mutating helpers bail.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let backend = NetworkdBackend::new();
        let cref = ConnectionRef::new("");
        rt.block_on(async {
            assert!(backend.read_connection_id(&cref).await.unwrap().is_none());
            assert!(backend.read_connection_uuid(&cref).await.unwrap().is_none());
        });
    }

    /// Issue #247: `available()` returned `true` on every systemd host
    /// because `/run/systemd/network` is a tmpfiles.d config directory
    /// that exists whether or not networkd is running. A non-root test
    /// process can't write `/etc/systemd/network/` so the second arm of
    /// the new probe always trips here — pin that the result is `false`
    /// in the test process. The runtime check (`/run/systemd/netif`)
    /// is also rare on a CI host without networkd active. Either gate
    /// being false is enough to flip availability to false; we just
    /// assert one of them holds so the test isn't tied to runtime
    /// daemons CI may or may not start.
    #[test]
    fn available_is_false_when_either_gate_fails() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let backend = NetworkdBackend::new();
        let netif_present = Path::new("/run/systemd/netif").is_dir();
        let writeable = super::config_dir_writeable();
        let result = rt.block_on(async { backend.available().await });
        // The honest answer is the conjunction of both gates; a backend
        // that can't write its drop-ins can't honestly claim usability,
        // and a daemon that isn't running can't either.
        assert_eq!(result, netif_present && writeable);
    }
}
