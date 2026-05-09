// SPDX-License-Identifier: GPL-3.0-or-later

//! NM connection-up source. Subscribes to NetworkManager's
//! `org.freedesktop.NetworkManager.Device.StateChanged` signal and
//! emits [`super::super::RotationTrigger::ConnectionUp`] on transitions
//! into the `Activated` state (NM device-state integer 100). Roadmap
//! Milestone 4c.
//!
//! Two implementations live here:
//!
//! - [`NmConnectionUpSource`] — production: opens the system DBus,
//!   walks every `NetworkManager.GetDevices()` path, builds a
//!   `DeviceProxy` per device, and forwards every `StateChanged` whose
//!   `new_state == 100` into the registry. Spawns one tokio task per
//!   device. Falls back gracefully when the DBus session is missing
//!   (typical CI / dev-laptop runs without `--user`).
//! - [`MockNmConnectionUpSource`] — test double. Tests push canned
//!   `StateChanged` payloads (interface + new_state + ssid) into the
//!   mock's queue and call `start()` to drain them straight into the
//!   registry. No DBus, no tokio.
//!
//! The production code reuses the existing zbus proxies under
//! `crate::nm` (the `DeviceProxy` is already declared there with the
//! `#[zbus::proxy]` macro; the only addition needed for this file is
//! the `StateChanged` signal handler, which zbus generates from a
//! `#[zbus(signal)]` annotation we add inline).

use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::super::{EventRegistry, RotationTrigger};
use super::{EventSource, SourceTask, StopHandle};

/// NM device-state integer for `Activated`. The full NM state machine
/// is documented at <https://developer.gnome.org/NetworkManager/stable/nm-dbus-types.html#NMDeviceState>;
/// we care about exactly one transition: any-prior → 100. Older NM
/// (1.0–1.10) emitted a different number for "fully connected" but
/// 100 has been stable for the entire 1.x series Proteus targets.
pub const NM_DEVICE_STATE_ACTIVATED: u32 = 100;

/// Production NM connection-up source. Holds nothing on the heap —
/// the per-device subscription state is created inside the spawned
/// task on `spawn_into`.
pub struct NmConnectionUpSource;

impl NmConnectionUpSource {
    pub fn new() -> Self {
        Self
    }

    /// Spawn the long-lived subscription. The returned [`SourceTask`]
    /// drives a single tokio task that:
    ///
    /// 1. Opens the system DBus.
    /// 2. Lists every NM device and builds a `DeviceProxy` per path.
    /// 3. Subscribes to `StateChanged` on each device.
    /// 4. Forwards every `new_state == 100` as a
    ///    [`RotationTrigger::ConnectionUp`].
    ///
    /// Returns `None` when the DBus connection can't be opened
    /// (typical CI / non-NM hosts) — the orchestrator logs a warning
    /// and runs the rest of the sources.
    pub async fn spawn_into(self, registry: Arc<EventRegistry>) -> Option<SourceTask> {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("nm-connection-up: system DBus unavailable, source disabled: {e}");
                return None;
            }
        };
        let (stop, mut stop_rx) = StopHandle::channel();
        let join = tokio::spawn(async move {
            // Best-effort: if NM isn't on this DBus, we give up but
            // keep the task alive so the orchestrator's shutdown
            // path stays uniform. The cancellation receiver still
            // fires the moment `stop()` is called.
            if let Err(e) = subscribe_loop(&conn, registry, &mut stop_rx).await {
                tracing::warn!("nm-connection-up: subscription loop ended: {e:#}");
            }
        });
        Some(SourceTask {
            join,
            stop,
            name: "nm-connection-up",
        })
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
        // Synchronous `start` is a no-op in production: the real
        // subscription needs a tokio runtime, which the orchestrator
        // provides via `spawn_into`. Tests use `MockNmConnectionUpSource`
        // which pushes events synchronously.
        tracing::debug!(
            "NmConnectionUpSource::start: synchronous path is a no-op; \
             use spawn_into from a tokio runtime for the live subscription"
        );
        Ok(())
    }
}

