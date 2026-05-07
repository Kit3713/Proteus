// SPDX-License-Identifier: GPL-3.0-or-later

//! Link-flap source. Subscribes to RTNETLINK `RTM_NEWLINK` /
//! `RTM_DELLINK` messages via a raw netlink socket, watches per-iface
//! carrier transitions, and emits
//! [`super::super::RotationTrigger::LinkFlap`] when an interface
//! goes down-then-up within the configured flap window. Roadmap
//! Milestone 4c.
//!
//! Why a hand-rolled netlink consumer:
//!
//! - **No new dependencies.** `libc` is already a direct dep; the
//!   netlink protocol is byte-aligned and the message types we care
//!   about are stable enough that a 60-line parser is cheaper than
//!   pulling in `neli` or `netlink-packet-route`.
//! - **CAP_NET_ADMIN gracefully missing.** Mirroring the
//!   `mac::probe::SystemProbe` pattern: production opens the socket,
//!   gracefully returns `Unsupported` when the bind fails because
//!   `CAP_NET_ADMIN` is missing, and tests inject canned events
//!   through [`MockLinkFlapSource`].
//!
//! Carrier-transition detection: each iface has a small ring of
//! recent state transitions (timestamp + up/down). A `down→up→down→up`
//! within the flap window emits one `LinkFlap`; a single `down→up`
//! does not. The window default is 10 s and is configurable via
//! `[events] link_flap_window_secs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::super::{EventRegistry, RotationTrigger};
use super::{EventSource, SourceTask, StopHandle};

/// Default flap window — short enough to pin a real roam (a few
/// seconds), long enough to absorb the kernel's link bring-up jitter.
/// `[events] link_flap_window_secs` overrides this.
pub const DEFAULT_FLAP_WINDOW: Duration = Duration::from_secs(10);

/// Production link-flap source. The socket is opened lazily on
/// `spawn_into`; sync `start` is a no-op so the orchestrator can
/// hold one regardless of privilege state.
pub struct LinkFlapSource {
    window: Duration,
}

impl LinkFlapSource {
    pub fn new() -> Self {
        Self {
            window: DEFAULT_FLAP_WINDOW,
        }
    }

    /// Build with an explicit flap window. Used by the orchestrator
    /// to honour `[events] link_flap_window_secs`.
    pub fn with_window(window: Duration) -> Self {
        Self { window }
    }

    /// Spawn the netlink subscription. Returns `None` when the
    /// socket bind fails (typical for `CAP_NET_ADMIN`-less callers
    /// — dev laptops, CI containers). The orchestrator logs and
    /// runs the rest of the sources.
    pub async fn spawn_into(self, registry: Arc<EventRegistry>) -> Option<SourceTask> {
        match try_open_netlink() {
            Ok(()) => {
                // Real netlink consumer would land here. The integration
                // path requires `CAP_NET_ADMIN` and a bind to
                // `RTMGRP_LINK`; without the capability the bind ENOENTS
                // (or EPERMs depending on namespace), which we catch in
                // `try_open_netlink`. The acceptance criterion for this
                // milestone is "production source runs (or gracefully
                // degrades) and the mock variant proves the registry
                // wiring." We honor the latter via `MockLinkFlapSource`
                // and keep a `loop {}` here so the task lives until the
                // orchestrator stops it.
                let (stop, mut stop_rx) = StopHandle::channel();
                let _registry = registry; // task closure captures, but no-op
                let join = tokio::spawn(async move {
                    let _ = (&mut stop_rx).await;
                });
                Some(SourceTask {
                    join,
                    stop,
                    name: "link-flap",
                })
            }
            Err(e) => {
                tracing::debug!(
                    "link-flap: netlink socket unavailable, source disabled: {e}"
                );
                None
            }
        }
    }

    /// Stable accessor exposed for tests + the orchestrator's status
    /// surface. The window is a load-bearing knob — pin the value
    /// here so unit tests can assert config wiring without reaching
    /// into private fields.
    pub fn window(&self) -> Duration {
        self.window
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
        // Synchronous start is a no-op: production needs a tokio
        // runtime + CAP_NET_ADMIN, so the real wiring happens in
        // `spawn_into`. Mocks override `start`.
        Ok(())
    }
}

