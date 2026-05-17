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

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod rate_limit;
pub mod source;

pub use rate_limit::{Decision, RateLimiter};

/// The four trigger kinds the registry tracks. Listed in a stable order
/// so the status snapshot's per-source array is deterministic across
/// daemon restarts (matters for `--json` consumers and snapshot tests).
pub const KINDS: [&str; 4] = [
    "connection-up",
    "link-flap",
    "reg-domain-change",
    "portal-auth",
];

/// Per-trigger-kind counters: how many triggers fired, when the last
/// one landed, and how many were dropped by the rate limiter. The
/// daemon writes one of these into the on-disk snapshot consumed by
/// `proteus events status` (#393).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KindCounters {
    /// Number of triggers of this kind the registry observed (post
    /// rate-limit; matches handler-invocation count).
    pub fires: u64,
    /// Number of triggers of this kind the rate limiter dropped.
    pub rate_limited: u64,
    /// Unix-epoch seconds of the most recent fire (`None` if the kind
    /// has never fired in this run). Serialised as a number for stable
    /// JSON; the CLI renders it as ISO-8601 for the human view.
    pub last_fired_unix: Option<u64>,
}

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
    /// Roadmap C7: count handler panics observed by `fire`. Per-registry
    /// (not per-handler) because the handler index is not a stable
    /// identifier across daemon runs and the operator's question is
    /// "is anything panicking at all?" rather than "which slot." Exposed
    /// read-only via [`EventRegistry::handler_panic_count`] so a future
    /// `proteus events status` can surface it. Atomic so the read is
    /// lock-free and never blocks dispatch.
    handler_panics: AtomicU64,
    /// Total handler invocations across every kind+handler combination
    /// — counts `handler.handle(...)` calls that did not panic, so
    /// `handler_fires + handler_panics` is "post-rate-limit dispatches
    /// attempted." Atomic so the read path stays lock-free. Surfaced
    /// to `proteus events status` (#393).
    handler_fires: AtomicU64,
    /// Per-kind counters (fires, rate-limited drops, last-fired
    /// timestamp). Stored behind a mutex because `last_fired_unix` and
    /// the two counts need to update atomically wrt one another — a
    /// reader that saw `fires=5, last_fired=None` would be confusing.
    /// The mutex is uncontended in practice: only `fire` writes, and
    /// `fire` is already serialised by the handler-vec mutex below.
    kind_counters: Mutex<HashMap<&'static str, KindCounters>>,
}

impl EventRegistry {
    /// Build an empty registry with the default rate-limit budget.
    /// Followed by zero or more `register` calls before the source
    /// loop starts firing.
    pub fn new() -> Self {
        Self::with_limiter(RateLimiter::new())
    }

