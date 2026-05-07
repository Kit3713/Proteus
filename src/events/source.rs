// SPDX-License-Identifier: GPL-3.0-or-later

//! Trigger source stubs. Roadmap Milestone 4c-followup.
//!
//! Each source is the bridge between an OS-level event stream
//! (NetworkManager `StateChanged`, rfkill notifications, `iw event`
//! netlink, the captive-portal poller) and the in-process
//! [`super::EventRegistry`]. This file ships the surface — types,
//! constructors, doc comments — without yet wiring into the live
//! streams. The follow-up PR replaces each `start` with the real
//! subscription.
//!
//! Why scaffold this now: callers in adjacent milestones (persona
//! application, captive portal post-auth rotation, RF surface
//! re-evaluation on regulatory change) need a stable type to register
//! against. Splitting the surface from the wiring lets those callers
//! land in parallel.

use anyhow::Result;

use super::EventRegistry;
// `RotationTrigger` itself is reachable via `super::RotationTrigger` from the
// stub-source pseudocode in the doc comments. Today only the doc comments
// reference it; once the wiring follow-up replaces each `start` body, each
// source will import it directly.

/// Sources that wrap a long-lived subscription implement this. The
/// trait is deliberately small: `start` blocks (or spawns) the
/// subscription, `name` is for log spans.
///
/// The follow-up wiring will introduce a `tokio::sync::mpsc` between
/// each source and a central dispatcher; for now a source just gets
/// a `&EventRegistry` and pushes triggers in synchronously.
pub trait EventSource: Send + Sync {
    /// Stable token for log spans.
    fn name(&self) -> &'static str;

    /// Begin subscribing. Today every implementation is a stub that
    /// returns immediately. Milestone 4c-followup turns this into a
    /// long-lived subscription.
    fn start(&self, registry: &EventRegistry) -> Result<()>;
}

/// Connection-up source. Subscribes to NetworkManager's
/// `org.freedesktop.NetworkManager.Device.StateChanged` signal and
/// emits [`RotationTrigger::ConnectionUp`] for transitions into the
/// `Activated` state (NM device-state integer 100).
///
/// Milestone 4c-followup: replace `start` with a zbus signal stream
/// reader. The pseudocode lives in this doc comment so the wiring PR
/// is a fill-in:
///
/// ```text
/// let nm = NetworkManagerProxy::new(&conn).await?;
/// for path in nm.get_devices().await? {
///     let dev = DeviceProxy::builder(&conn).path(path)?.build().await?;
///     let mut stream = dev.receive_state_changed().await?;
///     tokio::spawn(async move {
///         while let Some(sig) = stream.next().await {
///             let body = sig.args()?; // (new_state, old_state, reason)
///             if body.new_state == 100 {
///                 registry.fire(RotationTrigger::ConnectionUp {
///                     iface: dev.interface().await.unwrap_or_default(),
///                     ssid: read_active_ssid(&dev).await.ok().flatten(),
///                 })?;
///             }
///         }
///     });
/// }
/// ```
pub struct NmConnectionUpSource;

impl NmConnectionUpSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NmConnectionUpSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for NmConnectionUpSource {
    fn name(&self) -> &'static str {
        "nm-connection-up"
    }

    fn start(&self, _registry: &EventRegistry) -> Result<()> {
        // Milestone 4c-followup: subscribe to NM Device.StateChanged.
        Ok(())
    }
}

/// Link-flap source. Watches netlink RTM_NEWLINK / RTM_DELLINK and
/// fires when a single iface flips down→up faster than
/// `flap_window_secs` (config follow-up).
///
/// Milestone 4c-followup: bind an `AF_NETLINK` `NETLINK_ROUTE` socket
/// and run a small ring buffer of recent transitions per iface.
pub struct LinkFlapSource;

