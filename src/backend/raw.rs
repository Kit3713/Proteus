// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::raw` — scaffold for Milestone 1; full impl pending. Will
//! drive `ip` + `iw` + `wpa_supplicant`/`iwd` directly for the any-distro
//! fallback. Today every method bails so the trait object compiles
//! and `proteus doctor` can advertise availability.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::{BackendDevice, BoxFuture, ConnectionRef, NetworkBackend, RenewOutcome, RotateOutcome};
use crate::ipv6::nm::Ipv6NmSettings;
use crate::mac::{Mac, factory};
use crate::state::DhcpSettingsSnapshot;

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
        // Issue #247: the previous gate just checked that `ip` was
        // installed, which is true on essentially every Linux host —
        // including ones where NetworkManager is the active manager.
        // The selector then preferred raw on systems where NM was the
        // only honest answer, and `apply` failed with the milestone-1
        // "not yet implemented" stub.
        //
        // Raw is the last-resort fallback the selector falls through
        // to when no structured manager is present. Pin that meaning
        // at the availability gate: `ip` must exist (otherwise no
        // ethernet path can run), AND no higher-level manager owns
        // the network (otherwise raw's `ip link set` fights NM /
        // networkd and kills the active connection). Hosts where the
        // operator actively chose `--backend raw` still get the raw
        // impl — the selector skips `available()` for explicit names.
        Box::pin(async {
            let ip_present = Path::new("/sbin/ip").exists() || Path::new("/usr/bin/ip").exists();
            ip_present && !nm_is_running() && !networkd_is_running()
        })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async { bail!("backend::raw: list_devices not yet implemented (Milestone 1)") })
    }

    fn list_connections<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Vec<ConnectionRef>>> {
        Box::pin(async {
            bail!("backend::raw: list_connections not yet implemented (Milestone 1)")
        })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
        _mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { bail!("backend::raw: set_cloned_mac not yet implemented (Milestone 1)") })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async { bail!("backend::raw: read_cloned_mac not yet implemented (Milestone 1)") })
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
            bail!("backend::raw: set_dhcp_settings not yet implemented (Milestone 1)")
        })
    }

    fn set_ipv6_settings<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
        _settings: Ipv6NmSettings,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!("backend::raw: set_ipv6_settings not yet implemented (Milestone 1)")
        })
    }

    fn renew_lease<'a>(
        &'a self,
        _device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<RenewOutcome>> {
        Box::pin(async { bail!("backend::raw: renew_lease not yet implemented (Milestone 1)") })
    }

    fn write_anonymous_identity<'a>(
        &'a self,
        _connection: &'a ConnectionRef,
        _value: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            bail!("backend::raw: write_anonymous_identity not yet implemented (Milestone 1)")
        })
    }
}

/// Same runtime signal `commands::status::detect_system` and the NM
/// backend's own `available()` use. Pulled out so the raw backend can
/// honestly defer to NM when it's running.
fn nm_is_running() -> bool {
    Path::new("/run/NetworkManager").exists() || Path::new("/var/run/NetworkManager").exists()
}

/// Mirrors `backend::networkd::available()`'s positive gate: networkd
/// creates `/run/systemd/netif/` only after start. Raw must defer to
/// it when present so we don't fight a structured manager.
fn networkd_is_running() -> bool {
    Path::new("/run/systemd/netif").is_dir()
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

    #[test]
    fn read_connection_methods_yield_none() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let backend = RawBackend::new();
            let cref = ConnectionRef::new("");
            assert!(backend.read_connection_id(&cref).await.unwrap().is_none());
            assert!(backend.read_connection_uuid(&cref).await.unwrap().is_none());
        });
    }

    /// Issue #247: `available()` previously returned `true` whenever
    /// `ip` was installed, which made the selector pick raw on hosts
    /// running NetworkManager — and every raw method then bailed with
    /// the milestone-1 stub error. Pin the new contract: when NM (or
    /// networkd) is running, raw must defer.
    #[test]
    fn available_defers_to_nm_when_nm_is_running() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let backend = RawBackend::new();
        // Compute the expected answer from the same predicates the impl
        // uses so the test isn't tied to which daemon CI happens to run.
        let ip_present = Path::new("/sbin/ip").exists() || Path::new("/usr/bin/ip").exists();
        let expected = ip_present && !super::nm_is_running() && !super::networkd_is_running();
        let actual = rt.block_on(async { backend.available().await });
        assert_eq!(
            actual, expected,
            "raw availability must reflect ip-present && !nm && !networkd"
        );
        // And specifically: if NM is running, the answer is `false`.
        if super::nm_is_running() {
            assert!(
                !actual,
                "raw must defer to NM when NetworkManager is running"
            );
        }
    }
}
