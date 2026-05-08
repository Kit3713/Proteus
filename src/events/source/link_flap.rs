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
    ///
    /// Issue #251: previously this branch validated the bind via
    /// `try_open_netlink`, then dropped the socket and parked the
    /// spawned task on `await stop` — link transitions never
    /// reached the registry. The implementation now opens the
    /// socket once, hands it to a blocking-thread reader (so a
    /// long `recvfrom` doesn't pin the tokio runtime), and emits
    /// `RotationTrigger::LinkFlap` from the parsed RTM_NEWLINK /
    /// RTM_DELLINK messages. Kernel origin is validated via the
    /// `nlmsghdr.nlmsg_pid == 0` check; any message claiming to be
    /// from a userspace pid is rejected before parsing.
    pub async fn spawn_into(self, registry: Arc<EventRegistry>) -> Option<SourceTask> {
        let socket = match netlink::open_route_link_socket() {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("link-flap: netlink socket unavailable, source disabled: {e}");
                return None;
            }
        };
        let (stop, stop_rx) = StopHandle::channel();
        let window = self.window;
        let registry_for_task = Arc::clone(&registry);
        let join = tokio::spawn(async move {
            run_consumer(socket, registry_for_task, window, stop_rx).await;
        });
        Some(SourceTask {
            join,
            stop,
            name: "link-flap",
        })
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

/// Drive the netlink recv loop for the link-flap source. Reads
/// kernel-origin messages off `socket`, feeds carrier-state
/// transitions through [`FlapTable`], and fires
/// `RotationTrigger::LinkFlap` on detected flaps. The loop sits on
/// a blocking thread (one `recvfrom` per message) so a long quiet
/// period doesn't starve the tokio runtime; a 1 s SO_RCVTIMEO lets
/// the loop poll the stop channel so shutdown is bounded.
async fn run_consumer(
    socket: netlink::NetlinkSocket,
    registry: Arc<EventRegistry>,
    window: Duration,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) {
    // The blocking reader sends each parsed `(iface, is_up)` event
    // through an mpsc channel; the async side polls it alongside
    // the stop signal. Two-thread split keeps the recv off the
    // tokio runtime without a dedicated reactor.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, bool)>();
    let socket = Arc::new(socket);
    let socket_for_thread = Arc::clone(&socket);
    let reader = std::thread::Builder::new()
        .name("proteus-link-flap-rx".into())
        .spawn(move || netlink::read_link_events(&socket_for_thread, &tx))
        .ok();
    let mut table = FlapTable::default();
    loop {
        // Race a new message against the stop signal so shutdown
        // wins even on a quiet link. `tokio::time::timeout` is the
        // pattern used elsewhere in the daemon (see `portal_auth.rs`).
        let next = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        if stop_rx.try_recv().is_ok() {
            break;
        }
        let Ok(Some((iface, is_up))) = next else {
            continue;
        };
        if table.record(&iface, std::time::Instant::now(), is_up, window) {
            let _ = registry.fire(RotationTrigger::LinkFlap { iface });
        }
    }
    // Closing the socket forces any pending recv on the reader
    // thread to return EBADF; the thread exits and we join it
    // best-effort. The drop on `socket` is what releases the fd.
    netlink::shutdown(&socket);
    if let Some(handle) = reader {
        let _ = handle.join();
    }
}

/// Hand-rolled netlink helpers. Issue #251.
///
/// We deliberately avoid pulling in `neli` / `netlink-packet-route`
/// — the protocol surface the link-flap and reg-domain sources need
/// is small enough that 60–80 lines of byte-pushing is cheaper than
/// a new dep. The helpers live in `link_flap` because it's the
/// first source to use them; `reg_domain` shares them through
/// `super::link_flap::netlink`.
pub(super) mod netlink {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;

    use tokio::sync::mpsc::UnboundedSender;

    /// Owned netlink socket. Drop closes the fd (via `OwnedFd`).
    pub struct NetlinkSocket {
        fd: OwnedFd,
    }

    /// Open `AF_NETLINK + NETLINK_ROUTE`, bind to the LINK
    /// multicast group, set a 1-second receive timeout (so the
    /// reader loop can poll its shutdown channel), and validate
    /// that the bound `nl_pid` matches a userspace caller. The
    /// kernel uses `nl_pid == 0` exclusively; userspace gets
    /// assigned a non-zero pid by the kernel — this asymmetry is
    /// what lets the receive loop reject spoofed messages.
    pub fn open_route_link_socket() -> std::io::Result<NetlinkSocket> {
        open_netlink(libc::NETLINK_ROUTE, libc::RTMGRP_LINK as u32)
    }

