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

use anyhow::Result;

pub mod source;

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
    ConnectionUp {
        iface: String,
        ssid: Option<String>,
    },
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
}

impl EventRegistry {
    /// Build an empty registry. Followed by zero or more `register`
    /// calls before the source loop starts firing.
    pub fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
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
    pub fn register(&self, handler: Box<dyn EventHandler>) {
        if let Ok(mut h) = self.handlers.lock() {
            h.push(handler);
        }
    }

    /// Number of registered handlers. Mostly for tests; the live
    /// dispatch path doesn't care.
    pub fn handler_count(&self) -> usize {
        self.handlers.lock().map(|h| h.len()).unwrap_or(0)
    }

    /// Fire a trigger to every registered handler in registration
    /// order. Each handler's error is logged via `tracing::warn` and
    /// then suppressed — one failing handler must not silence the
    /// rest.
    pub fn fire(&self, trigger: RotationTrigger) -> Result<()> {
        let handlers = match self.handlers.lock() {
            Ok(g) => g,
            Err(e) => anyhow::bail!("event registry mutex poisoned: {e}"),
        };
        let kind = trigger.kind();
        for (idx, h) in handlers.iter().enumerate() {
            if let Err(e) = h.handle(&trigger) {
                tracing::warn!(handler_index = idx, kind = kind, "handler error: {e:#}");
            }
        }
        Ok(())
    }
}

impl Default for EventRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        }));
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
        }));
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&c2),
            last_kind: Arc::clone(&last2),
        }));
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
        reg.register(Box::new(AlwaysFails));
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }));
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
}