    /// Build a registry with an explicit rate limiter — used by
    /// tests that need to assert dispatch is dropped at the Nth
    /// trigger or by an orchestrator that wants to override the
    /// default cap. Production wiring uses [`EventRegistry::new`].
    pub fn with_limiter(limiter: RateLimiter) -> Self {
        // Pre-seed the per-kind map so a `Snapshot` taken before any
        // trigger fires still lists every kind with `fires = 0`. Keeps
        // the on-disk shape consumed by `proteus events status` (#393)
        // deterministic regardless of which sources have observed
        // events.
        let mut kinds = HashMap::with_capacity(KINDS.len());
        for k in KINDS {
            kinds.insert(k, KindCounters::default());
        }
        Self {
            handlers: Mutex::new(Vec::new()),
            limiter,
            handler_panics: AtomicU64::new(0),
            handler_fires: AtomicU64::new(0),
            kind_counters: Mutex::new(kinds),
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
    ///
    /// Roadmap C7: handler panics used to be silently swallowed —
    /// `EventHandler::handle` is synchronous so an unwind would
    /// propagate to the calling source task; a `tokio::spawn` source
    /// task with a panicking handler would have its `JoinHandle`
    /// resolve to `Err(JoinError::is_panic())` only at shutdown, and
    /// the shutdown path discards that error. Wrap every handler call
    /// in `std::panic::catch_unwind` so the panic surfaces at
    /// `tracing::error!` with the handler index, trigger kind, and
    /// best-effort downcast of the panic payload to `&str` / `String`,
    /// then continue dispatching to subsequent handlers. The per-
    /// registry [`EventRegistry::handler_panic_count`] counter is
    /// bumped so a future `proteus events status` can answer "did any
    /// handler panic in this run." `AssertUnwindSafe` is sound here:
    /// the handler trait object lives behind a `&dyn EventHandler` and
    /// the trigger is borrowed; we do not observe partially-mutated
    /// state across the unwind boundary.
    pub fn fire(&self, trigger: RotationTrigger) -> Result<()> {
        let kind = trigger.kind();
        let now = Instant::now();
        if let Decision::RateLimited(consecutive) = self.limiter.check_and_record(kind, now) {
            // #393: tally limiter drops per-kind so a future
            // `proteus events status` can answer "why aren't my
            // triggers firing." Locked inside the same recovery
            // pattern the handler mutex uses below.
            self.bump_rate_limited(kind);
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
        // #393: record the kind-fire before dispatch so the snapshot's
        // "last_fired" reflects observation time, not handler-completion
        // time (mirrors the `RotateOnTriggerHandler` counter semantics).
        self.bump_fire(kind);
        let handlers = match self.handlers.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("registry mutex was poisoned; recovered");
                poisoned.into_inner()
            }
        };
        for (idx, h) in handlers.iter().enumerate() {
            // C7: catch_unwind so a panicking handler does not take
            // down the source task. Each handler is called inside the
            // unwind boundary; subsequent handlers see a fresh frame.
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| h.handle(&trigger)));
            match result {
                Ok(Ok(())) => {
                    self.handler_fires.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Err(e)) => {
                    // Errors still count as "the handler ran" for the
                    // fires tally — they are observable executions, just
                    // unhappy ones. Panics get their own counter below.
                    self.handler_fires.fetch_add(1, Ordering::SeqCst);
                    tracing::warn!(handler_index = idx, kind = kind, "handler error: {e:#}");
                }
                Err(payload) => {
                    self.handler_panics.fetch_add(1, Ordering::SeqCst);
                    let msg = panic_payload_message(payload.as_ref());
                    tracing::error!(
                        handler_index = idx,
                        kind = kind,
                        panic = msg.as_str(),
                        "events: handler panicked; continuing dispatch to remaining handlers"
                    );
                }
            }
        }
        Ok(())
    }

    /// #393: poisoned-mutex-safe access to the per-kind counter map.
    /// Mirrors the registry's "a handler panic must not silently
    /// disable counters" stance applied to the handler-vec mutex.
    fn with_kind_counters<R>(
        &self,
        f: impl FnOnce(&mut HashMap<&'static str, KindCounters>) -> R,
    ) -> R {
        let mut map = match self.kind_counters.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut map)
    }

    /// #393: per-kind fire counter + last-fired timestamp update.
    fn bump_fire(&self, kind: &'static str) {
        self.with_kind_counters(|map| {
            let entry = map.entry(kind).or_default();
            entry.fires = entry.fires.saturating_add(1);
            entry.last_fired_unix = Some(now_unix_secs());
        });
    }

    /// #393: per-kind rate-limited counter. Bumped on every dropped
    /// trigger, not just the first in a streak, so the snapshot
    /// reflects total drop volume.
    fn bump_rate_limited(&self, kind: &'static str) {
        self.with_kind_counters(|map| {
            let entry = map.entry(kind).or_default();
            entry.rate_limited = entry.rate_limited.saturating_add(1);
        });
    }

    /// #393: clone the per-kind counters for the snapshot writer.
    /// Returns the map keyed by the stable token from
    /// [`RotationTrigger::kind`].
    pub fn kind_counters_snapshot(&self) -> HashMap<&'static str, KindCounters> {
        self.with_kind_counters(|map| map.clone())
    }

    /// #393: total handler invocations (Ok + Err returns). Panics are
    /// counted separately by [`handler_panic_count`].
    pub fn handler_fire_count(&self) -> u64 {
        self.handler_fires.load(Ordering::SeqCst)
    }

    /// Roadmap C7: count of handler panics observed since the registry
    /// was constructed. Exposed read-only so a future
    /// `proteus events status` (or the daemon's own telemetry line)
    /// can surface "handlers panicked N times this run" without
    /// exposing the underlying atomic.
    pub fn handler_panic_count(&self) -> u64 {
        self.handler_panics.load(Ordering::SeqCst)
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

/// #393: schema version embedded in every status snapshot. Bump when
/// the on-disk shape changes incompatibly so old readers refuse to
/// render a newer snapshot rather than mis-parsing it.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

impl Snapshot {
    /// Build a fresh snapshot from the registry's live counters plus
    /// the daemon-supplied per-source live/idle state. The `live`
    /// closure answers "is the source's task currently alive?" — the
    /// daemon caller derives this from its `tasks: Vec<SourceTask>`.
    pub fn capture(
        registry: &EventRegistry,
        started_at_unix: u64,
        pid: u32,
        mut live: impl FnMut(&'static str) -> bool,
    ) -> Self {
        let counters = registry.kind_counters_snapshot();
        let mut sources = Vec::with_capacity(KINDS.len());
        let mut total_triggers: u64 = 0;
        let mut total_rate_limited: u64 = 0;
        for &kind in KINDS.iter() {
            let c = counters.get(kind).cloned().unwrap_or_default();
            total_triggers = total_triggers.saturating_add(c.fires);
            total_rate_limited = total_rate_limited.saturating_add(c.rate_limited);
            sources.push(SourceStatus {
                name: kind.to_string(),
                status: if live(kind) { "running" } else { "idle" }.to_string(),
                triggers_fired: c.fires,
                rate_limited: c.rate_limited,
                last_fired_unix: c.last_fired_unix,
            });
        }
        // Today there is one registered handler (the default
        // rotation handler). When that changes the daemon should
        // pass per-handler counters in via a richer accessor; for
        // now the aggregate registry counters are the source of
        // truth.
        let handler = HandlerStatus {
            name: "rotate-on-trigger".to_string(),
            fires: registry.handler_fire_count(),
            panics: registry.handler_panic_count(),
            // The registry doesn't track "last handler fire" separately
            // from "last kind fire" — they're the same instant for a
            // single-handler daemon. Surface the most recent kind fire
            // as the handler's last-fired so the row is non-empty.
            last_fired_unix: sources.iter().filter_map(|s| s.last_fired_unix).max(),
        };
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            pid,
            started_at_unix,
            wrote_at_unix: now_unix_secs(),
            sources,
            handlers: vec![handler],
            total_triggers,
            total_rate_limited,
        }
    }
}

/// Wall-clock seconds since the Unix epoch, saturating to `0` if the
/// clock is set before 1970 (it isn't — this is just a non-panicking
/// shape we can pass to `SystemTime::duration_since`). Used by the
/// status-snapshot writer (#393) so the on-disk timestamp survives
/// serde round-trips as a plain integer.
pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// #393: on-disk snapshot the events daemon writes periodically so
/// `proteus events status` (a read-only sibling of `events run`) can
/// answer "how is the daemon doing?" without an IPC channel. The shape
/// is the documented public contract: per-source counters, per-handler
/// counters, daemon uptime, total triggers handled. Schema version
/// pins the serialisation so a future change is detectable by old
/// readers.
///
/// "Per-source" maps 1:1 to trigger kinds — each production source
/// (`nm-connection-up`, `link-flap`, `reg-domain-change`,
/// `portal-auth`) emits a single trigger kind, so the source name and
/// the trigger kind token are interchangeable in the snapshot. The
/// `status` of a source is "running" when its `SourceTask` is alive at
/// snapshot time and "idle" otherwise; "error" is reserved for the
/// future case where a source records a permanent-failure reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    /// Schema version. Bump when the on-disk shape changes so old
    /// `proteus events status` binaries can refuse to render a newer
    /// snapshot instead of mis-parsing it.
    pub schema_version: u32,
    /// Process id of the daemon that wrote this snapshot. The CLI
    /// uses this for the human-readable header; staleness is judged
    /// off `wrote_at_unix`, not the pid.
    pub pid: u32,
    /// Unix-epoch seconds when the daemon started.
    pub started_at_unix: u64,
    /// Unix-epoch seconds when this snapshot was written. The CLI
    /// compares this against `now` to decide whether the daemon is
    /// alive (snapshot freshness threshold lives at the read site).
    pub wrote_at_unix: u64,
    /// Per-source rows. One entry per known trigger kind; order is
    /// stable across snapshots (matches [`KINDS`]).
    pub sources: Vec<SourceStatus>,
    /// Per-handler rows. Today every daemon registers exactly one
    /// rotation handler; the shape is plural so a future
    /// per-component handler registration story doesn't break the
    /// on-disk contract.
    pub handlers: Vec<HandlerStatus>,
    /// Sum of `sources[*].fires` — total triggers post rate-limit.
    pub total_triggers: u64,
    /// Sum of `sources[*].rate_limited`.
    pub total_rate_limited: u64,
}

