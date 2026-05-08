// SPDX-License-Identifier: GPL-3.0-or-later

//! Captive-portal auth-completion source. Polls a captive-portal
//! [`PortalSampler`] every `portal_poll_secs` seconds; when the
//! classification flips from `PortalRequired` to `Clear` /
//! `PortalAuthed` it emits
//! [`super::super::RotationTrigger::PortalAuth`] with the SSID
//! currently associated on the relevant interface. Roadmap
//! Milestone 4c.
//!
//! Composition: the source is content-agnostic about how the portal
//! state is sampled. Production wires
//! [`crate::captive_portal::detect`] in via the
//! [`SystemPortalSampler`] adapter (private to this file). Tests
//! inject a `MockPortalSampler` that returns canned classifications
//! in sequence; combined with [`MockPortalAuthSource`], that's all
//! the harness a unit test needs to drive the trigger.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::captive_portal::Classification;

use super::super::{EventRegistry, RotationTrigger};
use super::{EventSource, SourceTask, StopHandle};

/// Default poll cadence — `[events] portal_poll_secs` overrides.
pub const DEFAULT_POLL_SECS: u64 = 30;

/// Pluggable portal classifier. Production samples the live
/// `crate::captive_portal::detect`; tests inject a queue of canned
/// classifications via [`MockPortalSampler`].
pub trait PortalSampler: Send + Sync {
    /// Return the current portal classification. Implementations
    /// must complete promptly — the source polls on a fixed cadence
    /// and a sampler that blocks longer than the cadence will skew
    /// the trigger latency.
    fn sample(&self) -> Classification;

    /// Best-effort SSID for the current connection. Returned with
    /// the trigger so handlers can decide whether to rotate. `None`
    /// is permitted; production fills this from
    /// `crate::captive_portal::current_ssid` (a follow-up — for now
    /// the field is `None` until the portal module exposes that
    /// helper).
    fn current_ssid(&self) -> Option<String> {
        None
    }
}

/// Production captive-portal sampler. Reads the configured detect
/// URL with a 5 s timeout and reports the resulting classification.
/// This adapter is intentionally lightweight — every poll re-runs
/// the detector — because the source's poll cadence (30 s default)
/// is the rate limiter.
pub struct SystemPortalSampler {
    pub detect_url: String,
    pub expected_response: String,
    pub timeout_secs: u64,
}

impl SystemPortalSampler {
    pub fn new(detect_url: String, expected_response: String, timeout_secs: u64) -> Self {
        Self {
            detect_url,
            expected_response,
            timeout_secs,
        }
    }
}

impl PortalSampler for SystemPortalSampler {
    fn sample(&self) -> Classification {
        crate::captive_portal::detect(
            &self.detect_url,
            &self.expected_response,
            std::time::Duration::from_secs(self.timeout_secs),
        )
        .classification
    }
}

/// Production portal-auth source. Holds the sampler + cadence; the
/// long-lived poll task is created in `spawn_into`.
pub struct PortalAuthSource {
    sampler: Arc<dyn PortalSampler>,
    poll_secs: u64,
}

impl PortalAuthSource {
    /// Build with an explicit sampler + cadence. Used by the
    /// orchestrator to wire `[events] portal_poll_secs` and the
    /// configured detector endpoint.
    pub fn new(sampler: Arc<dyn PortalSampler>, poll_secs: u64) -> Self {
        Self { sampler, poll_secs }
    }

    /// Spawn the poll task. Always returns `Some` — the captive-
    /// portal poller has no privilege requirement and the poll loop
    /// itself is the reactive surface.
    pub async fn spawn_into(self, registry: Arc<EventRegistry>) -> Option<SourceTask> {
        use std::time::Duration;
        let (stop, mut stop_rx) = StopHandle::channel();
        let sampler = Arc::clone(&self.sampler);
        let poll = Duration::from_secs(self.poll_secs.max(1));
        let join = tokio::spawn(async move {
            let mut prev: Option<Classification> = None;
            loop {
                let next = sampler.sample();
                if is_auth_edge(prev, next) {
                    let ssid = sampler.current_ssid().unwrap_or_default();
                    let _ = registry.fire(RotationTrigger::PortalAuth { ssid });
                }
                prev = Some(next);
                // Issue #233: race the stop signal against the poll
                // cadence so shutdown wins. Previously the loop slept
                // for a full `poll` interval before checking
                // `stop_rx.try_recv()`, costing up to `poll_secs`
                // shutdown latency on the only polling source. The
                // other three sources (netlink/dbus) respond in
                // milliseconds; the portal poll was the long pole.
                //
                // `tokio::time::timeout` is available without the
                // `tokio/macros` feature (we deliberately don't pull
                // `select!`). `oneshot::Receiver` is `Unpin`, so a
                // `&mut Receiver<()>` is usable directly as a Future.
                match tokio::time::timeout(poll, &mut stop_rx).await {
                    Ok(_) => break,     // stop signaled (or sender dropped)
                    Err(_) => continue, // poll cadence elapsed
                }
            }
        });
        Some(SourceTask {
            join,
            stop,
            name: "portal-auth",
        })
    }