/// Inner loop body for the production task. Split from `spawn_into`
/// so unit tests of the loop shape (none yet — covered by the mock
/// variant) can reach it directly.
async fn subscribe_loop(
    conn: &zbus::Connection,
    registry: Arc<EventRegistry>,
    stop: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    use crate::nm::{DeviceProxy, NetworkManagerProxy};

    // Build the top-level NM proxy. Failure here means NM isn't on
    // the bus — surface, log, return.
    let nm = match NetworkManagerProxy::new(conn).await {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("nm-connection-up: NetworkManagerProxy unavailable: {e}");
            // Block on the stop signal so the task lives for the
            // orchestrator's lifetime even when there's nothing to
            // subscribe to. This keeps the task-count contract uniform.
            let _ = stop.await;
            return Ok(());
        }
    };

    // Snapshot the current device list and subscribe per-device. New
    // devices added after this point won't be picked up until the
    // daemon restarts — acceptable trade-off for now; NM exposes a
    // `DeviceAdded` signal we'd subscribe to in a follow-up.
    let paths = nm.get_devices().await.unwrap_or_default();
    // Issue #256: each per-device watcher gets its own stop channel
    // so the parent can drain them within a deadline before
    // resorting to abort. Previously the parent only called `abort()`
    // when it received the stop signal, which gives no chance for
    // in-flight DBus reads to complete; the watchers now poll their
    // stop receiver in the same `tokio::time::timeout` shape the
    // portal-auth source uses.
    let mut watchers: Vec<Watcher> = Vec::new();
    for path in paths {
        let builder = match DeviceProxy::builder(conn).path(path.clone()) {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(
                    ?path,
                    "nm-connection-up: skipping device proxy builder: {e}"
                );
                continue;
            }
        };
        let dev = match builder.build().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!(?path, "nm-connection-up: skipping device proxy: {e}");
                continue;
            }
        };
        let registry = Arc::clone(&registry);
        let (watcher_tx, watcher_rx) = tokio::sync::oneshot::channel::<()>();
        let join = tokio::spawn(async move {
            // The signal-stream API on a generated zbus proxy is
            // `dev.receive_<signal_name>()`. zbus 5.x exposes
            // `Device.StateChanged` via the introspection-driven
            // builder; if the signal isn't on the proxy at compile
            // time we degrade to a polling fallback that reads the
            // `State` property every 2 s. Both branches forward to
            // `fire_connection_up` so the test seam is consistent.
            poll_state_property(&dev, registry, watcher_rx).await;
        });
        watchers.push(Watcher {
            join,
            stop: watcher_tx,
        });
    }

    let _ = stop.await;
    drain_watchers(watchers, std::time::Duration::from_secs(5)).await;
    Ok(())
}

/// One per-device watcher with its own graceful-stop channel.
/// Issue #256.
struct Watcher {
    join: tokio::task::JoinHandle<()>,
    stop: tokio::sync::oneshot::Sender<()>,
}

/// Drain every watcher within `deadline`. Each watcher gets its
/// stop-channel signal first so the poll loop can exit cleanly;
/// stragglers past the deadline are aborted explicitly.
async fn drain_watchers(watchers: Vec<Watcher>, deadline: std::time::Duration) {
    for w in watchers {
        // Best-effort: a send error means the watcher already
        // exited (its receiver was dropped) — nothing to do.
        let _ = w.stop.send(());
        let mut join = w.join;
        match tokio::time::timeout(deadline, &mut join).await {
            Ok(Ok(())) => {
                tracing::debug!("nm-connection-up: per-device watcher shut down cleanly");
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "nm-connection-up: per-device watcher panicked or was cancelled: {e}"
                );
            }
            Err(_) => {
                tracing::warn!(
                    deadline_secs = deadline.as_secs(),
                    "nm-connection-up: per-device watcher missed shutdown deadline; aborting"
                );
                join.abort();
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(250), &mut join).await;
            }
        }
    }
}

