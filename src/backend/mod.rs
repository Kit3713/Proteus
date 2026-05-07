// SPDX-License-Identifier: GPL-3.0-or-later

//! `NetworkBackend` — the abstraction that lets Proteus drive
//! NetworkManager, systemd-networkd, or raw `ip` + `iw` from the same
//! call sites.
//!
//! Roadmap Milestone 1 (`docs/ROADMAP.md`): the trait + scaffolding
//! land here; the per-command migration off of `crate::nm::*` is the
//! follow-up. NM stays the only fully-wired backend in this PR; the
//! `networkd` and `raw` impls compile but every method bails until
//! the dedicated work lands.
//!
//! Why a boxed-future trait instead of `#[async_trait]`: this codebase
//! deliberately keeps its direct dep list short (see `Cargo.toml`),
//! and `async-trait` only appears as a transitive of zbus. The
//! `Pin<Box<dyn Future>>` shape costs one allocation per call — fine
//! at the cardinality the backend trait runs at (a handful per
//! `apply` / `rotate`) — and avoids the proc-macro pull.
//!
//! Also see Milestone 1 issues #206-B and #206-C: the trait's
//! `rotate_if_needed` returns a typed [`RotateOutcome`] so the NM
//! dispatcher stops sed-parsing `proteus current --json`.

pub mod nm;
pub mod networkd;
pub mod raw;
pub mod select;

#[cfg(test)]
pub mod mock;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

use crate::mac::Mac;

pub use select::select;

/// Future returned by every async trait method. See module docs for
/// the rationale for the boxed-future shape over `#[async_trait]`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Mirrors `crate::nm::DeviceKind` but without the NM-specific integer
/// payload. The trait is meant to be called by code that should not
/// care which backend is underneath.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Wifi,
    Ethernet,
    Other,
}

/// Backend-agnostic device handle. `identifier` is whatever the
/// backend uses to address this device when issuing follow-up calls
/// (NM connection object path string, networkd `.network` filename,
/// `ip` link name, etc.). Treat it as opaque outside the impl.
#[derive(Debug, Clone)]
pub struct BackendDevice {
    pub iface: String,
    pub kind: BackendKind,
    pub hw_address: Option<String>,
    pub identifier: String,
}

/// Typed result for [`NetworkBackend::rotate_if_needed`]. Issue
/// #206-C: replaces the dispatcher's `proteus current --json | sed`
/// sniff with a structured value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotateOutcome {
    Rotated { new_mac: Mac },
    SkippedCooldown { remaining: Duration },
    NoFactoryMac,
    BackendUnavailable,
}

/// The trait Proteus's commands will route through once Milestone 1's
/// migration completes. Today only [`nm::NmBackend`] implements every
/// method end-to-end; `networkd` and `raw` are scaffolds.
pub trait NetworkBackend: Send + Sync {
    /// Stable token used in logs, config (`[backend] driver = "..."`),
    /// and `proteus doctor`. Must be one of `"nm"`, `"networkd"`,
    /// `"raw"`, or `"mock"` so the doctor matrix prints predictably.
    fn name(&self) -> &'static str;

    /// Whether this backend is usable on the current host. Cheap
    /// runtime probe — never panics, never mutates. The selector uses
    /// this for `driver = "auto"` resolution.
    fn available<'a>(&'a self) -> BoxFuture<'a, bool>;

    /// Enumerate every interface the backend can manage.
    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>>;

    /// Write `mac` as the cloned/spoofed MAC for `device`. The
    /// concrete write path varies per backend (NM `Settings.Connection.Update`,
    /// networkd drop-in, `ip link set`).
    fn set_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
        mac: Mac,
    ) -> BoxFuture<'a, Result<()>>;

    /// Read whatever the backend currently has set as the cloned MAC
    /// for `device`. Returns `Ok(None)` when the backend has no
    /// cloned-MAC concept for this device kind (e.g. NM's
    /// `802-3-ethernet` vs an unrecognized kind).
    fn read_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>>;

    /// Burned-in factory MAC. All backends defer to
    /// [`crate::mac::factory::permanent_address`] — this method
    /// exists on the trait so callers don't need to reach past the
    /// abstraction.
    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>>;

    /// Rotation entry point used by the NM dispatcher (issue
    /// #206-C). Backends that don't yet implement this return
    /// [`RotateOutcome::BackendUnavailable`].
    fn rotate_if_needed<'a>(
        &'a self,
        iface: &'a str,
        cooldown: Duration,
    ) -> BoxFuture<'a, Result<RotateOutcome>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_outcome_equality() {
        let mac: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        assert_eq!(
            RotateOutcome::Rotated { new_mac: mac },
            RotateOutcome::Rotated { new_mac: mac },
        );
        assert_ne!(
            RotateOutcome::Rotated { new_mac: mac },
            RotateOutcome::NoFactoryMac,
        );
    }

    #[test]
    fn backend_device_clone_round_trip() {
        let dev = BackendDevice {
            iface: "wlan0".into(),
            kind: BackendKind::Wifi,
            hw_address: Some("aa:bb:cc:dd:ee:ff".into()),
            identifier: "/org/freedesktop/NetworkManager/Devices/3".into(),
        };
        let cloned = dev.clone();
        assert_eq!(cloned.iface, dev.iface);
        assert_eq!(cloned.kind, dev.kind);
        assert_eq!(cloned.identifier, dev.identifier);
    }
}
