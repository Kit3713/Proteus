// SPDX-License-Identifier: GPL-3.0-or-later

//! `NetworkBackend` — the abstraction that lets Proteus drive
//! NetworkManager, systemd-networkd, or raw `ip` + `iw` from the same
//! call sites.
//!
//! Roadmap Milestone 1 (`docs/ROADMAP.md`): the trait surface lives
//! here; per-command migration off of `crate::nm::*` lands in this PR.
//! NM stays the only fully-wired backend; the `networkd` and `raw`
//! impls compile but every method bails until the dedicated work lands.
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
//! dispatcher stops sed-parsing `proteus current --json`, and
//! `state_lock` migrates to a `Mutex`-protected primitive that's
//! safe under the async event loops the trait will be called from.

pub mod mock;
pub mod networkd;
pub mod nm;
pub mod raw;
pub mod select;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

use crate::ipv6::nm::Ipv6NmSettings;
use crate::mac::Mac;
use crate::state::DhcpSettingsSnapshot;

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
/// (NM device object path string, networkd `.network` filename,
/// `ip` link name, etc.). Treat it as opaque outside the impl.
#[derive(Debug, Clone)]
pub struct BackendDevice {
    pub iface: String,
    pub kind: BackendKind,
    pub hw_address: Option<String>,
    pub identifier: String,
    /// Connection profiles bound to this device, in the backend's
    /// native handle form. NM has multiple profiles per Wi-Fi device
    /// (one per saved SSID); networkd / raw collapse this to one.
    pub connections: Vec<ConnectionRef>,
    /// Whether the backend currently considers the device "managed".
    /// Callers iterating all devices skip unmanaged ones; an explicit
    /// iface filter still lets them through so a "no such device"
    /// error surfaces.
    pub managed: bool,
}

/// Backend-opaque connection handle. The trait surface intentionally
/// hides `zbus::zvariant::OwnedObjectPath` — networkd's stub future
/// will key off a `.network` filename, raw's off an iface name. The
/// NM impl carries the dbus path internally; everything else uses the
/// `String` form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionRef {
    inner: String,
}

impl ConnectionRef {
    /// Construct from any string. Used by the NM backend with the
    /// `OwnedObjectPath::as_str()` view, and by tests / stub backends
    /// with whatever opaque token they've cooked up.
    pub fn new(s: impl Into<String>) -> Self {
        Self { inner: s.into() }
    }

    /// Borrow the opaque payload. Mostly for log lines and tests; the
    /// production callers should not parse this.
    pub fn as_str(&self) -> &str {
        &self.inner
    }
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

/// Typed result for [`NetworkBackend::renew_lease`]. Mirrors the
/// `nm::dhcp::RenewOutcome` enum but stays at the trait layer so
/// commands don't need to import the NM internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewOutcome {
    /// `Device.Reapply` (or its backend equivalent) succeeded — link
    /// stayed up and DHCP cycled.
    Reapplied,
    /// Reapply was rejected; we fell back to a Disconnect +
    /// Activate-style cycle.
    DisconnectActivated,
    /// No active connection on the device — nothing to renew, not an
    /// error.
    NoActiveConnection,
}

/// The trait Proteus's commands route through. Today only
/// [`nm::NmBackend`] implements every method end-to-end; `networkd`
/// and `raw` are scaffolds.
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

    /// Connection profiles bound to `device`. Convenience over
    /// [`BackendDevice::connections`] for callers that want to iterate
    /// without first holding the device value.
    fn list_connections<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Vec<ConnectionRef>>>;

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
    ///
    /// GH#381: `state_path` is the operator-supplied state file (from
    /// `--state`); when `None` the backend defaults to
    /// `crate::commands::DEFAULT_STATE_PATH`. Pre-fix, the NM backend
    /// hardcoded the default path and silently ignored `--state`, so a
    /// dispatcher run with a custom state file recorded the cooldown
    /// stamp on disk but the next `rotate-if-needed` check read from
    /// the default file and rotated again.
    ///
    /// Issue #294: `reason` is the sanitized audit string from the
    /// `--reason` flag. It's passed through to the inner rotate hook
    /// so each rotated iface's state record carries the same value
    /// the dispatcher / operator supplied. `None` keeps the pre-#294
    /// behaviour (no reason stamped). Sanitization (control-byte
    /// strip + 256-byte cap) happens at the CLI layer in
    /// [`crate::commands::rotate::sanitize_reason`] before reaching
    /// the trait, so backend impls treat the input as already-safe.
    fn rotate_if_needed<'a>(
        &'a self,
        iface: &'a str,
        cooldown: Duration,
        state_path: Option<&'a std::path::Path>,
        reason: Option<&'a str>,
    ) -> BoxFuture<'a, Result<RotateOutcome>>;

    /// Read the human-friendly profile id for `connection` (`connection.id`
    /// on NM). `Ok(None)` for backends that don't expose one.
    fn read_connection_id<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>>;

    /// Read the stable uuid for `connection` (`connection.uuid` on NM).
    /// `Ok(None)` for backends that don't expose one — the stub
    /// backends derive a synthetic uuid from the connection handle so
    /// state-keying still works.
    fn read_connection_uuid<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>>;

    /// Push the [`DhcpSettingsSnapshot`] onto `connection`. The
    /// snapshot's shape is the same one the per-command DHCP code
    /// already serialises into `state.json`; the backend translates
    /// each field into its native key path (NM `ipv4.dhcp-*` keys for
    /// nm, `[DHCPv4]` drop-in keys for networkd, etc.).
    fn set_dhcp_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        snapshot: DhcpSettingsSnapshot,
    ) -> BoxFuture<'a, Result<()>>;

    /// Push the IPv6 NM-style settings onto `connection`. Networkd
    /// and raw will translate into their native equivalents.
    fn set_ipv6_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        settings: Ipv6NmSettings,
    ) -> BoxFuture<'a, Result<()>>;

    /// Trigger a DHCP lease renew on `device` without touching the
    /// cloned MAC. Roadmap Milestone 4c.
    fn renew_lease<'a>(&'a self, device: &'a BackendDevice) -> BoxFuture<'a, Result<RenewOutcome>>;

    /// Write `value` into the connection's anonymous outer identity
    /// (`802-1x.anonymous-identity` on NM). An empty string clears
    /// the field per NM's documented contract for "unset on save".
    fn write_anonymous_identity<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        value: &'a str,
    ) -> BoxFuture<'a, Result<()>>;
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
            connections: vec![ConnectionRef::new(
                "/org/freedesktop/NetworkManager/Settings/2",
            )],
            managed: true,
        };
        let cloned = dev.clone();
        assert_eq!(cloned.iface, dev.iface);
        assert_eq!(cloned.kind, dev.kind);
        assert_eq!(cloned.identifier, dev.identifier);
        assert_eq!(cloned.connections, dev.connections);
        assert!(cloned.managed);
    }

    #[test]
    fn connection_ref_round_trips_payload() {
        let r = ConnectionRef::new("/org/freedesktop/NetworkManager/Settings/9");
        assert_eq!(r.as_str(), "/org/freedesktop/NetworkManager/Settings/9");
        let cloned = r.clone();
        assert_eq!(cloned, r);
    }

    #[test]
    fn renew_outcome_variants_are_distinct() {
        assert_ne!(RenewOutcome::Reapplied, RenewOutcome::DisconnectActivated);
        assert_ne!(RenewOutcome::Reapplied, RenewOutcome::NoActiveConnection);
    }
}
