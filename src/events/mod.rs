// SPDX-License-Identifier: GPL-3.0-or-later

//! Event-driven rotation framework. Roadmap Milestone 4c.
//!
//! Today's `rotate` / `apply` paths are explicit: an operator runs the
//! command (or a systemd timer fires it) and Proteus does the work.
//! Milestone 4c folds the wiki's `phase-c/event-driven-triggers` and
//! `phase-c/auto-triggers` rescue branches back into the tree by
//! exposing four reactive trigger sources:
//!
//! - **Connection-up** — Wi-Fi or Ethernet just came up. Rotate before
//!   the supplicant authenticates if config says so.
//! - **Link-flap** — link toggled in under N seconds. Probably a roam
//!   or a captive-portal redirect. Re-evaluate.
//! - **Regulatory-domain change** — `iw reg set` or hostapd handover
//!   changed the operating band. Re-evaluate RF surface.
//! - **Captive-portal auth completion** — portal just authed; the
//!   AP now sees the long-lived session. Time to swap MAC again.
//!
//! ## What this module ships
//!
//! - The [`RotationTrigger`] payload enum that every source emits.
//! - The [`EventHandler`] trait + [`EventRegistry`] that callers use to
//!   subscribe and dispatch.
//! - Per-source modules under [`source`] that wrap their OS-level
//!   subscriptions and emit triggers into the registry. Each source
//!   ships in two flavours: production (real DBus/netlink, gracefully
//!   degrading to `Unsupported` when capabilities are missing) and a
//!   mock variant tests use to inject canned events.
//! - The `proteus events run` subcommand (under `crate::commands::events`)
//!   that builds an [`EventRegistry`], registers a default rotation
//!   handler against [`crate::commands::rotate::run_with_backend`], and
//!   starts every available source.
//!
//! ## Compatibility with the dispatcher script
//!
//! The NM dispatcher under `dist/networkmanager/dispatcher.d/01-proteus`
//! keeps its existing role: per-event `proteus rotate-if-needed` calls
//! triggered directly by NM. The long-lived `proteus events run` daemon
//! is opt-in via a separate systemd unit (`dist/systemd/proteus-events.service`)
//! and is gated by `[events] enabled`. Operators on systems with the
//! dispatcher in place don't need both — the daemon is for distros
//! without the NM dispatcher (networkd, raw) and for triggers the
//! dispatcher doesn't expose (link-flap, reg-domain, portal-auth).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

pub mod rate_limit;
pub mod source;

pub use rate_limit::{Decision, RateLimiter};

/// The four reactive triggers the framework supports. Each variant
/// carries enough payload for a handler to make a routing decision
/// without consulting the network state separately.
///
/// Variant ordering is stable — handlers may switch on it. Adding a
/// fifth variant in a future milestone is a non-breaking change as
/// long as existing variants keep their shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationTrigger {
    /// A connection just came up. `iface` is the interface name; `ssid`
    /// is the Wi-Fi SSID when applicable, `None` for ethernet or for
    /// pre-association events.
    ConnectionUp { iface: String, ssid: Option<String> },
    /// The link toggled down→up→down→up within the configured flap
    /// window. Likely a roam, a captive-portal interception, or a
    /// flaky AP.
    LinkFlap { iface: String },
    /// The 802.11 regulatory domain changed (`iw reg set`, country IE
    /// change at hostapd, or a USB-rfkill cycle). RF surface and
    /// allowed channels may have shifted.
    RegDomainChange { from: String, to: String },
    /// A captive portal just finished auth. The AP now sees a
    /// long-lived session against the current MAC; downstream code may
    /// want to rotate before that becomes a fingerprint.
    PortalAuth { ssid: String },
}

impl RotationTrigger {
    /// Stable, human-readable token — `"connection-up"`,
    /// `"link-flap"`, etc. Used by tracing spans and the (eventual)
    /// `proteus events --json` log surface.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ConnectionUp { .. } => "connection-up",
            Self::LinkFlap { .. } => "link-flap",
            Self::RegDomainChange { .. } => "reg-domain-change",
            Self::PortalAuth { .. } => "portal-auth",
        }
    }
}

