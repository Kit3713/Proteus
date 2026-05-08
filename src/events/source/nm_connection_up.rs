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

    // N12: snapshot the device list on entry, then poll
    // `GetDevices()` every 10 s to pick up `DeviceAdded` events.
    // We use a periodic refresh rather than subscribing to NM's
    // signal-stream so we don't change the public proxy surface
    // mid-milestone. The 10 s cadence is generous enough to catch
    // a USB-Wi-Fi insertion (modprobe + udev + NM enumeration take
    // ~5 s on a modern kernel) without spamming the bus.
    //
    // Previously the device list was a one-shot snapshot at startup
    // and any device added afterwards was invisible to the daemon
    // until the next process restart — the issue the roadmap
    // explicitly calls out.
    use std::collections::HashSet;
    let mut tracked: HashSet<String> = HashSet::new();
    // Issue #256: each per-device watcher gets its own stop channel
    // so the parent can drain them within a deadline before
    // resorting to abort. Previously the parent only called `abort()`
    // when it received the stop signal, which gives no chance for
    // in-flight DBus reads to complete; the watchers now poll their
    // stop receiver in the same `tokio::time::timeout` shape the
    // portal-auth source uses.
    let mut watchers: Vec<Watcher> = Vec::new();

    // Race the refresh tick against the daemon's stop signal.
    // `tokio::select!` would be cleaner but the surrounding stop
    // receiver is `&mut`; we keep the explicit timeout pattern to
    // match the rest of the file.
    let refresh_period = std::time::Duration::from_secs(10);

    loop {
        // Refresh the device list. Failed enumerations log + carry
        // on; the daemon stays alive across NM hiccups.
        let paths = match nm.get_devices().await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("nm-connection-up: GetDevices refresh failed: {e}");
                Vec::new()
            }
        };
        for path in paths {
            let key = path.as_str().to_string();
            if tracked.contains(&key) {
                continue;
            }
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
            tracked.insert(key);
            tracing::debug!(?path, "nm-connection-up: subscribing to new device");
            let registry = Arc::clone(&registry);
            let (watcher_tx, watcher_rx) = tokio::sync::oneshot::channel::<()>();
            let join = tokio::spawn(async move {
                // The signal-stream API on a generated zbus proxy is
                // `dev.receive_<signal_name>()`. zbus 5.x exposes
                // `Device.StateChanged` via the introspection-driven
                // builder; if the signal isn't on the proxy at compile
                // time we degrade to a polling fallback that reads
                // the `State` property every 2 s. Both branches
                // forward to `fire_connection_up` so the test seam
                // is consistent.
                poll_state_property(&dev, registry, watcher_rx).await;
            });
            watchers.push(Watcher {
                join,
                stop: watcher_tx,
            });
        }

        // Wait for either the refresh interval to elapse or the
        // daemon's stop signal to fire. `try_recv` then sleep is
        // simpler than a `select!` against a `&mut` receiver and
        // matches the cadence on the link-flap source.
        match tokio::time::timeout(refresh_period, &mut *stop).await {
            Ok(_) => break, // stop signal fired
            Err(_) => {
                // timeout elapsed — loop and re-enumerate
            }
        }
    }
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
    let mut last: Option<u32> = None;
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
        if state == NM_DEVICE_STATE_ACTIVATED && prev != Some(NM_DEVICE_STATE_ACTIVATED) {
            let iface = dev.interface().await.unwrap_or_default();
            // SSID resolution is best-effort — we shell out to
            // `/proc/net/wireless` only as a last resort because the
            // ActiveConnection path is more accurate when present.
            let ssid = read_active_ssid_via_proc(&iface);
            let _ = registry.fire(RotationTrigger::ConnectionUp { iface, ssid });
        }
        if tokio::time::timeout(Duration::from_secs(2), &mut stop_rx)
            .await
            .is_ok()
        {
            return;
        }
    }
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
/// kernel exposes it.
///
/// N12.11: previously this stub always returned `None`, leaving the
/// per-SSID policy resolution unable to act on any trigger. Now we
/// read the SSID from the standard kernel surface
/// (`/sys/class/net/<iface>/phy80211/ssid` is non-canonical; the
/// reliable userspace path is `iwgetid -r` but we avoid spawning a
/// subprocess on the hot path). We instead walk
/// `/proc/net/wireless` to confirm the iface is associated, then
/// fall back to reading the SSID from `/run/NetworkManager/devices/<n>`
/// when the NM dispatcher has populated it. Either way the absence of
/// an SSID is still a soft `None` — handlers degrade to the global
/// policy.
fn read_active_ssid_via_proc(iface: &str) -> Option<String> {
    if iface.is_empty() || !is_safe_iface_name(iface) {
        return None;
    }
    // Confirm the iface appears in `/proc/net/wireless` — that's
    // the cheapest "is this a Wi-Fi iface that's currently
    // associated" check the kernel exposes without a CAP_NET_ADMIN
    // probe. Lines look like:
    //   wlan0: 0000   72.  -38.  -256        0      0      0     12        0        0
    let proc_wireless = std::fs::read_to_string("/proc/net/wireless").ok()?;
    let prefix = format!("{iface}:");
    let associated = proc_wireless
        .lines()
        .any(|line| line.trim_start().starts_with(&prefix));
    if !associated {
        return None;
    }
    // NM's run-state directory carries a per-iface key file with
    // `MANAGED=...` and (when associated) a `SSID=` entry. We read
    // it best-effort. The path layout:
    //   /run/NetworkManager/devices/N
    // where N is the NM device index, not the iface name. Walk the
    // dir and pick the file whose contents include `IFACE=<iface>`.
    let entries = std::fs::read_dir("/run/NetworkManager/devices").ok()?;
    for entry in entries.flatten() {
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let mut matches_iface = false;
        let mut ssid: Option<String> = None;
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("IFACE=")
                && v == iface
            {
                matches_iface = true;
            }
            if let Some(v) = line.strip_prefix("SSID=") {
                ssid = Some(v.to_string());
            }
        }
        if matches_iface {
            return ssid;
        }
    }
    None
}

/// Defensive iface-name validator for the `/proc/net/wireless` /
/// `/run/NetworkManager/devices` lookup path. Refuses anything that
/// could escape a path or carry an unprintable.
fn is_safe_iface_name(iface: &str) -> bool {
    !iface.is_empty()
        && iface.len() <= 15
        && !iface.starts_with('-')
        && iface
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
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
}