/// #393: one source's row in the snapshot. The name is the stable
/// token from [`source::EventSource::name`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceStatus {
    pub name: String,
    /// `"running"` | `"idle"` | `"error"`. The daemon decides the
    /// label at snapshot-write time; the CLI does not interpret it.
    pub status: String,
    pub triggers_fired: u64,
    pub rate_limited: u64,
    pub last_fired_unix: Option<u64>,
}

/// #393: one handler's row in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandlerStatus {
    pub name: String,
    pub fires: u64,
    pub panics: u64,
    pub last_fired_unix: Option<u64>,
}

/// Roadmap C7: best-effort downcast of a `Box<dyn Any + Send>` panic
/// payload (the shape `std::panic::catch_unwind` /
/// `JoinError::into_panic` hand back) to a human-readable string. The
/// conventional encodings are `&'static str` (from `panic!("literal")`)
/// and `String` (from `panic!("{}", ...)`). Anything else renders as
/// `"<non-string panic payload>"` — the daemon stays up, and the
/// operator at least sees that *something* panicked even if the
/// payload is a bespoke type. Logging the type id would be nice but
/// is not stable across compiler versions.
pub fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
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

    /// Roadmap C7: a handler that panics inside `handle` must not
    /// take down the registry, must not abort dispatch to later
    /// handlers, and must be observable via the per-registry panic
    /// counter. Pre-fix this was a silent abort path — the panic
    /// would unwind into the source task, leaving subsequent
    /// handlers un-invoked and the operator with no signal that
    /// anything had gone wrong.
    #[test]
    fn panicking_handler_does_not_abort_dispatch_and_bumps_counter() {
        struct AlwaysPanics;
        impl EventHandler for AlwaysPanics {
            fn handle(&self, _: &RotationTrigger) -> Result<()> {
                panic!("synthetic handler panic for C7");
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(AlwaysPanics)).unwrap();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();

        // The fire call itself must return Ok — the panic is caught
        // and logged inside the registry. Pre-fix this would have
        // unwound the calling source task.
        reg.fire(RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: Some("home".into()),
        })
        .expect("fire must return Ok after a handler panic");

        // The second handler still ran — panic is isolated.
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "later handler must still run after an earlier one panicked"
        );
        // The panic counter is bumped exactly once.
        assert_eq!(
            reg.handler_panic_count(),
            1,
            "panic counter must reflect the one observed handler panic"
        );

        // Fire again — the panicking handler panics again, the
        // healthy one runs again, and the panic counter advances.
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .expect("fire must keep returning Ok across repeated handler panics");
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(reg.handler_panic_count(), 2);
    }

    /// C7: the panic-payload downcaster handles both conventional
    /// encodings (`&'static str` from `panic!("literal")`, `String`
    /// from `panic!("{}", ...)`) and falls back to a placeholder
    /// for everything else. Pin so the log surface stays operator-
    /// readable across the kinds of panic call sites Rust code
    /// emits.
    #[test]
    fn panic_payload_message_handles_str_string_and_other() {
        let str_payload: Box<dyn std::any::Any + Send> = Box::new("static literal");
        assert_eq!(
            panic_payload_message(str_payload.as_ref()),
            "static literal"
        );

        let string_payload: Box<dyn std::any::Any + Send> = Box::new(String::from("owned text"));
        assert_eq!(panic_payload_message(string_payload.as_ref()), "owned text");

        // u32 is the canonical "anything else" — the placeholder
        // path keeps the daemon from rendering raw type ids.
        let other_payload: Box<dyn std::any::Any + Send> = Box::new(42u32);
        assert_eq!(
            panic_payload_message(other_payload.as_ref()),
            "<non-string panic payload>"
        );
    }

    /// C7: a panicking handler followed by a panicking handler still
    /// dispatches all the way through (every panic is caught and
    /// counted individually). Pins the "every handler runs in its
    /// own unwind boundary" shape so a future refactor cannot fall
    /// back to a single outer boundary that aborts on the first
    /// panic.
    #[test]
    fn multiple_panicking_handlers_each_bump_the_counter() {
        struct AlwaysPanics(&'static str);
        impl EventHandler for AlwaysPanics {
            fn handle(&self, _: &RotationTrigger) -> Result<()> {
                panic!("{}", self.0);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(AlwaysPanics("first panicker")))
            .unwrap();
        reg.register(Box::new(AlwaysPanics("second panicker")))
            .unwrap();
        // The trailing healthy handler proves dispatch reached the
        // end of the list past two consecutive panics.
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();

        reg.fire(RotationTrigger::PortalAuth {
            ssid: "Cafe".into(),
        })
        .expect("fire must return Ok after multiple handler panics");

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(reg.handler_panic_count(), 2);
    }

    /// #393: every fire bumps the per-kind counter and refreshes the
    /// last-fired timestamp. Pin so the snapshot writer that surfaces
    /// these values can't silently regress.
    #[test]
    fn fire_bumps_per_kind_counters_and_last_fired_timestamp() {
        let reg = EventRegistry::new();
        reg.fire(RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: None,
        })
        .unwrap();
        reg.fire(RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: None,
        })
        .unwrap();
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        let counters = reg.kind_counters_snapshot();
        assert_eq!(counters.get("connection-up").unwrap().fires, 2);
        assert_eq!(counters.get("link-flap").unwrap().fires, 1);
        assert_eq!(counters.get("portal-auth").unwrap().fires, 0);
        // Last-fired is recorded for the kinds we fired and absent
        // for the ones we did not.
        assert!(
            counters
                .get("connection-up")
                .unwrap()
                .last_fired_unix
                .is_some()
        );
        assert!(counters.get("link-flap").unwrap().last_fired_unix.is_some());
        assert!(
            counters
                .get("portal-auth")
                .unwrap()
                .last_fired_unix
                .is_none()
        );
    }

    /// #393: rate-limited fires bump the per-kind rate_limited counter
    /// but not the fires counter — they never reached the handlers.
    #[test]
    fn rate_limited_fires_only_bump_rate_limited_counter() {
        use std::time::Duration;
        let reg =
            EventRegistry::with_limiter(RateLimiter::with_capacity(1, Duration::from_secs(60)));
        // First fire passes the limiter; second is dropped.
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        reg.fire(RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        let counters = reg.kind_counters_snapshot();
        let lf = counters.get("link-flap").unwrap();
        assert_eq!(lf.fires, 1);
        assert_eq!(lf.rate_limited, 1);
    }

    /// #393: handler fire counter advances per dispatch (Ok and Err
    /// both count). Pre-fix the registry only tracked panics; this
    /// test pins the new shape so a future refactor can't silently
    /// stop counting.
    #[test]
    fn handler_fire_counter_advances_on_each_dispatch() {
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let reg = EventRegistry::new();
        reg.register(Box::new(CountingHandler {
            count: Arc::clone(&count),
            last_kind: Arc::clone(&last),
        }))
        .unwrap();
        assert_eq!(reg.handler_fire_count(), 0);
        for _ in 0..3 {
            reg.fire(RotationTrigger::LinkFlap {
                iface: "wlan0".into(),
            })
            .unwrap();
        }
        assert_eq!(reg.handler_fire_count(), 3);
    }

    /// #393: `Snapshot::capture` matches the live registry state and
    /// labels sources `running` vs `idle` based on the `live` closure.
    #[test]
    fn snapshot_capture_reflects_registry_and_live_set() {
        let reg = EventRegistry::new();
        reg.fire(RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: None,
        })
        .unwrap();
        reg.fire(RotationTrigger::PortalAuth {
            ssid: "Cafe".into(),
        })
        .unwrap();
        // The live closure receives trigger-kind tokens (matching the
        // snapshot row key, not the source-task name).
        let snap = Snapshot::capture(&reg, 1_700_000_000, 42, |name| {
            name == "connection-up" || name == "portal-auth"
        });
        assert_eq!(snap.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snap.pid, 42);
        assert_eq!(snap.started_at_unix, 1_700_000_000);
        assert_eq!(snap.total_triggers, 2);
        // Sources are ordered per `KINDS` and labelled per the live
        // closure.
        let names: Vec<&str> = snap.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, KINDS.to_vec());
        let nm = snap
            .sources
            .iter()
            .find(|s| s.name == "connection-up")
            .unwrap();
        assert_eq!(nm.status, "running");
        assert_eq!(nm.triggers_fired, 1);
        let lf = snap.sources.iter().find(|s| s.name == "link-flap").unwrap();
        assert_eq!(lf.status, "idle");
        assert_eq!(lf.triggers_fired, 0);
        // Single handler row with the aggregate fires/panics.
        assert_eq!(snap.handlers.len(), 1);
        assert_eq!(snap.handlers[0].name, "rotate-on-trigger");
        // No handlers were registered, so fires stays at 0 even though
        // two triggers landed.
        assert_eq!(snap.handlers[0].fires, 0);
        assert_eq!(snap.handlers[0].panics, 0);
    }
}