/// Implement this on anything that wants to react to triggers. The
/// trait is intentionally minimal: one method, owned trigger
/// reference, anyhow-shaped error. Handlers should be cheap to call —
/// the registry runs them serially in registration order.
pub trait EventHandler: Send + Sync {
    /// Called once per fired trigger. Returning `Err` is logged; it
    /// does not abort dispatch to later-registered handlers.
    fn handle(&self, trigger: &RotationTrigger) -> Result<()>;
}

/// Holds the registered handlers and routes triggers to them.
///
/// The registry is `Send + Sync` so source tasks can fire triggers
/// from whatever runtime they live on. Handlers are invoked serially
/// in registration order; if a handler does long work it should spawn
/// its own task.
///
/// Callers that want to share one registry across multiple source
/// tasks wrap it in an [`Arc`] and clone the handle into each task
/// — the inner `Mutex` makes that safe. The `&self` shape on `register`
/// is intentional: the orchestrator pushes every handler before it
/// starts a source, so contention on the registration mutex is a
/// non-issue in practice and the `Arc<EventRegistry>` story stays
/// simple.
pub struct EventRegistry {
    handlers: Mutex<Vec<Box<dyn EventHandler>>>,
    /// Issue #254: per-kind rate limiter. Default budget (10/kind/60s)
    /// is plenty for a real burst (NM emits up to 3 connection-up
    /// events in a tight window when reactivating a profile) but
    /// catches a runaway source. Test rigs override via
    /// [`EventRegistry::with_limiter`].
    limiter: RateLimiter,
}

impl EventRegistry {
    /// Build an empty registry with the default rate-limit budget.
    /// Followed by zero or more `register` calls before the source
    /// loop starts firing.
    pub fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
            limiter: RateLimiter::new(),
        }
    }

    /// Build a registry with an explicit rate limiter — used by
    /// tests that need to assert dispatch is dropped at the Nth
    /// trigger or by an orchestrator that wants to override the
    /// default cap. Production wiring uses [`EventRegistry::new`].
    pub fn with_limiter(limiter: RateLimiter) -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
            limiter,
        }
    }

    /// Build an `Arc<EventRegistry>` for the orchestrator. Convenience
    /// — every long-lived source path needs a clonable handle, and
    /// `Arc::new(EventRegistry::new())` shows up at every call site.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Add a handler. Order of registration is the order of dispatch
    /// — register more-important handlers first if a later one might
    /// short-circuit (the registry doesn't propagate that today, but
    /// the convention is worth establishing).
    ///
    /// `&self` (not `&mut self`) so callers can register against an
    /// `Arc<EventRegistry>` without juggling `Arc::get_mut`. The inner
    /// `Mutex` serialises pushes; in practice every register call
    /// happens before the first source starts, so contention is nil.
    ///
    /// Issue #252: when the inner mutex is poisoned (an earlier
    /// handler panicked while holding the guard), recover via
    /// `into_inner()` and log a warning instead of silently dropping
    /// the new handler. Returning `Err` rather than swallowing keeps
    /// the orchestrator's "every handler is wired" invariant honest.
    pub fn register(&self, handler: Box<dyn EventHandler>) -> Result<()> {
        match self.handlers.lock() {
            Ok(mut h) => {
                h.push(handler);
                Ok(())
            }
            Err(poisoned) => {
                tracing::warn!("registry mutex was poisoned; recovered");
                let mut h = poisoned.into_inner();
                h.push(handler);
                Ok(())
            }
        }
    }

    /// Number of registered handlers. Mostly for tests; the live
    /// dispatch path doesn't care.
    ///
    /// Issue #252 mirror: a poisoned inner mutex is recovered via
    /// `into_inner` so this read stays useful for diagnostics even
    /// after a handler panicked.
    pub fn handler_count(&self) -> usize {
        match self.handlers.lock() {
            Ok(h) => h.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// Fire a trigger to every registered handler in registration
    /// order. Each handler's error is logged via `tracing::warn` and
    /// then suppressed — one failing handler must not silence the
    /// rest.
    ///
    /// Issue #254: every fire goes through the per-kind rate limiter
    /// first. If the kind is over budget for the current window we
    /// drop the trigger silently and bump a counter; one warn line
    /// fires per overflow streak and is then coalesced for the
    /// remainder of the window. This protects journald and
    /// downstream rotation paths from a flapping source (a NIC that
    /// down→up→down→ups every 200 ms). Returning `Ok(())` when
    /// rate-limited is intentional: the source caller treats a fired
    /// trigger as a fire-and-forget signal.
    ///
    /// Issue #252: a poisoned inner mutex is recovered via
    /// `into_inner` rather than aborting dispatch. A handler panic
    /// must not silently disable the registry for the rest of the
    /// daemon's life — it would silently swallow every subsequent
    /// rotation trigger.
    pub fn fire(&self, trigger: RotationTrigger) -> Result<()> {
        let kind = trigger.kind();
        let now = Instant::now();
        if let Decision::RateLimited(consecutive) = self.limiter.check_and_record(kind, now) {
            if self.limiter.note_overflow(kind, now).is_some() {
                tracing::warn!(
                    kind = kind,
                    consecutive_drops = consecutive,
                    "event-trigger rate-limit exceeded; dropping further \
                     triggers of this kind for the current window"
                );
            }
            return Ok(());
        }
        let handlers = match self.handlers.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("registry mutex was poisoned; recovered");
                poisoned.into_inner()
            }
        };
        for (idx, h) in handlers.iter().enumerate() {
            if let Err(e) = h.handle(&trigger) {
                tracing::warn!(handler_index = idx, kind = kind, "handler error: {e:#}");
            }
        }
        Ok(())
    }

    /// Inspection helper — exposes the limiter so an orchestrator
    /// (or tests) can query the current per-kind count or override
    /// the budget at runtime. Read-only access via shared reference.
    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }
}

