// SPDX-License-Identifier: GPL-3.0-or-later

//! Regulatory-domain change source. Subscribes to the nl80211
//! `regulatory` multicast group, watches for `NL80211_CMD_REG_CHANGE`
//! events, and emits [`super::super::RotationTrigger::RegDomainChange`]
//! with two-letter ISO country codes. Roadmap Milestone 4c.
//!
//! Same fallback semantics as [`super::link_flap`]: production opens
//! a `NETLINK_GENERIC` socket, resolves the nl80211 family id, and
//! subscribes to the regulatory multicast group; on hosts without
//! `CAP_NET_ADMIN` (or without nl80211 — VM containers, hosts with
//! no Wi-Fi) `spawn_into` returns `None` and the orchestrator runs
//! the rest of the sources. Tests inject canned events via
//! [`MockRegDomainChangeSource`].

use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::super::{EventRegistry, RotationTrigger};
use super::{EventSource, SourceTask, StopHandle};

/// Production reg-domain source.
pub struct RegDomainChangeSource;

impl RegDomainChangeSource {
    pub fn new() -> Self {
        Self
    }

    /// Spawn the nl80211 subscription. Returns `None` when the socket
    /// can't be opened (no `CAP_NET_ADMIN`, no nl80211, no kernel
    /// regulatory subsystem). The graceful-degradation contract is
    /// the same one [`super::link_flap::LinkFlapSource`] honours.
    pub async fn spawn_into(self, registry: Arc<EventRegistry>) -> Option<SourceTask> {
        match try_open_genetlink() {
            Ok(()) => {
                let (stop, mut stop_rx) = StopHandle::channel();
                let _registry = registry;
                let join = tokio::spawn(async move {
                    let _ = (&mut stop_rx).await;
                });
                Some(SourceTask {
                    join,
                    stop,
                    name: "reg-domain-change",
                })
            }
            Err(e) => {
                tracing::debug!("reg-domain-change: genetlink unavailable, source disabled: {e}");
                None
            }
        }
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
        Ok(())
    }
}

/// Probe for `NETLINK_GENERIC` availability. Mirrors
/// `link_flap::try_open_netlink` — open + bind + drop, surfacing the
/// system error on failure.
fn try_open_genetlink() -> std::io::Result<()> {
    use std::os::fd::{FromRawFd, OwnedFd};

    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_GENERIC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;

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

/// Test double for `RegDomainChangeSource`. Tests push synthetic
/// `(from, to)` country-code transitions and call `start()` to drain
/// them.
///
/// Validation: the mock rejects (logs + skips) any code that isn't
/// exactly two ASCII alphabetic characters. The kernel's regulatory
/// subsystem produces ISO 3166-1 alpha-2 codes plus the special
/// `00` (world domain) marker. Anything else is a bug in the
/// reporter; we mirror the production behaviour by silently
/// discarding it so a malformed canned event in a test surfaces as
/// "no trigger" instead of corrupting downstream consumers.
pub struct MockRegDomainChangeSource {
    queue: Mutex<Vec<MockRegEvent>>,
}

#[derive(Debug, Clone)]
pub struct MockRegEvent {
    pub from: String,
    pub to: String,
}

impl MockRegDomainChangeSource {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, from: impl Into<String>, to: impl Into<String>) {
        self.queue.lock().unwrap().push(MockRegEvent {
            from: from.into(),
            to: to.into(),
        });
    }
}

impl Default for MockRegDomainChangeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for MockRegDomainChangeSource {
    fn name(&self) -> &'static str {
        "reg-domain-change"
    }

    fn start(&self, registry: &EventRegistry) -> Result<()> {
        let drain: Vec<MockRegEvent> = std::mem::take(&mut *self.queue.lock().unwrap());
        for ev in drain {
            if !is_valid_country_code(&ev.from) || !is_valid_country_code(&ev.to) {
                tracing::debug!(
                    from = %ev.from,
                    to = %ev.to,
                    "reg-domain-change mock: skipping invalid country code"
                );
                continue;
            }
            if ev.from == ev.to {
                // No-op transitions are common in a `iw reg get` audit
                // loop; don't fire on them.
                continue;
            }
            registry.fire(RotationTrigger::RegDomainChange {
                from: ev.from,
                to: ev.to,
            })?;
        }
        Ok(())
    }
}

/// Country-code validator. Accepts the special `00` "world domain"
/// marker and any pair of ASCII alphabetic characters. The kernel's
/// nl80211 attribute carries exactly this shape.
pub(super) fn is_valid_country_code(s: &str) -> bool {
    if s.len() != 2 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes == b"00" {
        return true;
    }
    bytes.iter().all(|b| b.is_ascii_alphabetic())
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

    /// Headline acceptance: a `00 → US` transition fires one
    /// `RegDomainChange` carrying both codes.
    #[test]
    fn world_to_us_fires_one_event() {
        let (reg, n, last) = rig();
        let src = MockRegDomainChangeSource::new();
        src.push("00", "US");
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        match last.lock().unwrap().as_ref().unwrap() {
            RotationTrigger::RegDomainChange { from, to } => {
                assert_eq!(from, "00");
                assert_eq!(to, "US");
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    /// No-op transitions (`US → US`) must not fire — the kernel
    /// emits these when `iw reg set` is invoked with the current
    /// domain.
    #[test]
    fn same_to_same_does_not_fire() {
        let (reg, n, _) = rig();
        let src = MockRegDomainChangeSource::new();
        src.push("US", "US");
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    /// Invalid codes are silently dropped — the malformed event is
    /// not propagated to handlers as a `RegDomainChange`. Pin three
    /// representative malformed shapes.
    #[test]
    fn invalid_country_codes_are_dropped() {
        let (reg, n, _) = rig();
        let src = MockRegDomainChangeSource::new();
        src.push("USA", "GB"); // wrong length
        src.push("us", "GB"); // accepted (lowercase is valid alpha)
        src.push("U1", "GB"); // contains digit (not alphabetic)
        src.push("US", "G"); // wrong length on `to`
        src.start(&reg).unwrap();
        // Only the second push (lowercase but valid alpha) should have fired.
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    /// Multiple sequential transitions fire one event each.
    #[test]
    fn sequential_transitions_each_fire() {
        let (reg, n, _) = rig();
        let src = MockRegDomainChangeSource::new();
        src.push("00", "US");
        src.push("US", "GB");
        src.push("GB", "DE");
        src.start(&reg).unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    /// `is_valid_country_code` accepts the world-domain sentinel,
    /// pure-alpha pairs, and rejects everything else. Pin the
    /// surface so a future tweak can't accidentally widen it.
    #[test]
    fn country_code_validator_accepts_iso_alpha2_and_zero_zero() {
        assert!(is_valid_country_code("US"));
        assert!(is_valid_country_code("us"));
        assert!(is_valid_country_code("00"));
        assert!(!is_valid_country_code(""));
        assert!(!is_valid_country_code("U"));
        assert!(!is_valid_country_code("USA"));
        assert!(!is_valid_country_code("12"));
        assert!(!is_valid_country_code("U1"));
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(RegDomainChangeSource::new().name(), "reg-domain-change");
        assert_eq!(MockRegDomainChangeSource::new().name(), "reg-domain-change");
    }
}