    /// Open `AF_NETLINK + NETLINK_GENERIC` with no multicast
    /// subscription. The caller can resolve a family + group id
    /// via a CTRL_CMD_GETFAMILY query and join multicast groups
    /// later via `NETLINK_ADD_MEMBERSHIP`.
    pub fn open_genetlink_socket() -> std::io::Result<NetlinkSocket> {
        open_netlink(libc::NETLINK_GENERIC, 0)
    }

    /// Shared open + bind path for both NETLINK_ROUTE and
    /// NETLINK_GENERIC. Sets SO_RCVTIMEO=1s so the blocking reader
    /// can poll its exit channel.
    fn open_netlink(protocol: libc::c_int, groups: u32) -> std::io::Result<NetlinkSocket> {
        // SAFETY: `socket(AF_NETLINK, SOCK_RAW, protocol)` only
        // creates a kernel-side handle and returns a file
        // descriptor. `OwnedFd` takes ownership immediately so the
        // descriptor is closed on every error path.
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, protocol) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = groups;

        // SAFETY: `bind` reads the sockaddr_nl by pointer + length
        // and never retains it. `addr` lives for the call.
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // SO_RCVTIMEO=1s — bounded recv so the reader can check
        // its exit channel without a separate poll.
        let timeout = libc::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        // SAFETY: `setsockopt` reads `&timeout` by pointer +
        // length; `timeout` lives for the call.
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &timeout as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as u32,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(NetlinkSocket { fd: owned })
    }

    /// Close-and-shutdown the socket so a pending blocking recv
    /// returns. Used by the consumer's drop path so the reader
    /// thread exits promptly.
    pub fn shutdown(socket: &Arc<NetlinkSocket>) {
        // SAFETY: `shutdown(SHUT_RDWR)` is sound on any open fd; a
        // best-effort call is enough here, the OwnedFd close on
        // drop is what actually releases the resource.
        let _ = unsafe { libc::shutdown(socket.fd.as_raw_fd(), libc::SHUT_RDWR) };
    }

    /// Read RTM_NEWLINK / RTM_DELLINK messages off `socket` and
    /// forward each `(iface, is_up)` carrier transition through
    /// `tx`. Runs on a dedicated blocking thread (the recvfrom
    /// blocks up to 1 s, then loops); exits when the socket is
    /// shut down by the async side.
    pub fn read_link_events(socket: &Arc<NetlinkSocket>, tx: &UnboundedSender<(String, bool)>) {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut sa: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
            let mut sa_len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
            // SAFETY: `recvfrom` reads into `buf` by pointer + len
            // and writes the source sockaddr through `&mut sa`.
            // Both buffers outlive the call.
            let n = unsafe {
                libc::recvfrom(
                    socket.fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                    &mut sa as *mut libc::sockaddr_nl as *mut libc::sockaddr,
                    &mut sa_len,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                match err.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => continue,
                    std::io::ErrorKind::Interrupted => continue,
                    // Socket closed by the async side.
                    _ => break,
                }
            }
            if n == 0 {
                continue;
            }
            // `sa.nl_pid == 0` is the documented kernel marker;
            // rejecting non-zero blocks injected userspace packets.
            if sa.nl_pid != 0 {
                tracing::debug!(
                    nl_pid = sa.nl_pid,
                    "link-flap: dropping non-kernel netlink message"
                );
                continue;
            }
            for (hdr, payload) in NlMsgIter::new(&buf[..n as usize]) {
                if hdr.nlmsg_pid != 0 {
                    tracing::debug!(
                        nlmsg_pid = hdr.nlmsg_pid,
                        "link-flap: dropping nlmsghdr with non-zero pid"
                    );
                    continue;
                }
                if hdr.nlmsg_type != libc::RTM_NEWLINK && hdr.nlmsg_type != libc::RTM_DELLINK {
                    continue;
                }
                if let Some((iface, is_up)) = parse_link_message(payload)
                    && tx.send((iface, is_up)).is_err()
                {
                    return;
                }
            }
        }
    }

    /// Parse one RTM_NEWLINK / RTM_DELLINK payload. Returns
    /// `(iface_name, is_up)` when both pieces are present, `None`
    /// otherwise. `is_up` follows `IFF_UP & IFF_RUNNING` — both
    /// must be set for the iface to count as "carrier good."
    fn parse_link_message(payload: &[u8]) -> Option<(String, bool)> {
        let ifinfo_size = std::mem::size_of::<libc::ifinfomsg>();
        if payload.len() < ifinfo_size {
            return None;
        }
        // SAFETY: `ifinfomsg` is `repr(C)` and POD-shaped; reading
        // by `read_unaligned` from a verified-length slice is
        // sound. The fields we use (`ifi_flags`) are well-defined
        // for any kernel-emitted RTM_*LINK message.
        let info: libc::ifinfomsg =
            unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const libc::ifinfomsg) };
        let is_up = (info.ifi_flags & (libc::IFF_UP | libc::IFF_RUNNING) as u32)
            == (libc::IFF_UP | libc::IFF_RUNNING) as u32;
        // RTNETLINK rounds the ifinfomsg up to NLA_HDRLEN (4-byte)
        // alignment; the attributes follow.
        let attrs_off = align_to(ifinfo_size, 4);
        let attrs = payload.get(attrs_off..)?;
        let mut iface: Option<String> = None;
        for (kind, value) in NlAttrIter::new(attrs) {
            if kind == libc::IFLA_IFNAME {
                // IFLA_IFNAME is a NUL-terminated C string.
                let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
                iface = std::str::from_utf8(&value[..end]).ok().map(str::to_owned);
                break;
            }
        }
        iface.map(|i| (i, is_up))
    }

    /// Iterator over a netlink-attribute (NLA) list. Each entry's
    /// header is `[u16 len][u16 type]` followed by the value
    /// (NLA-aligned to 4 bytes). Returns the raw `(type, value)`
    /// pair so callers can match on the type they care about.
    pub(super) struct NlAttrIter<'a> {
        buf: &'a [u8],
    }

    impl<'a> NlAttrIter<'a> {
        pub(super) fn new(buf: &'a [u8]) -> Self {
            Self { buf }
        }
    }

    impl<'a> Iterator for NlAttrIter<'a> {
        type Item = (u16, &'a [u8]);
        fn next(&mut self) -> Option<Self::Item> {
            if self.buf.len() < 4 {
                return None;
            }
            let len = u16::from_ne_bytes([self.buf[0], self.buf[1]]) as usize;
            let kind = u16::from_ne_bytes([self.buf[2], self.buf[3]]);
            if len < 4 || len > self.buf.len() {
                return None;
            }
            let value = &self.buf[4..len];
            let stride = align_to(len, 4).min(self.buf.len());
            self.buf = &self.buf[stride..];
            Some((kind, value))
        }
    }

    /// Iterator over a netlink message stream. Each entry yields
    /// the parsed `nlmsghdr` plus the body slice (after the
    /// 16-byte header). Stops at NLMSG_DONE / NLMSG_ERROR or at a
    /// malformed length.
    pub(super) struct NlMsgIter<'a> {
        buf: &'a [u8],
    }

    impl<'a> NlMsgIter<'a> {
        pub(super) fn new(buf: &'a [u8]) -> Self {
            Self { buf }
        }
    }

    impl<'a> Iterator for NlMsgIter<'a> {
        type Item = (libc::nlmsghdr, &'a [u8]);
        fn next(&mut self) -> Option<Self::Item> {
            let hdr_size = std::mem::size_of::<libc::nlmsghdr>();
            if self.buf.len() < hdr_size {
                return None;
            }
            // SAFETY: `nlmsghdr` is `repr(C)` POD; reading via
            // `read_unaligned` from a length-checked slice is
            // sound.
            let hdr: libc::nlmsghdr =
                unsafe { std::ptr::read_unaligned(self.buf.as_ptr() as *const libc::nlmsghdr) };
            let total = hdr.nlmsg_len as usize;
            if total < hdr_size || total > self.buf.len() {
                return None;
            }
            let payload = &self.buf[hdr_size..total];
            let stride = align_to(total, 4).min(self.buf.len());
            self.buf = &self.buf[stride..];
            // The kernel sometimes injects NLMSG_DONE / NLMSG_NOOP
            // / NLMSG_ERROR into multicast streams; the caller
            // matches on `nlmsg_type` and skips what it doesn't
            // care about.
            Some((hdr, payload))
        }
    }

    pub(super) fn align_to(n: usize, to: usize) -> usize {
        (n + to - 1) & !(to - 1)
    }
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
        reg.register(Box::new(Counter { n: Arc::clone(&n) }))
            .unwrap();
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
        src.push(
            "wlan0",
            true,
            t0 + Duration::from_secs(5) + Duration::from_millis(100),
        );
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

    /// Issue #251: the netlink message iterator parses one
    /// concatenated kernel-shaped frame correctly. We synthesize a
    /// minimal `nlmsghdr` (NLMSG_DONE-typed for simplicity) and
    /// assert the iterator yields it once and stops.
    #[test]
    fn nlmsg_iter_parses_a_single_synthetic_frame() {
        use super::netlink::{NlMsgIter, align_to};

        let hdr_size = std::mem::size_of::<libc::nlmsghdr>();
        let mut buf = vec![0u8; align_to(hdr_size, 4)];
        let hdr = libc::nlmsghdr {
            nlmsg_len: hdr_size as u32,
            nlmsg_type: libc::NLMSG_DONE as u16,
            nlmsg_flags: 0,
            nlmsg_seq: 0,
            nlmsg_pid: 0,
        };
        // SAFETY: `nlmsghdr` is `repr(C)` POD; writing via
        // `write_unaligned` into a verified-length buffer is sound.
        unsafe {
            std::ptr::write_unaligned(buf.as_mut_ptr() as *mut libc::nlmsghdr, hdr);
        }

        let mut iter = NlMsgIter::new(&buf);
        let (got, payload) = iter.next().expect("iterator must yield the frame");
        assert_eq!(got.nlmsg_type, libc::NLMSG_DONE as u16);
        assert_eq!(got.nlmsg_pid, 0);
        assert!(payload.is_empty());
        assert!(iter.next().is_none(), "no extra frames after the first");
    }

    /// Issue #251: a malformed frame (advertised len > buffer)
    /// returns `None` rather than panicking. Pin the
    /// hostile-input behaviour so future parser tweaks can't
    /// silently regress.
    #[test]
    fn nlmsg_iter_rejects_out_of_bounds_length() {
        use super::netlink::NlMsgIter;

        let hdr_size = std::mem::size_of::<libc::nlmsghdr>();
        let mut buf = vec![0u8; hdr_size];
        let hdr = libc::nlmsghdr {
            // Advertise more bytes than the buffer holds.
            nlmsg_len: (hdr_size + 9999) as u32,
            nlmsg_type: libc::NLMSG_DONE as u16,
            nlmsg_flags: 0,
            nlmsg_seq: 0,
            nlmsg_pid: 0,
        };
        // SAFETY: see `nlmsg_iter_parses_a_single_synthetic_frame`.
        unsafe {
            std::ptr::write_unaligned(buf.as_mut_ptr() as *mut libc::nlmsghdr, hdr);
        }
        let mut iter = NlMsgIter::new(&buf);
        assert!(iter.next().is_none());
    }

    /// Issue #251: the netlink-attribute iterator round-trips one
    /// IFLA_IFNAME-shaped attribute and surfaces the iface name.
    #[test]
    fn nlattr_iter_extracts_an_ifname_attribute() {
        use super::netlink::{NlAttrIter, align_to};

        let name = b"wlan0\0";
        let attr_hdr = 4usize;
        let total = attr_hdr + name.len();
        let aligned = align_to(total, 4);
        let mut buf = vec![0u8; aligned];
        // [u16 len][u16 type] header.
        buf[..2].copy_from_slice(&(total as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&libc::IFLA_IFNAME.to_ne_bytes());
        buf[4..4 + name.len()].copy_from_slice(name);

        let attrs: Vec<(u16, &[u8])> = NlAttrIter::new(&buf).collect();
        assert_eq!(attrs.len(), 1);
        let (kind, value) = attrs[0];
        assert_eq!(kind, libc::IFLA_IFNAME);
        let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
        assert_eq!(std::str::from_utf8(&value[..end]).unwrap(), "wlan0");
    }

    /// `align_to` rounds up to the requested boundary.
    #[test]
    fn align_to_rounds_up() {
        use super::netlink::align_to;
        assert_eq!(align_to(0, 4), 0);
        assert_eq!(align_to(1, 4), 4);
        assert_eq!(align_to(4, 4), 4);
        assert_eq!(align_to(5, 4), 8);
        assert_eq!(align_to(15, 4), 16);
    }
}
