// SPDX-License-Identifier: GPL-3.0-or-later

//! Trigger sources: the bridge between OS-level event streams
//! (NetworkManager `StateChanged`, RTNETLINK link messages, nl80211
//! regulatory multicast, the captive-portal poller) and the in-process
//! [`super::EventRegistry`]. Roadmap Milestone 4c.
//!
//! Each source is split into a production implementation and a mock
//! implementation. Production opens the real DBus / netlink socket
//! (or in the portal case spins a poll task) and gracefully degrades
//! to `Unsupported` when the necessary capability is missing
//! (`CAP_NET_ADMIN` for the netlink sources, system DBus access for
//! NM). Mocks let unit tests inject canned events and assert the
//! registry observed exactly the right `RotationTrigger`s.
//!
//! The five files under `events/` are the four sources plus this
//! coordinator:
//!
//! - `nm_connection_up.rs` — NM Device.StateChanged → `ConnectionUp`
//! - `link_flap.rs`        — RTM_NEWLINK/DELLINK → `LinkFlap`
//! - `reg_domain.rs`       — nl80211 regulatory mc → `RegDomainChange`
//! - `portal_auth.rs`      — captive-portal poller → `PortalAuth`
//!
//! Splitting per-file keeps each under the 600-line ceiling and lets
//! the unit tests live next to the source they cover.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::EventRegistry;

pub mod link_flap;
pub mod nm_connection_up;
pub mod portal_auth;
pub mod reg_domain;

pub use link_flap::{LinkFlapSource, MockLinkFlapSource};
pub use nm_connection_up::{MockNmConnectionUpSource, NmConnectionUpSource};
pub use portal_auth::{
    MockPortalAuthSource, MockPortalSampler, PortalAuthSource, PortalSampler, SystemPortalSampler,
};
pub use reg_domain::{MockRegDomainChangeSource, RegDomainChangeSource};

/// Sources that wrap a long-lived subscription implement this. The
/// trait is deliberately small: `start` synchronously kicks off the
/// subscription, `name` is for log spans.
///
/// The synchronous `start` shape is preserved from the scaffolding PR
/// so existing callers (and the ever-shrinking pool of tests that
/// drive a mock through `start`) keep working. Long-lived sources
/// also expose async [`spawn_into`] which returns a `(JoinHandle,
/// StopHandle)` pair the orchestrator drives from a tokio runtime.
///
/// Implementors should treat `start` as fire-and-forget: it's allowed
/// to spawn its own background tasks, but it must not block the
/// caller. Mocks just push canned events into the registry inline
/// and return immediately; production sources spawn a task and let
/// it run for the lifetime of the process.
pub trait EventSource: Send + Sync {
    /// Stable token for log spans (`"nm-connection-up"`,
    /// `"link-flap"`, `"reg-domain-change"`, `"portal-auth"`).
    fn name(&self) -> &'static str;

    /// Begin subscribing. Mock sources push their queued events
    /// inline; production sources spawn a background task. Either way
    /// the call returns promptly. Errors here are surfaced when the
    /// underlying API rejects the open (e.g. DBus session not
    /// reachable, netlink bind failed because `CAP_NET_ADMIN` is
    /// missing on the running process); the orchestrator logs and
    /// continues so a single failed source does not take down the
    /// rest.
    fn start(&self, registry: &EventRegistry) -> Result<()>;
}

/// Handle returned by [`EventSource::spawn_into`]. Drop or call
/// [`StopHandle::stop`] to ask the source to wind down. The drop
/// behaviour is best-effort — production sources block on a netlink
/// recv that doesn't yield to a `Drop`, so the orchestrator should
/// always call `stop()` explicitly when shutting down cleanly.
pub struct StopHandle {
    sender: Option<oneshot::Sender<()>>,
}

impl StopHandle {
    /// Build a `(StopHandle, oneshot::Receiver<()>)` pair. The
    /// receiver is what the spawned task selects on alongside its
    /// event-stream future.
    pub fn channel() -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self { sender: Some(tx) }, rx)
    }

    /// Signal the spawned task to wind down. Ignores send errors so
    /// double-stop and post-task-exit calls are safe.
    pub fn stop(mut self) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for StopHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(());
        }
    }
}

/// Output of [`spawn_into`]: a `JoinHandle` for the source's task and
/// a `StopHandle` for graceful shutdown. The orchestrator collects
/// one of these per source and awaits them in `proteus events run`'s
/// shutdown path.
pub struct SourceTask {
    pub join: JoinHandle<()>,
    pub stop: StopHandle,
    pub name: &'static str,
}

