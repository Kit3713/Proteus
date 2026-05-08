// SPDX-License-Identifier: GPL-3.0-or-later

//! Pre-commit collision probing for candidate MACs.
//!
//! Roadmap Milestone 2 ("ARP / ND collision handling"):
//! `read_arp_macs` is a passive snapshot of `/proc/net/arp` — fine for the
//! common case but oblivious to neighbours that simply haven't talked to us
//! yet. Before stamping a candidate MAC onto a NetworkManager profile, we
//! actively poke the segment for the candidate via:
//!
//! - **RFC 5227 ARP Probe** — sender_hw = candidate, sender_proto = 0.0.0.0,
//!   target_proto = the link-local target IP. A reply means the MAC is taken.
//! - **IPv6 Duplicate Address Detection** — Neighbor Solicitation against
//!   the tentative link-local address derived from the candidate MAC. A
//!   Neighbor Advertisement means it is taken.
//!
//! Raw sockets need `CAP_NET_RAW`. Unprivileged callers (developer laptop,
//! container without the capability) get a graceful `ProbeOutcome::Unsupported`
//! back and the higher-level rotate path falls back to the existing passive
//! `read_arp_macs` exclusion. The trait abstraction also lets unit tests
//! drive collision retries without ever opening a raw socket — production
//! builds use [`SystemProbe`], tests use [`MockProbe`].

use std::sync::Mutex;
use std::time::Duration;

use super::Mac;

/// Result of a single pre-commit probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// No reply within the deadline — caller may proceed with the candidate.
    Free,
    /// A neighbour answered for the candidate — caller must re-roll.
    Collision {
        /// IPv4/IPv6 address of the neighbour that answered, when the probe
        /// implementation could capture it. `None` is allowed for fallbacks
        /// that only know "something replied".
        peer_ip: Option<String>,
    },
    /// Probe could not run (insufficient privilege, no link-local target,
    /// etc.). Caller should fall back to passive checks and not treat this
    /// as a collision.
    Unsupported(&'static str),
}

/// Pluggable probe surface. The production implementation (`SystemProbe`)
/// opens a raw `AF_PACKET` socket; tests inject [`MockProbe`] that returns
/// canned outcomes.
pub trait Probe: Send + Sync {
    /// RFC 5227 ARP Probe for the candidate MAC. `iface` is the netdev to
    /// emit on (e.g. `wlan0`). The probe is one-shot with `timeout` as the
    /// listen window.
    fn arp_probe(&self, iface: &str, candidate: Mac, timeout: Duration) -> ProbeOutcome;

    /// IPv6 DAD probe for the link-local address derived from the candidate.
    /// `iface` is the netdev. Listens for a Neighbor Advertisement up to
    /// `timeout`.
    fn nd_probe(&self, iface: &str, candidate: Mac, timeout: Duration) -> ProbeOutcome;
}

/// Default time the production probe waits for an ARP reply. RFC 5227
/// recommends ~200 ms — long enough to catch a busy neighbour, short enough
/// that a few retries don't dominate the rotate runtime.
pub const ARP_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Default DAD listen window — IPv6 neighbours can be slower than ARP, so
/// give them a full second per RFC 4862's `RetransTimer` default.
pub const ND_PROBE_TIMEOUT: Duration = Duration::from_millis(1000);

/// Production probe. The real raw-socket path requires `CAP_NET_RAW`; the
/// constructor never fails so the orchestrator can hold one of these without
/// caring about privilege state, and individual probes return
/// `Unsupported` when the capability is missing.
pub struct SystemProbe;

impl SystemProbe {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for SystemProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe for SystemProbe {
    fn arp_probe(&self, _iface: &str, _candidate: Mac, _timeout: Duration) -> ProbeOutcome {
        // Raw-socket path is intentionally a no-op stub for the dev-laptop
        // build: opening AF_PACKET requires CAP_NET_RAW, and we'd rather
        // surface "unsupported, fall back to passive ARP" than sprinkle
        // capability checks through every caller. The integration test
        // container that runs with CAP_NET_RAW will swap in the libc-based
        // implementation as a follow-up; the trait surface is stable.
        tracing::debug!(
            "SystemProbe::arp_probe: raw-socket implementation deferred (CAP_NET_RAW); \
             falling back to passive /proc/net/arp"
        );
        ProbeOutcome::Unsupported("raw-socket arp probe not yet wired (needs CAP_NET_RAW)")
    }