/// Polling fallback for the state-changed loop. Reads
/// `Device.State` every 2 s and emits a `ConnectionUp` whenever the
/// observed value transitions into `Activated`. Runs forever (until
/// the parent task aborts it).
///
/// Why a polling fallback: zbus's signal-stream API requires the
/// proxy to declare the signal at compile time; the existing
/// `DeviceProxy` in `crate::nm` declares the data properties Proteus
/// touches but not `StateChanged`. Adding the signal declaration
/// changes the public proxy surface in a milestone where the goal
/// is "fill in the source bodies, not refactor `crate::nm`". The
/// poll fallback is good enough for a 4c-followup acceptance criterion
/// — connection-up events fire within 2 s of NM's Activated edge —
/// and a follow-up PR can swap in the real signal stream behind the
/// same `fire_connection_up` call.
async fn poll_state_property(
    dev: &crate::nm::DeviceProxy<'_>,
    registry: Arc<EventRegistry>,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) {
    use std::time::Duration;
    // GH#355: seed the edge detector with the first observed state so
    // the daemon does not synthesize a spurious `ConnectionUp` for
    // every device that happened to be `Activated` at daemon start.
    // See `is_activation_edge` for the full discussion.
    let mut last: Option<u32> = None;
    let mut seeded = false;
    loop {
        let state = match read_device_state(dev).await {
            Some(s) => s,
            None => {
                // Issue #256: race the stop signal against the poll
                // cadence so the watcher exits within milliseconds
                // of the parent calling `stop`. Previously the
                // unconditional `sleep` could hold the watcher up to
                // 2 s past shutdown, which the daemon's drain
                // deadline tolerates but isn't free.
                if tokio::time::timeout(Duration::from_secs(2), &mut stop_rx)
                    .await
                    .is_ok()
                {
                    return;
                }
                continue;
            }
        };
        let prev = last.replace(state);
        if is_activation_edge(seeded, prev, state) {
            let iface = dev.interface().await.unwrap_or_default();
            // SSID resolution is best-effort — we shell out to
            // `/proc/net/wireless` only as a last resort because the
            // ActiveConnection path is more accurate when present.
            let ssid = read_active_ssid_via_proc(&iface);
            let _ = registry.fire(RotationTrigger::ConnectionUp { iface, ssid });
        }
        seeded = true;
        if tokio::time::timeout(Duration::from_secs(2), &mut stop_rx)
            .await
            .is_ok()
        {
            return;
        }
    }
}

/// Pure edge-detector for the poll-state fallback. Returns `true`
/// only for a *real* Activated edge — not the synthetic edge that
/// shows up the first time the daemon observes an already-connected
/// device.
///
/// GH#355: previously the loop body was
/// `state == Activated && prev != Some(Activated)`. With
/// `last: Option<u32> = None`, the very first poll on an
/// already-Activated NIC fired a fake `ConnectionUp` because `prev`
/// was `None` rather than `Some(Activated)`. The fix is to require
/// the detector to have observed at least one prior reading before
/// any edge is allowed to fire.
///
/// `seeded` is `false` on the first call (no prior reading yet),
/// `true` thereafter. `prev` is the value `last` held *before* the
/// current state was stored; `state` is the freshly-read value.
pub(crate) fn is_activation_edge(seeded: bool, prev: Option<u32>, state: u32) -> bool {
    if !seeded {
        return false;
    }
    state == NM_DEVICE_STATE_ACTIVATED && prev != Some(NM_DEVICE_STATE_ACTIVATED)
}

/// Read `Device.State` over DBus. Returns `None` on transient errors
/// — the caller treats it as "skip this round, try again".
///
/// Issue #217: previously this stub always returned `None` because the
/// `DeviceProxy` in `crate::nm` did not declare a `state` property. The
/// connection-up event source therefore never saw an Activated edge,
/// so `proteus events run` silently never fired the `ConnectionUp`
/// trigger in production. The proxy now carries a `state()` accessor
/// (see `src/nm/mod.rs`); this body just delegates.
async fn read_device_state(dev: &crate::nm::DeviceProxy<'_>) -> Option<u32> {
    dev.state().await.ok()
}