/// Build the four production sources, instantiated. Convenience for
/// the orchestrator entry point so it can iterate without each
/// caller listing the set by hand.
pub fn all_sources() -> Vec<Box<dyn EventSource>> {
    vec![
        Box::new(NmConnectionUpSource::new()),
        Box::new(LinkFlapSource::new()),
        Box::new(RegDomainChangeSource::new()),
        Box::new(PortalAuthSource::default()),
    ]
}

/// Run a stub trigger through every source. Today every production
/// `start` either spawns a real subscription (when the host has the
/// capability) or no-ops (when it doesn't). Mocks bypass this helper
/// entirely; tests construct the mock and call `.start` directly.
pub fn start_all(registry: &EventRegistry) -> Result<()> {
    for s in all_sources() {
        if let Err(e) = s.start(registry) {
            tracing::warn!(source = s.name(), "source start failed: {e:#}");
        }
    }
    Ok(())
}

/// Spawn every production source into a tokio runtime, registering
/// each one against the given `Arc<EventRegistry>`. Returns one
/// [`SourceTask`] per source; the orchestrator awaits them in
/// shutdown. Sources whose subscription fails (e.g. no `CAP_NET_ADMIN`
/// for the netlink sources) log a warning and contribute no
/// `SourceTask` — the rest still run.
pub async fn spawn_all(registry: Arc<EventRegistry>) -> Vec<SourceTask> {
    let mut out = Vec::new();
    if let Some(t) = NmConnectionUpSource::new()
        .spawn_into(Arc::clone(&registry))
        .await
    {
        out.push(t);
    }
    if let Some(t) = LinkFlapSource::new()
        .spawn_into(Arc::clone(&registry))
        .await
    {
        out.push(t);
    }
    if let Some(t) = RegDomainChangeSource::new()
        .spawn_into(Arc::clone(&registry))
        .await
    {
        out.push(t);
    }
    if let Some(t) = PortalAuthSource::default()
        .spawn_into(Arc::clone(&registry))
        .await
    {
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every source's `name` token is stable — tests pin them so
    /// log scrapers and the eventual `proteus events status` table
    /// keep their column values across patch releases.
    #[test]
    fn source_names_are_stable() {
        assert_eq!(NmConnectionUpSource::new().name(), "nm-connection-up");
        assert_eq!(LinkFlapSource::new().name(), "link-flap");
        assert_eq!(RegDomainChangeSource::new().name(), "reg-domain-change");
        assert_eq!(PortalAuthSource::default().name(), "portal-auth");
    }

    /// `all_sources` returns one of each kind — the count is
    /// load-bearing because the daemon expects four.
    #[test]
    fn all_sources_returns_the_four_kinds() {
        let names: Vec<&'static str> = all_sources().iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"nm-connection-up"));
        assert!(names.contains(&"link-flap"));
        assert!(names.contains(&"reg-domain-change"));
        assert!(names.contains(&"portal-auth"));
    }

    /// Production-source `start` is a graceful no-op when the
    /// underlying API isn't reachable from the test host — never
    /// panic, never register handlers itself, never error so loudly
    /// that the orchestrator falls over. The actual subscription
    /// behaviour is exercised through the mock variants.
    #[test]
    fn production_start_is_a_clean_noop_on_test_host() {
        let reg = EventRegistry::new();
        for source in all_sources() {
            // start() may legitimately error when DBus / netlink isn't
            // accessible; what matters is no panic.
            let _ = source.start(&reg);
        }
        assert_eq!(
            reg.handler_count(),
            0,
            "production start must not register any handlers"
        );
    }

    /// `StopHandle::stop` is idempotent — calling it twice (or after
    /// the receiver has gone away) must not panic.
    #[test]
    fn stop_handle_is_idempotent() {
        let (handle, rx) = StopHandle::channel();
        drop(rx);
        handle.stop();
    }

    /// Drop fires the stop signal too. Important so a panicking
    /// orchestrator task signals winddown without an explicit cleanup.
    #[test]
    fn stop_handle_drop_signals_shutdown() {
        let (handle, mut rx) = StopHandle::channel();
        drop(handle);
        // Receiver should observe a sender-side close (Ok(())).
        assert!(rx.try_recv().is_ok());
    }
}