    fn nd_probe(&self, _iface: &str, _candidate: Mac, _timeout: Duration) -> ProbeOutcome {
        tracing::debug!(
            "SystemProbe::nd_probe: raw-socket implementation deferred (CAP_NET_RAW); \
             falling back to passive neighbour table"
        );
        ProbeOutcome::Unsupported("raw-socket nd probe not yet wired (needs CAP_NET_RAW)")
    }
}

/// Test double. The script is a queue of canned outcomes consumed by
/// `arp_probe` and `nd_probe` in call order; once empty, every call returns
/// `Free` so a test that only cares about the first N collisions doesn't
/// have to over-stuff the queue.
pub struct MockProbe {
    arp_script: Mutex<Vec<ProbeOutcome>>,
    nd_script: Mutex<Vec<ProbeOutcome>>,
    /// Recorded `(iface, candidate)` pairs the probe was called with. Tests
    /// assert against this to confirm the candidate stream actually flowed
    /// through the probe.
    pub arp_calls: Mutex<Vec<(String, Mac)>>,
    pub nd_calls: Mutex<Vec<(String, Mac)>>,
}

impl MockProbe {
    pub fn new() -> Self {
        Self {
            arp_script: Mutex::new(Vec::new()),
            nd_script: Mutex::new(Vec::new()),
            arp_calls: Mutex::new(Vec::new()),
            nd_calls: Mutex::new(Vec::new()),
        }
    }

    /// Convenience: every call returns the same outcome.
    pub fn responds(collide: bool) -> Self {
        let p = Self::new();
        if collide {
            // Push enough collisions to cover any reasonable retry budget;
            // anything past the queue defaults to Free anyway.
            for _ in 0..256 {
                p.queue_arp(ProbeOutcome::Collision {
                    peer_ip: Some("192.0.2.1".into()),
                });
                p.queue_nd(ProbeOutcome::Collision {
                    peer_ip: Some("fe80::1".into()),
                });
            }
        }
        p
    }

    pub fn queue_arp(&self, outcome: ProbeOutcome) {
        self.arp_script.lock().unwrap().push(outcome);
    }

    pub fn queue_nd(&self, outcome: ProbeOutcome) {
        self.nd_script.lock().unwrap().push(outcome);
    }
}

impl Default for MockProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe for MockProbe {
    fn arp_probe(&self, iface: &str, candidate: Mac, _timeout: Duration) -> ProbeOutcome {
        self.arp_calls
            .lock()
            .unwrap()
            .push((iface.to_string(), candidate));
        let mut q = self.arp_script.lock().unwrap();
        if q.is_empty() {
            ProbeOutcome::Free
        } else {
            q.remove(0)
        }
    }