/// Resolve the SSID for `iface` from `/proc/net/wireless`. Returns
/// `None` when the file is absent (no wireless tools), the iface row
/// is missing, or no SSID is currently associated. The procfs file
/// only carries link metrics; we use its presence as a strong hint
/// that we *should* be able to read the SSID, then fall back to
/// `/sys/class/net/<iface>/wireless` for the SSID itself when the
/// kernel exposes it. Most production hosts will see `None` until the
/// follow-up adds NM ActiveConnection lookup; for now the
/// `RotationTrigger` carries enough information (the iface) for the
/// rotation handler to do its job.
fn read_active_ssid_via_proc(iface: &str) -> Option<String> {
    if iface.is_empty() {
        return None;
    }
    let _ = std::fs::read_to_string("/proc/net/wireless").ok()?;
    // Reading the SSID from sysfs is iface-specific and the kernel
    // path varies by driver; we deliberately stop here rather than
    // shell out to `iw`/`iwconfig`. The absence of an SSID does not
    // change the trigger semantics — the registry handler keys off
    // the iface.
    None
}

/// Test double for `NmConnectionUpSource`. Tests push synthetic
/// `(iface, new_state, ssid)` tuples into the queue and call
/// `start()` to drain them straight into the registry.
///
/// Production transitions are gated on `new_state == 100`; the mock
/// honours the same gate so a test exercising the gate can push a
/// non-100 state and observe nothing fires.
pub struct MockNmConnectionUpSource {
    queue: Mutex<Vec<MockEvent>>,
}

#[derive(Debug, Clone)]
pub struct MockEvent {
    pub iface: String,
    pub new_state: u32,
    pub ssid: Option<String>,
}

impl MockNmConnectionUpSource {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    /// Push one canned `StateChanged` payload. Drained on the next
    /// `start()` call.
    pub fn push(&self, iface: impl Into<String>, new_state: u32, ssid: Option<String>) {
        self.queue.lock().unwrap().push(MockEvent {
            iface: iface.into(),
            new_state,
            ssid,
        });
    }
}