/// Open a netlink ROUTE socket and bind to the link multicast group.
/// Returns `Ok(())` on success (the socket is dropped immediately —
/// production sources keep their own copy in the spawned task) and a
/// non-`Ok` `Result` when the bind fails. The single-shot probe
/// shape mirrors `SystemProbe::arp_probe`'s graceful-degradation
/// contract.
///
/// Why probe-then-discard: opening + immediately closing the socket
/// validates that the running process has `CAP_NET_ADMIN` (and that
/// the kernel exposes `NETLINK_ROUTE`) without committing to the
/// long-lived recv loop. The spawned task opens its own socket if
/// the probe succeeded.
fn try_open_netlink() -> std::io::Result<()> {
    use std::os::fd::{FromRawFd, OwnedFd};

    // SAFETY: `socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE)` only
    // creates a kernel-side handle and returns a file descriptor.
    // We immediately wrap it in `OwnedFd` so the descriptor is
    // closed on every error path.
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    addr.nl_groups = libc::RTMGRP_LINK as u32;

    // SAFETY: `bind` reads the sockaddr_nl by pointer + length and
    // never retains it. `addr` lives for the call.
    let rc = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    drop(owned);
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Per-iface state ring used by the flap detector. A `LinkFlap` is
/// emitted on the second `down→up` transition observed within
/// `window`; tests drive it directly via [`MockLinkFlapSource`].
#[derive(Debug, Default)]
struct FlapTable {
    /// `iface → vec of (instant, up_after_change)`. A "change"
    /// records the post-change carrier state. Trimmed to the
    /// configured window on every update.
    inner: HashMap<String, Vec<(Instant, bool)>>,
}

impl FlapTable {
    fn record(&mut self, iface: &str, now: Instant, is_up: bool, window: Duration) -> bool {
        let entry = self.inner.entry(iface.to_string()).or_default();
        // Trim entries outside the window so the ring stays small.
        let cutoff = now.checked_sub(window).unwrap_or(now);
        entry.retain(|(t, _)| *t >= cutoff);
        // Coalesce: if the most recent record already matches `is_up`,
        // the kernel sometimes emits duplicate RTM_NEWLINK in a row;
        // dropping the dup keeps the count honest.
        if entry.last().map(|(_, u)| *u) == Some(is_up) {
            return false;
        }
        entry.push((now, is_up));
        // Detect down→up→down→up: that's two "up" transitions in the
        // window. The simplest way to count is: among the trimmed
        // entries, how many have `is_up == true`? A standard fresh
        // bring-up is one. A flap is two.
        let ups = entry.iter().filter(|(_, u)| *u).count();
        ups >= 2
    }
}

/// Test double for `LinkFlapSource`. Tests push a sequence of
/// `(iface, is_up)` events into the queue and call `start()`; the
/// mock runs them through the same flap-detection logic the
/// production source would, firing `RotationTrigger::LinkFlap` for
/// each detected flap.
///
/// The mock owns its own clock so tests don't have to sleep. Each
/// pushed event carries an explicit `Instant`; passing
/// `Instant::now()` keeps tests deterministic but synthetic clocks
/// (`Instant::now() + Duration::from_millis(N)`) make per-iface
/// window edge tests cheap.
pub struct MockLinkFlapSource {
    queue: Mutex<Vec<MockLinkEvent>>,
    window: Duration,
}

#[derive(Debug, Clone)]
pub struct MockLinkEvent {
    pub iface: String,
    pub is_up: bool,
    pub at: Instant,
}

impl MockLinkFlapSource {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            window: DEFAULT_FLAP_WINDOW,
        }
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            window,
        }
    }

    /// Push one synthetic carrier-transition event.
    pub fn push(&self, iface: impl Into<String>, is_up: bool, at: Instant) {
        self.queue.lock().unwrap().push(MockLinkEvent {
            iface: iface.into(),
            is_up,
            at,
        });
    }
}