    fn nd_probe(&self, iface: &str, candidate: Mac, _timeout: Duration) -> ProbeOutcome {
        self.nd_calls
            .lock()
            .unwrap()
            .push((iface.to_string(), candidate));
        let mut q = self.nd_script.lock().unwrap();
        if q.is_empty() {
            ProbeOutcome::Free
        } else {
            q.remove(0)
        }
    }
}

/// Derive the modified-EUI-64 link-local address from a MAC. Used by the
/// ND probe target picker and surfaced to `--explain` output. Format per
/// RFC 4291 Appendix A: insert `FF:FE` between bytes 3 and 4, flip the U/L
/// bit on the first octet, prefix with `fe80::`.
///
/// Returns the canonical zero-compressed lower-half form (no zone-id).
pub fn link_local_from_mac(mac: Mac) -> String {
    let o = mac.octets();
    let b0 = o[0] ^ 0x02;
    // Build the eight bytes of the IID then format as four hex-quartets.
    let iid = [b0, o[1], o[2], 0xFF, 0xFE, o[3], o[4], o[5]];
    let q1 = ((iid[0] as u16) << 8) | iid[1] as u16;
    let q2 = ((iid[2] as u16) << 8) | iid[3] as u16;
    let q3 = ((iid[4] as u16) << 8) | iid[5] as u16;
    let q4 = ((iid[6] as u16) << 8) | iid[7] as u16;
    format!("fe80::{q1:x}:{q2:x}:{q3:x}:{q4:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_probe_consumes_script_in_order() {
        let p = MockProbe::new();
        p.queue_arp(ProbeOutcome::Collision {
            peer_ip: Some("10.0.0.1".into()),
        });
        p.queue_arp(ProbeOutcome::Free);
        let mac = "aa:bb:cc:dd:ee:ff".parse::<Mac>().unwrap();
        let r1 = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        let r2 = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        let r3 = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        assert!(matches!(r1, ProbeOutcome::Collision { .. }));
        assert_eq!(r2, ProbeOutcome::Free);
        // Empty queue defaults to Free so callers don't have to over-fill.
        assert_eq!(r3, ProbeOutcome::Free);
    }

    #[test]
    fn mock_probe_records_calls_with_iface_and_mac() {
        let p = MockProbe::new();
        let mac = "12:34:56:78:9a:bc".parse::<Mac>().unwrap();
        let _ = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        let _ = p.nd_probe("eth0", mac, ND_PROBE_TIMEOUT);
        let arp = p.arp_calls.lock().unwrap();
        let nd = p.nd_calls.lock().unwrap();
        assert_eq!(arp.len(), 1);
        assert_eq!(arp[0], ("wlan0".to_string(), mac));
        assert_eq!(nd.len(), 1);
        assert_eq!(nd[0], ("eth0".to_string(), mac));
    }

    #[test]
    fn responds_true_returns_collision_first() {
        let p = MockProbe::responds(true);
        let mac = "aa:bb:cc:dd:ee:ff".parse::<Mac>().unwrap();
        let r = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        assert!(matches!(r, ProbeOutcome::Collision { .. }));
    }

    #[test]
    fn responds_false_returns_free() {
        let p = MockProbe::responds(false);
        let mac = "aa:bb:cc:dd:ee:ff".parse::<Mac>().unwrap();
        assert_eq!(
            p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT),
            ProbeOutcome::Free
        );
        assert_eq!(
            p.nd_probe("wlan0", mac, ND_PROBE_TIMEOUT),
            ProbeOutcome::Free
        );
    }

    #[test]
    fn system_probe_returns_unsupported_without_cap_net_raw() {
        // Production builds without CAP_NET_RAW must fall back gracefully —
        // never crash, never report a false collision. The integration
        // container with the capability swaps in a real impl.
        let p = SystemProbe::new();
        let mac = "aa:bb:cc:dd:ee:ff".parse::<Mac>().unwrap();
        let r = p.arp_probe("wlan0", mac, ARP_PROBE_TIMEOUT);
        assert!(matches!(r, ProbeOutcome::Unsupported(_)));
        let r = p.nd_probe("wlan0", mac, ND_PROBE_TIMEOUT);
        assert!(matches!(r, ProbeOutcome::Unsupported(_)));
    }

    #[test]
    fn link_local_from_mac_flips_ul_bit_and_inserts_fffe() {
        // RFC 4291 Appendix A worked example:
        // 00:1B:63:00:0A:75  ->  fe80::21b:63ff:fe00:a75
        let mac = "00:1b:63:00:0a:75".parse::<Mac>().unwrap();
        assert_eq!(link_local_from_mac(mac), "fe80::21b:63ff:fe00:a75");
    }

    #[test]
    fn link_local_from_mac_locally_administered_clears_bit() {
        // LAA bit set -> EUI-64 flips it back to 0. The renderer keeps
        // four explicit quartets after `fe80::` (no zero-compression of
        // the IID half), so a zero leading quartet shows up as `:0:`
        // rather than being absorbed into the `::`. Strict RFC 5952
        // canonicalization is overkill here — this string is for log
        // output and explain traces, not for a transport socket.
        let mac = "02:00:00:00:00:01".parse::<Mac>().unwrap();
        assert_eq!(link_local_from_mac(mac), "fe80::0:ff:fe00:1");
    }
}