    /// Stable accessor for the configured poll cadence. Pin so
    /// config-wiring tests can assert `[events] portal_poll_secs`
    /// without reaching into private fields.
    pub fn poll_secs(&self) -> u64 {
        self.poll_secs
    }
}

impl Default for PortalAuthSource {
    fn default() -> Self {
        // The default sampler returns `Unknown` forever — production
        // builds always replace this via the orchestrator. The
        // default exists so `all_sources()` and the synchronous
        // `start` shape compose cleanly without a sampler.
        struct AlwaysUnknown;
        impl PortalSampler for AlwaysUnknown {
            fn sample(&self) -> Classification {
                Classification::Unknown
            }
        }
        Self {
            sampler: Arc::new(AlwaysUnknown),
            poll_secs: DEFAULT_POLL_SECS,
        }
    }
}

impl EventSource for PortalAuthSource {
    fn name(&self) -> &'static str {
        "portal-auth"
    }

    fn start(&self, _registry: &EventRegistry) -> Result<()> {
        // Synchronous start is a no-op: the poll loop needs a tokio
        // runtime, which the orchestrator provides via `spawn_into`.
        Ok(())
    }
}

/// True when the (`prev`, `next`) classification pair is the
/// `PortalRequired → Clear|PortalAuthed` edge. Pulled out so the
/// detection logic is unit-testable without standing up the full
/// poll loop.
fn is_auth_edge(prev: Option<Classification>, next: Classification) -> bool {
    matches!(prev, Some(Classification::PortalRequired))
        && matches!(next, Classification::Clear | Classification::PortalAuthed)
}

/// Test sampler — returns canned classifications in sequence. Tests
/// queue a script of classifications via [`MockPortalSampler::push`]
/// and pass the sampler to [`MockPortalAuthSource`].
pub struct MockPortalSampler {
    queue: Mutex<Vec<Classification>>,
    ssid: Mutex<Option<String>>,
}

impl MockPortalSampler {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            ssid: Mutex::new(None),
        }
    }

    pub fn push(&self, c: Classification) {
        self.queue.lock().unwrap().push(c);
    }

    pub fn set_ssid(&self, ssid: impl Into<String>) {
        *self.ssid.lock().unwrap() = Some(ssid.into());
    }
}

impl Default for MockPortalSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalSampler for MockPortalSampler {
    fn sample(&self) -> Classification {
        let mut q = self.queue.lock().unwrap();
        if q.is_empty() {
            Classification::Unknown
        } else {
            q.remove(0)
        }
    }

    fn current_ssid(&self) -> Option<String> {
        self.ssid.lock().unwrap().clone()
    }
}

/// Test source. Drives a [`MockPortalSampler`] through one drain of
/// the queued classifications and fires
/// `RotationTrigger::PortalAuth` on every detected auth edge.
///
/// The source borrows the sampler so a single sampler can outlive
/// multiple `start()` calls — useful for tests that want to assert
/// dedup behaviour across drains.
pub struct MockPortalAuthSource {
    sampler: Arc<MockPortalSampler>,
}

impl MockPortalAuthSource {
    pub fn new(sampler: Arc<MockPortalSampler>) -> Self {
        Self { sampler }
    }
}