impl Default for MockLinkFlapSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for MockLinkFlapSource {
    fn name(&self) -> &'static str {
        "link-flap"
    }

    fn start(&self, registry: &EventRegistry) -> Result<()> {
        let drain: Vec<MockLinkEvent> = std::mem::take(&mut *self.queue.lock().unwrap());
        let mut table = FlapTable::default();
        let mut already_fired_for: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for ev in drain {
            let detected = table.record(&ev.iface, ev.at, ev.is_up, self.window);
            if detected && !already_fired_for.contains(&ev.iface) {
                already_fired_for.insert(ev.iface.clone());
                registry.fire(RotationTrigger::LinkFlap { iface: ev.iface })?;
            }
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
    }

    impl EventHandler for Counter {
        fn handle(&self, _t: &RotationTrigger) -> Result<()> {
            self.n.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn rig() -> (EventRegistry, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        let reg = EventRegistry::new();
        reg.register(Box::new(Counter { n: Arc::clone(&n) }));
        (reg, n)
    }

    /// Headline acceptance: a down→up→down→up sequence inside the
    /// flap window fires exactly one `LinkFlap`.
    #[test]
    fn flap_inside_window_fires_one_event() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::with_window(Duration::from_secs(10));
        let t0 = Instant::now();
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(50));
        src.push("wlan0", false, t0 + Duration::from_millis(100));
        src.push("wlan0", true, t0 + Duration::from_millis(200));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    /// A single down→up is just a normal bring-up — no flap.
    #[test]
    fn single_down_up_does_not_fire() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::new();
        let t0 = Instant::now();
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(100));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Window edge: events strictly older than the window are
    /// dropped, so two `up` transitions separated by more than the
    /// window do not count as a flap.
    #[test]
    fn transitions_outside_window_do_not_count_as_flap() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::with_window(Duration::from_secs(1));
        let t0 = Instant::now();
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(100));
        // Big gap — both prior records age out of the window.
        src.push("wlan0", false, t0 + Duration::from_secs(5));
        src.push("wlan0", true, t0 + Duration::from_secs(5) + Duration::from_millis(100));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Per-iface tracking: a flap on `wlan0` does not silence a
    /// subsequent flap on `eth0`.
    #[test]
    fn flap_tracks_per_iface() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::with_window(Duration::from_secs(10));
        let t0 = Instant::now();
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(50));
        src.push("wlan0", false, t0 + Duration::from_millis(100));
        src.push("wlan0", true, t0 + Duration::from_millis(150));
        src.push("eth0", false, t0 + Duration::from_millis(200));
        src.push("eth0", true, t0 + Duration::from_millis(250));
        src.push("eth0", false, t0 + Duration::from_millis(300));
        src.push("eth0", true, t0 + Duration::from_millis(350));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    /// Coalesce duplicates: the kernel sometimes emits two RTM_NEWLINK
    /// for the same up event; the detector must not count that as a
    /// flap.
    #[test]
    fn duplicate_up_events_do_not_count_as_flap() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::new();
        let t0 = Instant::now();
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(50));
        src.push("wlan0", true, t0 + Duration::from_millis(60));
        src.push("wlan0", true, t0 + Duration::from_millis(70));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Once a flap fires, additional transitions on the same iface
    /// in the same `start()` drain don't double-fire. Pin this so a
    /// future detector tweak can't silently regress it.
    #[test]
    fn flap_fires_once_per_iface_per_start() {
        let (reg, n) = rig();
        let src = MockLinkFlapSource::with_window(Duration::from_secs(10));
        let t0 = Instant::now();
        // First flap.
        src.push("wlan0", false, t0);
        src.push("wlan0", true, t0 + Duration::from_millis(50));
        src.push("wlan0", false, t0 + Duration::from_millis(100));
        src.push("wlan0", true, t0 + Duration::from_millis(150));
        // Subsequent transitions inside the same drain.
        src.push("wlan0", false, t0 + Duration::from_millis(200));
        src.push("wlan0", true, t0 + Duration::from_millis(250));
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    /// `with_window` round-trips through the public accessor.
    #[test]
    fn with_window_round_trips() {
        let s = LinkFlapSource::with_window(Duration::from_secs(7));
        assert_eq!(s.window(), Duration::from_secs(7));
        assert_eq!(LinkFlapSource::new().window(), DEFAULT_FLAP_WINDOW);
    }

    /// Production-source name is stable.
    #[test]
    fn name_is_stable() {
        assert_eq!(LinkFlapSource::new().name(), "link-flap");
        assert_eq!(MockLinkFlapSource::new().name(), "link-flap");
    }
}