impl Default for EventRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Test handler: counts invocations and remembers the most recent
    /// trigger so tests can assert dispatch shape.
    struct CountingHandler {
        count: Arc<AtomicUsize>,
        last_kind: Arc<Mutex<Option<&'static str>>>,
    }

    impl EventHandler for CountingHandler {
        fn handle(&self, trigger: &RotationTrigger) -> Result<()> {
            self.count.fetch_add(1, Ordering::SeqCst);
            *self.last_kind.lock().unwrap() = Some(trigger.kind());
            Ok(())
        }
    }

    /// Roadmap 4c — registry round trip. Register a handler, fire a
    /// trigger, observe that the handler ran exactly once and saw the
    /// right kind.
    #[test]
    fn registry_dispatches_to_registered_handler() {
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();
        assert_eq!(reg.handler_count(), 1);
        reg.fire(RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: Some("home".into()),
        })
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last.lock().unwrap(), Some("connection-up"));
    }

    /// Multiple handlers run in registration order; firing twice
    /// double-invokes each one.
    #[test]
    fn registry_dispatches_to_all_handlers_in_order() {
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        let last1 = Arc::new(Mutex::new(None));
        let last2 = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&c1),
            last_kind: Arc::clone(&last1),
        }))
        .unwrap();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&c2),
            last_kind: Arc::clone(&last2),
        }))
        .unwrap();
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        reg.fire(RotationTrigger::PortalAuth {
            ssid: "Cafe".into(),
        })
        .unwrap();
        assert_eq!(c1.load(Ordering::SeqCst), 2);
        assert_eq!(c2.load(Ordering::SeqCst), 2);
        assert_eq!(*last1.lock().unwrap(), Some("portal-auth"));
        assert_eq!(*last2.lock().unwrap(), Some("portal-auth"));
    }

    /// A handler returning Err must not abort dispatch to later-
    /// registered handlers. The error is logged and swallowed.
    #[test]
    fn failing_handler_does_not_abort_dispatch() {
        struct AlwaysFails;
        impl EventHandler for AlwaysFails {
            fn handle(&self, _: &RotationTrigger) -> Result<()> {
                anyhow::bail!("synthetic failure for test")
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(AlwaysFails)).unwrap();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();
        reg.fire(RotationTrigger::RegDomainChange {
            from: "00".into(),
            to: "US".into(),
        })
        .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "later handler must still run after an earlier one errored"
        );
    }

    /// Debug rendering must remain stable so log scrapers and
    /// snapshot tests don't break across patch releases.
    #[test]
    fn rotation_trigger_debug_rendering_is_stable() {
        let t = RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: Some("home".into()),
        };
        let s = format!("{t:?}");
        assert!(s.contains("ConnectionUp"));
        assert!(s.contains("wlan0"));
        assert!(s.contains("home"));

        let t = RotationTrigger::RegDomainChange {
            from: "00".into(),
            to: "US".into(),
        };
        let s = format!("{t:?}");
        assert!(s.contains("RegDomainChange"));
        assert!(s.contains("\"00\""));
        assert!(s.contains("\"US\""));
    }

    /// The four kinds map to stable string tokens. Tests pin the
    /// tokens because `proteus events --json` (follow-up) will key
    /// off them and operators will grep for them.
    #[test]
    fn rotation_trigger_kind_tokens_are_stable() {
        assert_eq!(
            RotationTrigger::ConnectionUp {
                iface: "wlan0".into(),
                ssid: None
            }
            .kind(),
            "connection-up"
        );
        assert_eq!(
            RotationTrigger::LinkFlap {
                iface: "wlan0".into()
            }
            .kind(),
            "link-flap"
        );
        assert_eq!(
            RotationTrigger::RegDomainChange {
                from: "00".into(),
                to: "US".into()
            }
            .kind(),
            "reg-domain-change"
        );
        assert_eq!(
            RotationTrigger::PortalAuth {
                ssid: "Cafe".into()
            }
            .kind(),
            "portal-auth"
        );
    }

    /// An empty registry accepts a `fire` call cleanly — it's a
    /// no-op, not an error. Important so an uninitialised registry
    /// (no handlers registered yet) doesn't crash the source loop.
    #[test]
    fn empty_registry_fires_cleanly() {
        let reg = EventRegistry::new();
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        assert_eq!(reg.handler_count(), 0);
    }

    /// Issue #254: the rate limiter wired into `fire` drops triggers
    /// past the per-kind cap. Use a tight 3/window limiter so the
    /// test doesn't have to fire ten triggers to prove the cap.
    #[test]
    fn fire_drops_triggers_past_per_kind_cap() {
        use std::time::Duration;
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg =
            EventRegistry::with_limiter(RateLimiter::with_capacity(3, Duration::from_secs(60)));
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();
        // First three fires of the same kind run.
        for _ in 0..3 {
            reg.fire(RotationTrigger::LinkFlap {
                iface: "wlan0".into(),
            })
            .unwrap();
        }
        assert_eq!(count.load(Ordering::SeqCst), 3);
        // Fourth and fifth: dropped by the limiter, handler not run.
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "limiter must drop the 4th+ trigger"
        );
        // A different kind has its own budget.
        reg.fire(RotationTrigger::PortalAuth {
            ssid: "Cafe".into(),
        })
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    /// Issue #252: a poisoned inner mutex must not silently drop
    /// `register` calls. The fix recovers via `into_inner()` and
    /// continues; we deliberately poison the mutex by panicking
    /// inside a closure that holds the guard, then assert that a
    /// later `register` still lands and the registered handler runs
    /// when the registry fires.
    #[test]
    fn register_recovers_from_poisoned_mutex() {
        let reg = Arc::new(EventRegistry::new());
        // Poison the inner mutex by panicking while holding the
        // guard. We do this on a worker thread so the test process
        // survives the panic.
        let reg_for_thread = Arc::clone(&reg);
        let _ = std::thread::spawn(move || {
            let _g = reg_for_thread.handlers.lock().unwrap();
            panic!("synthetic poison");
        })
        .join();
        assert!(reg.handlers.is_poisoned());

        // Register after poison: must succeed (recovered) and the
        // handler must be observable via `handler_count`.
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .expect("register on a poisoned registry must recover, not silently drop");
        assert_eq!(reg.handler_count(), 1);

        // And the registered handler still runs on `fire`.
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    /// Issue #252 mirror: `fire` on a poisoned mutex must dispatch
    /// to the recovered handler list rather than abort. This pins
    /// the "a handler panic does not silently disable the registry"
    /// invariant.
    #[test]
    fn fire_recovers_from_poisoned_mutex() {
        let reg = Arc::new(EventRegistry::new());
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();

        // Poison the inner mutex.
        let reg_for_thread = Arc::clone(&reg);
        let _ = std::thread::spawn(move || {
            let _g = reg_for_thread.handlers.lock().unwrap();
            panic!("synthetic poison");
        })
        .join();
        assert!(reg.handlers.is_poisoned());

        // Fire after poison: handler must still run (registry
        // recovered).
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .expect("fire on a poisoned registry must recover and dispatch");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