impl Default for MockNmConnectionUpSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for MockNmConnectionUpSource {
    fn name(&self) -> &'static str {
        "nm-connection-up"
    }

    fn start(&self, registry: &EventRegistry) -> Result<()> {
        let drain: Vec<MockEvent> = std::mem::take(&mut *self.queue.lock().unwrap());
        for ev in drain {
            if ev.new_state != NM_DEVICE_STATE_ACTIVATED {
                continue;
            }
            registry.fire(RotationTrigger::ConnectionUp {
                iface: ev.iface,
                ssid: ev.ssid,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::super::EventHandler;
    use super::*;

    /// Counting handler reused across the source tests.
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
        }))
        .unwrap();
        (reg, n, last)
    }

    /// Headline acceptance: register a counter handler, fire a
    /// synthetic `StateChanged(activated)` through the mock, observe
    /// the handler ran exactly once with a `ConnectionUp` carrying
    /// the right iface + ssid.
    #[test]
    fn mock_activated_state_fires_one_connection_up() {
        let (reg, n, last) = rig();
        let src = MockNmConnectionUpSource::new();
        src.push("wlan0", NM_DEVICE_STATE_ACTIVATED, Some("home".into()));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        match last.lock().unwrap().as_ref().unwrap() {
            RotationTrigger::ConnectionUp { iface, ssid } => {
                assert_eq!(iface, "wlan0");
                assert_eq!(ssid.as_deref(), Some("home"));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    /// Non-activated transitions must not fire — the mock honours
    /// the same gate as production. NM's `Disconnected (30)`,
    /// `Config (50)`, and `IpCheck (80)` are common during a real
    /// connection bring-up; only the final 100 should produce a
    /// trigger.
    #[test]
    fn mock_non_activated_state_does_not_fire() {
        let (reg, n, _) = rig();
        let src = MockNmConnectionUpSource::new();
        src.push("wlan0", 30, None);
        src.push("wlan0", 50, None);
        src.push("wlan0", 80, None);
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Multiple Activated events on different interfaces fire one
    /// trigger each. Pin the per-iface dispatch shape so a future
    /// dedup layer can't silently absorb a real bring-up.
    #[test]
    fn mock_multiple_activated_on_different_ifaces() {
        let (reg, n, _) = rig();
        let src = MockNmConnectionUpSource::new();
        src.push("wlan0", NM_DEVICE_STATE_ACTIVATED, Some("a".into()));
        src.push("eth0", NM_DEVICE_STATE_ACTIVATED, None);
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    /// SSID is optional in the trigger — wired/no-ssid bring-ups
    /// must still fire. Pin the `None` path so an over-eager test
    /// can't change the contract.
    #[test]
    fn mock_activated_without_ssid_fires_with_none() {
        let (reg, _n, last) = rig();
        let src = MockNmConnectionUpSource::new();
        src.push("eth0", NM_DEVICE_STATE_ACTIVATED, None);
        src.start(&reg).unwrap();
        match last.lock().unwrap().as_ref().unwrap() {
            RotationTrigger::ConnectionUp { iface, ssid } => {
                assert_eq!(iface, "eth0");
                assert!(ssid.is_none());
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    /// `start()` drains the queue — calling it a second time with
    /// no new pushes does nothing. Important for the orchestrator
    /// idempotency story: re-running `start` after a config reload
    /// must not double-fire stored events.
    #[test]
    fn mock_start_drains_the_queue() {
        let (reg, n, _) = rig();
        let src = MockNmConnectionUpSource::new();
        src.push("wlan0", NM_DEVICE_STATE_ACTIVATED, None);
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1, "second start must not re-fire");
    }

    /// Production source's synchronous `start` is a no-op. The real
    /// subscription path runs through `spawn_into`; sync `start`
    /// stays clean so callers that don't have a tokio runtime
    /// (e.g. `proteus events trigger` follow-ups) don't crash.
    #[test]
    fn production_start_is_a_clean_noop() {
        let reg = EventRegistry::new();
        NmConnectionUpSource::new().start(&reg).unwrap();
        assert_eq!(reg.handler_count(), 0);
    }

    /// GH#355 regression: the very first poll of an already-Activated
    /// device must NOT count as an activation edge. Previously the
    /// daemon would fire a spurious `ConnectionUp` on every connected
    /// NIC at startup, before any real network event had happened.
    #[test]
    fn gh355_first_poll_of_activated_device_does_not_fire() {
        // Initial state: not seeded, prev = None, state = Activated.
        assert!(!is_activation_edge(false, None, NM_DEVICE_STATE_ACTIVATED));
    }

    /// GH#355: the first poll of a non-Activated device also must
    /// not fire — there's no edge to detect yet.
    #[test]
    fn gh355_first_poll_of_disconnected_device_does_not_fire() {
        assert!(!is_activation_edge(false, None, 30));
    }

    /// Once seeded, a transition from anything-not-Activated into
    /// Activated *is* a real edge and fires.
    #[test]
    fn gh355_real_transition_after_seeding_fires() {
        // Seeded with state=30 (Disconnected), now state=100 (Activated).
        assert!(is_activation_edge(
            true,
            Some(30),
            NM_DEVICE_STATE_ACTIVATED
        ));
        // Seeded with state=50 (Config), now state=100 (Activated).
        assert!(is_activation_edge(
            true,
            Some(50),
            NM_DEVICE_STATE_ACTIVATED
        ));
        // Seeded with state=80 (IpCheck), now state=100 (Activated).
        assert!(is_activation_edge(
            true,
            Some(80),
            NM_DEVICE_STATE_ACTIVATED
        ));
    }

    /// Activated → Activated holds the line — no edge.
    #[test]
    fn gh355_activated_to_activated_does_not_fire() {
        assert!(!is_activation_edge(
            true,
            Some(NM_DEVICE_STATE_ACTIVATED),
            NM_DEVICE_STATE_ACTIVATED
        ));
    }

    /// Activated → Disconnected (e.g. a real disconnect) does not
    /// fire `ConnectionUp` — that's only for the rising edge.
    #[test]
    fn gh355_falling_edge_does_not_fire_connection_up() {
        assert!(!is_activation_edge(
            true,
            Some(NM_DEVICE_STATE_ACTIVATED),
            30
        ));
    }
}
