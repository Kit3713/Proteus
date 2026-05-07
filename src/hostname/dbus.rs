// SPDX-License-Identifier: GPL-3.0-or-later

//! Thin wrapper over `org.freedesktop.hostname1`. Proteus never writes
//! `/etc/hostname`, `/etc/machine-info`, or `/proc/sys/kernel/hostname`
//! directly — systemd's hostnamed owns those files and any direct write
//! invites the daemon to re-write what we just set.

use anyhow::{Context, Result};
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.hostname1",
    default_service = "org.freedesktop.hostname1",
    default_path = "/org/freedesktop/hostname1",
    gen_blocking = false
)]
pub trait Hostname1 {
    /// Kernel hostname; persists across reboots (`/etc/hostname`).
    #[zbus(property)]
    fn hostname(&self) -> zbus::Result<String>;

    /// Same as `hostname` but explicitly the static one. Older hostnamed
    /// versions only expose this via the property — the method below changes
    /// it.
    #[zbus(property)]
    fn static_hostname(&self) -> zbus::Result<String>;

    /// Pretty hostname (`/etc/machine-info` PRETTY_HOSTNAME).
    #[zbus(property)]
    fn pretty_hostname(&self) -> zbus::Result<String>;

    /// Set the kernel hostname (writes /etc/hostname). `interactive=false`
    /// disables polkit interactive prompts; we always pass false.
    fn set_static_hostname(&self, name: &str, interactive: bool) -> zbus::Result<()>;

    /// Set the pretty hostname (writes /etc/machine-info).
    fn set_pretty_hostname(&self, name: &str, interactive: bool) -> zbus::Result<()>;

    /// Set the transient (kernel runtime) hostname. Reverts at reboot.
    fn set_hostname(&self, name: &str, interactive: bool) -> zbus::Result<()>;
}

/// Snapshot of all three hostnamed-tracked names, captured before mutation.
#[derive(Debug, Clone, Default)]
pub struct HostnameSnapshot {
    pub static_name: Option<String>,
    pub pretty_name: Option<String>,
    pub transient_name: Option<String>,
}

/// Connect to the system bus and create a hostname1 proxy. Errors are passed
/// through with context so callers can present a clean failure message rather
/// than a bare zbus error.
pub async fn proxy() -> Result<Hostname1Proxy<'static>> {
    let conn = zbus::Connection::system()
        .await
        .context("connecting to system DBus (hostnamed required)")?;
    Hostname1Proxy::new(&conn)
        .await
        .context("constructing hostname1 proxy")
}

/// Read all three hostname fields. Reads are best-effort — any individual
/// field that errors comes back as `None` rather than poisoning the whole
/// snapshot, because `Hostname` (transient) in particular is unset on many
/// systems and reading it returns an empty string we can't distinguish from
/// "actually empty".
pub async fn read_snapshot(p: &Hostname1Proxy<'_>) -> HostnameSnapshot {
    HostnameSnapshot {
        static_name: p.static_hostname().await.ok().filter(|s| !s.is_empty()),
        pretty_name: p.pretty_hostname().await.ok().filter(|s| !s.is_empty()),
        transient_name: p.hostname().await.ok().filter(|s| !s.is_empty()),
    }
}