impl LinkFlapSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LinkFlapSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for LinkFlapSource {
    fn name(&self) -> &'static str {
        "link-flap"
    }

    fn start(&self, _registry: &EventRegistry) -> Result<()> {
        // Milestone 4c-followup: subscribe to RTM_NEWLINK/DELLINK.
        Ok(())
    }
}

/// Regulatory-domain change source. Subscribes to nl80211 multicast
/// group `regulatory` and fires when the userspace tool (`iw reg
/// set`), the kernel, or hostapd's country-IE handover changes the
/// active domain.
///
/// Milestone 4c-followup: bind a `NETLINK_GENERIC` socket against
/// `nl80211` and resolve the multicast group at runtime.
pub struct RegDomainChangeSource;

impl RegDomainChangeSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RegDomainChangeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for RegDomainChangeSource {
    fn name(&self) -> &'static str {
        "reg-domain-change"
    }

    fn start(&self, _registry: &EventRegistry) -> Result<()> {
        // Milestone 4c-followup: subscribe to nl80211 regulatory group.
        Ok(())
    }
}

/// Captive-portal auth-completion source. Watches
/// [`crate::captive_portal`] state transitions for the
/// `Required → Authed` edge and fires
/// [`RotationTrigger::PortalAuth`] with the SSID currently associated
/// on the relevant interface.
///
/// Milestone 4c-followup: portal module exposes a `Watcher` handle
/// that this source subscribes to; the watcher already exists in
/// stub form.
pub struct PortalAuthSource;

impl PortalAuthSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PortalAuthSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for PortalAuthSource {
    fn name(&self) -> &'static str {
        "portal-auth"
    }

    fn start(&self, _registry: &EventRegistry) -> Result<()> {
        // Milestone 4c-followup: subscribe to portal state changes.
        Ok(())
    }
}

/// All four sources, instantiated. Convenience for the (eventual)
/// daemon entry point so it can iterate without each caller listing
/// the set by hand.
pub fn all_sources() -> Vec<Box<dyn EventSource>> {
    vec![
        Box::new(NmConnectionUpSource::new()),
        Box::new(LinkFlapSource::new()),
        Box::new(RegDomainChangeSource::new()),
        Box::new(PortalAuthSource::new()),
    ]
}

/// Run a stub trigger through every source. Today every `start` is a
/// no-op; this helper exists so the wiring PR has a single integration
/// point to land against without duplicating the iteration loop.
pub fn start_all(registry: &EventRegistry) -> Result<()> {
    for s in all_sources() {
        if let Err(e) = s.start(registry) {
            tracing::warn!(source = s.name(), "source start failed: {e:#}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every source's `name` token is stable — tests pin them so
    /// log scrapers and the (eventual) `proteus events status` table
    /// keep their column values across patch releases.
    #[test]
    fn source_names_are_stable() {
        assert_eq!(NmConnectionUpSource::new().name(), "nm-connection-up");
        assert_eq!(LinkFlapSource::new().name(), "link-flap");
        assert_eq!(RegDomainChangeSource::new().name(), "reg-domain-change");
        assert_eq!(PortalAuthSource::new().name(), "portal-auth");
    }

    /// Stub `start` must succeed without doing anything observable.
    /// Once the follow-up wires the real subscriptions, this test
    /// becomes a smoke check that construction doesn't panic.
    #[test]
    fn stub_start_is_a_clean_noop() {
        let reg = EventRegistry::new();
        for source in all_sources() {
            source.start(&reg).expect("stub start must not error");
        }
        assert_eq!(
            reg.handler_count(),
            0,
            "stub start must not register any handlers"
        );
    }

    /// `all_sources` returns one of each kind — the count is
    /// load-bearing because the (eventual) daemon expects four.
    #[test]
    fn all_sources_returns_the_four_kinds() {
        let names: Vec<&'static str> = all_sources().iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"nm-connection-up"));
        assert!(names.contains(&"link-flap"));
        assert!(names.contains(&"reg-domain-change"));
        assert!(names.contains(&"portal-auth"));
    }
}