impl EventSource for MockPortalAuthSource {
    fn name(&self) -> &'static str {
        "portal-auth"
    }

    fn start(&self, registry: &EventRegistry) -> Result<()> {
        let drain: Vec<Classification> = std::mem::take(&mut *self.sampler.queue.lock().unwrap());
        let mut prev: Option<Classification> = None;
        for next in drain {
            if is_auth_edge(prev, next) {
                let ssid = self.sampler.current_ssid().unwrap_or_default();
                registry.fire(RotationTrigger::PortalAuth { ssid })?;
            }
            prev = Some(next);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::super::EventHandler;
    use super::*;

    struct Counter {
        n: Arc<AtomicUsize>,
        last: Arc<Mutex<Option<RotationTrigger>>>,
    }

    impl EventHandler for Counter {
        fn handle(&self, t: &RotationTrigger) -> Result<()> {
            self.n.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(t.clone());
            Ok(())
        }
    }

    fn rig() -> (
        EventRegistry,
        Arc<AtomicUsize>,
        Arc<Mutex<Option<RotationTrigger>>>,
    ) {
        let n = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(Counter {
            n: Arc::clone(&n),
            last: Arc::clone(&last),
        }));
        (reg, n, last)
    }

    /// Headline acceptance: `PortalRequired → Clear` fires one
    /// `PortalAuth` carrying the configured SSID.
    #[test]
    fn auth_edge_required_to_clear_fires_one_event() {
        let (reg, n, last) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::PortalRequired);
        s.push(Classification::Clear);
        s.set_ssid("Cafe");
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        match last.lock().unwrap().as_ref().unwrap() {
            RotationTrigger::PortalAuth { ssid } => assert_eq!(ssid, "Cafe"),
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    /// `PortalRequired → PortalAuthed` is also an auth edge.
    #[test]
    fn auth_edge_required_to_authed_fires_one_event() {
        let (reg, n, _) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::PortalRequired);
        s.push(Classification::PortalAuthed);
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    /// Steady-state `Clear → Clear` does not fire.
    #[test]
    fn clear_to_clear_does_not_fire() {
        let (reg, n, _) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::Clear);
        s.push(Classification::Clear);
        s.push(Classification::Clear);
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Falling back into `PortalRequired` from `Clear` does not fire
    /// — the trigger is the auth completion, not the regression.
    #[test]
    fn clear_to_required_does_not_fire() {
        let (reg, n, _) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::Clear);
        s.push(Classification::PortalRequired);
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// `Unknown` (TCP failure / DNS timeout) doesn't fire — wrapping
    /// it in either direction is just noise.
    #[test]
    fn unknown_classification_does_not_fire() {
        let (reg, n, _) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::PortalRequired);
        s.push(Classification::Unknown);
        s.push(Classification::Clear);
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        // Required→Unknown is not an auth edge; Unknown→Clear is not
        // either (prev needs to be Required). Net: zero triggers.
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Multiple auth flips inside one drain each fire once.
    #[test]
    fn back_to_back_auth_edges_each_fire() {
        let (reg, n, _) = rig();
        let s = Arc::new(MockPortalSampler::new());
        s.push(Classification::PortalRequired);
        s.push(Classification::Clear);
        s.push(Classification::PortalRequired);
        s.push(Classification::PortalAuthed);
        let src = MockPortalAuthSource::new(Arc::clone(&s));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    /// `is_auth_edge` is the load-bearing predicate. Pin every input
    /// pair so a future state-machine refactor can't silently
    /// regress the trigger contract.
    #[test]
    fn is_auth_edge_truth_table() {
        use Classification as C;
        // True: Required → Clear / Required → PortalAuthed.
        assert!(is_auth_edge(Some(C::PortalRequired), C::Clear));
        assert!(is_auth_edge(Some(C::PortalRequired), C::PortalAuthed));
        // False: prev not Required.
        assert!(!is_auth_edge(None, C::Clear));
        assert!(!is_auth_edge(Some(C::Clear), C::Clear));
        assert!(!is_auth_edge(Some(C::PortalAuthed), C::Clear));
        assert!(!is_auth_edge(Some(C::Unknown), C::Clear));
        // False: next not the auth side.
        assert!(!is_auth_edge(Some(C::PortalRequired), C::PortalRequired));
        assert!(!is_auth_edge(Some(C::PortalRequired), C::Unknown));
    }

    /// `poll_secs` round-trips through the constructor.
    #[test]
    fn poll_secs_round_trips() {
        let s = Arc::new(MockPortalSampler::new());
        let src = PortalAuthSource::new(s, 7);
        assert_eq!(src.poll_secs(), 7);
    }

    #[test]
    fn name_is_stable() {
        let s = Arc::new(MockPortalSampler::new());
        assert_eq!(PortalAuthSource::new(s, 30).name(), "portal-auth");
        assert_eq!(
            MockPortalAuthSource::new(Arc::new(MockPortalSampler::new())).name(),
            "portal-auth"
        );
    }
}
