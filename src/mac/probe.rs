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
    fn arp_probe(&self, iface: &str, candidate: Mac, timeout: Duration) -> ProbeOutcome {
        // Issue #267: real RFC 5227 ARP probe via AF_PACKET raw socket.
        // Falls back to `Unsupported` when:
        //   - the iface name is empty / contains shell-unsafe characters
        //   - the kernel rejects AF_PACKET (we lack CAP_NET_RAW)
        //   - `if_nametoindex` returns 0 (the iface vanished)
        // The fallback string is concrete enough that the rotate path can
        // distinguish "couldn't probe" from "no collision" — the M2 docs
        // call this "graceful degrade to passive ARP".
        match raw::arp_probe(iface, candidate, timeout) {
            Ok(o) => o,
            Err(e) => {
                tracing::debug!(
                    iface,
                    candidate = %candidate,
                    "SystemProbe::arp_probe failed: {e}; falling back to passive ARP"
                );
                ProbeOutcome::Unsupported("arp probe unsupported on this system (see debug log)")
            }
        }
    }

    fn nd_probe(&self, iface: &str, candidate: Mac, timeout: Duration) -> ProbeOutcome {
        // Issue #267: best-effort IPv6 DAD probe. We send a Neighbor
        // Solicitation for the link-local address derived from the
        // candidate MAC and listen for any Neighbor Advertisement. Same
        // failure-mode contract as `arp_probe`.
        match raw::nd_probe(iface, candidate, timeout) {
            Ok(o) => o,
            Err(e) => {
                tracing::debug!(
                    iface,
                    candidate = %candidate,
                    "SystemProbe::nd_probe failed: {e}; falling back to passive neighbour table"
                );
                ProbeOutcome::Unsupported("nd probe unsupported on this system (see debug log)")
            }
        }
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

// ---- Raw-socket probe implementations (issue #267) -----------------------
//
// Lives in a private submodule so the unsafe libc surface is contained to
// one file and the `SystemProbe` impl above stays a thin dispatcher. All
// helpers return `Result<ProbeOutcome>`; an `Err` becomes
// `ProbeOutcome::Unsupported` at the trait boundary so callers always see
// the same three-way outcome regardless of which kernel call failed.
mod raw {
    use super::{Mac, ProbeOutcome};
    use anyhow::{Context, Result, bail};
    use std::ffi::CString;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    /// EtherType for ARP. Hard-coded so we don't need to depend on a
    /// platform-specific libc constant.
    const ETH_P_ARP: u16 = 0x0806;
    /// EtherType for IPv6 (the link layer above which we'd inject ICMPv6).
    const ETH_P_IPV6: u16 = 0x86DD;
    /// ARP hardware type for Ethernet.
    const ARP_HW_ETHER: u16 = 1;
    /// ARP opcodes per RFC 826.
    const ARP_OP_REQUEST: u16 = 1;
    const ARP_OP_REPLY: u16 = 2;

    /// RFC 5227 ARP Probe.
    ///
    /// Send an ARP request with `sender_hw = candidate`,
    /// `sender_proto = 0.0.0.0`, target_proto = a fixed link-local address
    /// derived from the candidate (so two parallel probes for different
    /// candidates don't trip each other). Listen on the same socket for
    /// any ARP REPLY whose `sender_hw` matches the candidate MAC — that's
    /// a defender announcing the address is taken.
    pub fn arp_probe(iface: &str, candidate: Mac, timeout: Duration) -> Result<ProbeOutcome> {
        let (fd, ifindex) = open_bound_socket(iface, ETH_P_ARP, timeout)?;

        // Probe target: a stable per-candidate IPv4 link-local address. We
        // don't actually own this IP — it's just the slot we ask "anybody
        // claiming this?" for. Picking it from the candidate's tail keeps
        // parallel probes for different MACs from colliding on the same
        // target.
        let octets = candidate.octets();
        let target_ip = [169u8, 254u8, octets[4], octets[5]];
        let frame = build_arp_request(candidate.octets(), [0u8; 4], [0u8; 6], target_ip);
        sendto_packet(&fd, &frame, ifindex, ETH_P_ARP, [0xffu8; 6]).context("sending ARP probe")?;

        // Filter on `parse_arp_reply` + `sender_hw == candidate`. Other
        // ARP traffic on the segment is dropped.
        listen_for_collision(&fd, timeout, |frame| {
            parse_arp_reply(frame).and_then(|r| {
                if r.sender_hw == candidate.octets() {
                    Some(ip_to_string(&r.sender_proto))
                } else {
                    None
                }
            })
        })
    }

    /// Best-effort IPv6 DAD probe. Sends a Neighbor Solicitation for the
    /// candidate's modified-EUI-64 link-local address and listens for an
    /// NA. The implementation is deliberately conservative — most drivers
    /// answer DAD in <100ms, so a 1s default window is generous; we
    /// degrade to `Unsupported` on any kernel-side rejection.
    pub fn nd_probe(iface: &str, candidate: Mac, timeout: Duration) -> Result<ProbeOutcome> {
        let (fd, ifindex) = open_bound_socket(iface, ETH_P_IPV6, timeout)?;

        // Target IPv6: the candidate's modified-EUI-64 link-local address.
        // Per RFC 4862, DAD sends an NS for the *tentative* address with
        // src=:: and target=tentative. A defender (the existing owner)
        // replies with an NA whose target_addr == tentative.
        let target_ipv6 = link_local_octets_from_mac(candidate);
        let solicited_node = solicited_node_multicast(&target_ipv6);
        let solicited_dst_mac = [
            0x33,
            0x33,
            solicited_node[12],
            solicited_node[13],
            solicited_node[14],
            solicited_node[15],
        ];
        let frame = build_neighbor_solicitation(
            candidate.octets(),
            solicited_dst_mac,
            target_ipv6,
            solicited_node,
        );
        sendto_packet(&fd, &frame, ifindex, ETH_P_IPV6, solicited_dst_mac)
            .context("sending IPv6 NS")?;

        listen_for_collision(&fd, timeout, |frame| {
            parse_neighbor_advertisement(frame).and_then(|na| {
                if na.target_addr == target_ipv6 {
                    Some(ipv6_to_string(&target_ipv6))
                } else {
                    None
                }
            })
        })
    }

    /// Open + bind an AF_PACKET socket on `iface` for `eth_proto` and set
    /// the initial recv timeout. Returns the socket and the resolved
    /// ifindex so callers don't have to call `if_nametoindex` again for
    /// `sendto`.
    fn open_bound_socket(iface: &str, eth_proto: u16, timeout: Duration) -> Result<(OwnedFd, u32)> {
        validate_iface(iface)?;
        let ifindex = if_nametoindex(iface)?;
        let fd = open_packet_socket(eth_proto)
            .context("opening AF_PACKET raw socket (needs CAP_NET_RAW)")?;
        bind_packet_to_iface(&fd, ifindex, eth_proto)
            .context("binding AF_PACKET socket to iface")?;
        set_recv_timeout(&fd, timeout).context("setting socket recv timeout")?;
        Ok((fd, ifindex))
    }

    /// Listen on `fd` until `timeout` elapses or the per-frame `match_fn`
    /// returns `Some(peer_ip)`. Maps timeout / WouldBlock to
    /// `ProbeOutcome::Free`; any other I/O error bubbles up so the caller
    /// can demote it to `Unsupported`.
    fn listen_for_collision(
        fd: &OwnedFd,
        timeout: Duration,
        match_fn: impl Fn(&[u8]) -> Option<String>,
    ) -> Result<ProbeOutcome> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 1500];
        loop {
            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(d) if !d.is_zero() => d,
                _ => return Ok(ProbeOutcome::Free),
            };
            // Refresh the SO_RCVTIMEO so a slow defender doesn't drop us
            // out of the loop early.
            set_recv_timeout(fd, remaining)?;
            let n = match recv_packet(fd, &mut buf) {
                Ok(n) => n,
                Err(e)
                    if e.raw_os_error() == Some(libc::EAGAIN)
                        || e.kind() == io::ErrorKind::WouldBlock =>
                {
                    return Ok(ProbeOutcome::Free);
                }
                Err(e) => return Err(e.into()),
            };
            if let Some(peer_ip) = match_fn(&buf[..n]) {
                return Ok(ProbeOutcome::Collision {
                    peer_ip: Some(peer_ip),
                });
            }
        }
    }

    // ---- low-level libc wrappers -----------------------------------------

    fn validate_iface(iface: &str) -> Result<()> {
        if iface.is_empty() {
            bail!("empty iface name");
        }
        if iface.contains('\0') {
            bail!("iface contains NUL byte");
        }
        if iface.len() >= 16 {
            // IFNAMSIZ on Linux is 16 — a name longer than that can't be
            // looked up via if_nametoindex.
            bail!("iface name too long");
        }
        Ok(())
    }

    fn if_nametoindex(iface: &str) -> Result<u32> {
        let cstr = CString::new(iface).context("iface name → CString")?;
        // SAFETY: `cstr.as_ptr()` is a valid NUL-terminated C string for
        // the duration of the call; libc reads it and returns either 0
        // (errno set) or the kernel-assigned ifindex.
        let n = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
        if n == 0 {
            return Err(io::Error::last_os_error()).context("if_nametoindex failed");
        }
        Ok(n)
    }

    fn open_packet_socket(eth_proto: u16) -> io::Result<OwnedFd> {
        // SOCK_RAW + AF_PACKET requires CAP_NET_RAW. The protocol is the
        // EtherType in network byte order so the kernel can pre-filter.
        // SAFETY: the call only allocates a kernel socket; we wrap in
        // OwnedFd immediately so close() runs on every error path.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, eth_proto.to_be() as i32) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn bind_packet_to_iface(fd: &OwnedFd, ifindex: u32, eth_proto: u16) -> io::Result<()> {
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as u16;
        addr.sll_protocol = eth_proto.to_be();
        addr.sll_ifindex = ifindex as i32;
        let rc = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as u32,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn set_recv_timeout(fd: &OwnedFd, timeout: Duration) -> io::Result<()> {
        let tv = libc::timeval {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_usec: timeout.subsec_micros() as libc::suseconds_t,
        };
        let rc = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &tv as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as u32,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn sendto_packet(
        fd: &OwnedFd,
        frame: &[u8],
        ifindex: u32,
        eth_proto: u16,
        dst_mac: [u8; 6],
    ) -> io::Result<()> {
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as u16;
        addr.sll_protocol = eth_proto.to_be();
        addr.sll_ifindex = ifindex as i32;
        addr.sll_halen = 6;
        addr.sll_addr[..6].copy_from_slice(&dst_mac);
        let rc = unsafe {
            libc::sendto(
                fd.as_raw_fd(),
                frame.as_ptr() as *const libc::c_void,
                frame.len(),
                0,
                &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as u32,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn recv_packet(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    // ---- frame builders / parsers ----------------------------------------

    /// Build an Ethernet + ARP request frame. RFC 5227 says the sender
    /// hardware is the candidate MAC, sender_proto = 0.0.0.0, target_proto =
    /// the IP we're claiming. Target hardware in the request is unused by
    /// the responder so we send zeros.
    fn build_arp_request(
        sender_hw: [u8; 6],
        sender_proto: [u8; 4],
        target_hw: [u8; 6],
        target_proto: [u8; 4],
    ) -> [u8; 42] {
        let mut frame = [0u8; 42];
        // Ethernet: dst = broadcast, src = sender_hw, type = ARP.
        frame[0..6].copy_from_slice(&[0xff; 6]);
        frame[6..12].copy_from_slice(&sender_hw);
        frame[12..14].copy_from_slice(&ETH_P_ARP.to_be_bytes());
        // ARP header.
        frame[14..16].copy_from_slice(&ARP_HW_ETHER.to_be_bytes());
        frame[16..18].copy_from_slice(&0x0800u16.to_be_bytes()); // proto = IPv4
        frame[18] = 6; // hw addr len
        frame[19] = 4; // proto addr len
        frame[20..22].copy_from_slice(&ARP_OP_REQUEST.to_be_bytes());
        frame[22..28].copy_from_slice(&sender_hw);
        frame[28..32].copy_from_slice(&sender_proto);
        frame[32..38].copy_from_slice(&target_hw);
        frame[38..42].copy_from_slice(&target_proto);
        frame
    }

    struct ArpReply {
        sender_hw: [u8; 6],
        sender_proto: [u8; 4],
    }

    fn parse_arp_reply(frame: &[u8]) -> Option<ArpReply> {
        if frame.len() < 42 {
            return None;
        }
        // EtherType
        if u16::from_be_bytes([frame[12], frame[13]]) != ETH_P_ARP {
            return None;
        }
        // Opcode
        if u16::from_be_bytes([frame[20], frame[21]]) != ARP_OP_REPLY {
            return None;
        }
        let mut sender_hw = [0u8; 6];
        sender_hw.copy_from_slice(&frame[22..28]);
        let mut sender_proto = [0u8; 4];
        sender_proto.copy_from_slice(&frame[28..32]);
        Some(ArpReply {
            sender_hw,
            sender_proto,
        })
    }

    /// Build the EUI-64 link-local IPv6 address bytes (fe80::/10 + IID).
    fn link_local_octets_from_mac(mac: Mac) -> [u8; 16] {
        let o = mac.octets();
        let b0 = o[0] ^ 0x02;
        let mut a = [0u8; 16];
        a[0] = 0xfe;
        a[1] = 0x80;
        // a[2..8] zero
        a[8] = b0;
        a[9] = o[1];
        a[10] = o[2];
        a[11] = 0xff;
        a[12] = 0xfe;
        a[13] = o[3];
        a[14] = o[4];
        a[15] = o[5];
        a
    }

    /// Solicited-node multicast = ff02::1:ffXX:XXXX where the last 24 bits
    /// are the target's last 24 bits.
    fn solicited_node_multicast(target: &[u8; 16]) -> [u8; 16] {
        let mut a = [0u8; 16];
        a[0] = 0xff;
        a[1] = 0x02;
        a[11] = 0x01;
        a[12] = 0xff;
        a[13] = target[13];
        a[14] = target[14];
        a[15] = target[15];
        a
    }

    /// Build an Ethernet + IPv6 + ICMPv6 NS frame. This is the minimal
    /// frame the kernel will let us emit; we don't need to add MAC source
    /// link-layer option since we're just probing.
    fn build_neighbor_solicitation(
        src_mac: [u8; 6],
        dst_mac: [u8; 6],
        target_ipv6: [u8; 16],
        dst_ipv6: [u8; 16],
    ) -> Vec<u8> {
        // Ethernet (14) + IPv6 (40) + ICMPv6 NS (24) = 78 bytes.
        let mut f = Vec::with_capacity(78);
        // Ethernet header
        f.extend_from_slice(&dst_mac);
        f.extend_from_slice(&src_mac);
        f.extend_from_slice(&ETH_P_IPV6.to_be_bytes());
        // IPv6 header (40 bytes)
        // Version 6 + Traffic Class 0 + Flow Label 0 = 0x60000000
        f.extend_from_slice(&[0x60, 0x00, 0x00, 0x00]);
        // Payload length = 24 (ICMPv6 NS)
        f.extend_from_slice(&24u16.to_be_bytes());
        f.push(58); // Next header = ICMPv6
        f.push(255); // Hop limit (RFC 4861 mandates 255 for ND)
        // Source address: :: (DAD-style probe; RFC 4862)
        f.extend_from_slice(&[0u8; 16]);
        // Destination = solicited-node multicast
        f.extend_from_slice(&dst_ipv6);
        // ICMPv6 Neighbor Solicitation (24 bytes)
        let icmp_start = f.len();
        f.push(135); // Type = NS
        f.push(0); // Code
        f.extend_from_slice(&[0u8, 0u8]); // Checksum (computed below)
        f.extend_from_slice(&[0u8; 4]); // Reserved
        f.extend_from_slice(&target_ipv6);
        // Compute ICMPv6 checksum over pseudo-header + ICMPv6 message.
        let cksum = icmpv6_checksum(&[0u8; 16], &dst_ipv6, &f[icmp_start..]);
        f[icmp_start + 2..icmp_start + 4].copy_from_slice(&cksum.to_be_bytes());
        f
    }

    fn icmpv6_checksum(src: &[u8; 16], dst: &[u8; 16], msg: &[u8]) -> u16 {
        // Pseudo-header: src + dst + length + zeros + next_header(58)
        let mut sum: u32 = 0;
        for chunk in src.chunks_exact(2).chain(dst.chunks_exact(2)) {
            sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
        }
        sum = sum.wrapping_add(msg.len() as u32);
        sum = sum.wrapping_add(58); // Next header
        let mut iter = msg.chunks_exact(2);
        for chunk in iter.by_ref() {
            sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
        }
        if let &[last] = iter.remainder() {
            sum = sum.wrapping_add((last as u32) << 8);
        }
        while (sum >> 16) != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        !(sum as u16)
    }

    struct Na {
        target_addr: [u8; 16],
    }

    fn parse_neighbor_advertisement(frame: &[u8]) -> Option<Na> {
        // Eth(14) + IPv6(40) + ICMPv6(24)
        if frame.len() < 78 {
            return None;
        }
        // EtherType = IPv6?
        if u16::from_be_bytes([frame[12], frame[13]]) != ETH_P_IPV6 {
            return None;
        }
        // Next header = ICMPv6?
        if frame[14 + 6] != 58 {
            return None;
        }
        // ICMPv6 type = 136 (NA)?
        if frame[14 + 40] != 136 {
            return None;
        }
        let mut target = [0u8; 16];
        target.copy_from_slice(&frame[14 + 40 + 8..14 + 40 + 24]);
        Some(Na {
            target_addr: target,
        })
    }

    fn ip_to_string(ip: &[u8; 4]) -> String {
        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
    }

    fn ipv6_to_string(ip: &[u8; 16]) -> String {
        // Simple non-canonical form for log output.
        format!(
            "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
            u16::from_be_bytes([ip[0], ip[1]]),
            u16::from_be_bytes([ip[2], ip[3]]),
            u16::from_be_bytes([ip[4], ip[5]]),
            u16::from_be_bytes([ip[6], ip[7]]),
            u16::from_be_bytes([ip[8], ip[9]]),
            u16::from_be_bytes([ip[10], ip[11]]),
            u16::from_be_bytes([ip[12], ip[13]]),
            u16::from_be_bytes([ip[14], ip[15]])
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn arp_request_frame_layout_matches_rfc826() {
            let frame = build_arp_request(
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                [0u8; 4],
                [0u8; 6],
                [169, 254, 0xdd, 0xee],
            );
            // Ethernet dst = broadcast.
            assert_eq!(&frame[0..6], &[0xff; 6]);
            // Ethernet src = sender_hw.
            assert_eq!(&frame[6..12], &[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            // EtherType = 0x0806.
            assert_eq!(&frame[12..14], &[0x08, 0x06]);
            // Hardware type = 1.
            assert_eq!(&frame[14..16], &[0, 1]);
            // Protocol = 0x0800.
            assert_eq!(&frame[16..18], &[0x08, 0x00]);
            // hwlen=6, protolen=4.
            assert_eq!(frame[18], 6);
            assert_eq!(frame[19], 4);
            // Opcode = REQUEST.
            assert_eq!(&frame[20..22], &[0, 1]);
            // Sender hw.
            assert_eq!(&frame[22..28], &[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            // Sender proto = 0.0.0.0 per RFC 5227 probe.
            assert_eq!(&frame[28..32], &[0, 0, 0, 0]);
            // Target proto = 169.254.dd.ee
            assert_eq!(&frame[38..42], &[169, 254, 0xdd, 0xee]);
        }

        #[test]
        fn arp_reply_parser_extracts_sender_fields() {
            // Build a synthetic REPLY for the sender 12:34:56:78:9a:bc /
            // 192.168.1.1.
            let mut f = [0u8; 42];
            f[0..6].copy_from_slice(&[0xff; 6]);
            f[6..12].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]);
            f[12..14].copy_from_slice(&ETH_P_ARP.to_be_bytes());
            f[14..16].copy_from_slice(&ARP_HW_ETHER.to_be_bytes());
            f[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
            f[18] = 6;
            f[19] = 4;
            f[20..22].copy_from_slice(&ARP_OP_REPLY.to_be_bytes());
            f[22..28].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]);
            f[28..32].copy_from_slice(&[192, 168, 1, 1]);
            let r = parse_arp_reply(&f).unwrap();
            assert_eq!(r.sender_hw, [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]);
            assert_eq!(r.sender_proto, [192, 168, 1, 1]);
        }

        #[test]
        fn arp_reply_parser_rejects_request_opcode() {
            // A REQUEST frame must not be parsed as a reply.
            let f = build_arp_request(
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                [0u8; 4],
                [0u8; 6],
                [169, 254, 0, 1],
            );
            assert!(parse_arp_reply(&f).is_none());
        }

        #[test]
        fn arp_reply_parser_rejects_short_frame() {
            let short = [0u8; 30];
            assert!(parse_arp_reply(&short).is_none());
        }

        #[test]
        fn link_local_octets_from_mac_matches_rfc4291_example() {
            // 00:1B:63:00:0A:75  ->  fe80::21b:63ff:fe00:a75
            let mac: Mac = "00:1b:63:00:0a:75".parse().unwrap();
            let a = link_local_octets_from_mac(mac);
            assert_eq!(a[0], 0xfe);
            assert_eq!(a[1], 0x80);
            assert_eq!(a[8], 0x02); // U/L bit flipped
            assert_eq!(a[9], 0x1b);
            assert_eq!(a[10], 0x63);
            assert_eq!(a[11], 0xff);
            assert_eq!(a[12], 0xfe);
            assert_eq!(a[13], 0x00);
            assert_eq!(a[14], 0x0a);
            assert_eq!(a[15], 0x75);
        }

        #[test]
        fn solicited_node_multicast_uses_last_24_bits() {
            let target = [
                0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x1b, 0x63, 0xff, 0xfe, 0x00, 0x0a, 0x75,
            ];
            let s = solicited_node_multicast(&target);
            // ff02::1:ff00:a75
            assert_eq!(s[0], 0xff);
            assert_eq!(s[1], 0x02);
            assert_eq!(s[11], 0x01);
            assert_eq!(s[12], 0xff);
            assert_eq!(s[13], 0x00);
            assert_eq!(s[14], 0x0a);
            assert_eq!(s[15], 0x75);
        }

        #[test]
        fn icmpv6_checksum_round_trips() {
            // Spot-check: a payload checksum + the same checksum recomputed
            // (with the checksum field zeroed) must equal the original.
            let src = [0u8; 16];
            let mut dst = [0u8; 16];
            dst[0] = 0xff;
            dst[1] = 0x02;
            dst[15] = 0x01;
            let mut msg = vec![0u8; 24];
            msg[0] = 135; // NS
            // Compute checksum, place it, then verify it returns 0 over the
            // full payload including the checksum field.
            let cksum = icmpv6_checksum(&src, &dst, &msg);
            msg[2..4].copy_from_slice(&cksum.to_be_bytes());
            let verify = icmpv6_checksum(&src, &dst, &msg);
            assert_eq!(verify, 0, "valid ICMPv6 checksum verifies to 0");
        }

        #[test]
        fn validate_iface_rejects_pathological_inputs() {
            assert!(validate_iface("").is_err());
            assert!(validate_iface("a\0b").is_err());
            assert!(validate_iface("a".repeat(20).as_str()).is_err());
            assert!(validate_iface("wlan0").is_ok());
        }

        #[test]
        fn neighbor_advertisement_parser_extracts_target_addr() {
            // Build a synthetic NA frame.
            let mut f = vec![0u8; 78];
            // Ethernet
            f[0..6].copy_from_slice(&[0x33, 0x33, 0, 0, 0, 1]);
            f[6..12].copy_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            f[12..14].copy_from_slice(&ETH_P_IPV6.to_be_bytes());
            // IPv6 header — version 6
            f[14] = 0x60;
            // Next header at offset 14+6
            f[14 + 6] = 58;
            // ICMPv6 type at offset 14+40
            f[14 + 40] = 136;
            // Target addr at offset 14+40+8..14+40+24
            let target = [
                0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x1b, 0x63, 0xff, 0xfe, 0x00, 0x0a, 0x75,
            ];
            f[14 + 40 + 8..14 + 40 + 24].copy_from_slice(&target);
            let na = parse_neighbor_advertisement(&f).unwrap();
            assert_eq!(na.target_addr, target);
        }

        #[test]
        fn neighbor_advertisement_parser_rejects_wrong_icmp_type() {
            let mut f = vec![0u8; 78];
            f[12..14].copy_from_slice(&ETH_P_IPV6.to_be_bytes());
            f[14] = 0x60;
            f[14 + 6] = 58;
            f[14 + 40] = 135; // NS, not NA
            assert!(parse_neighbor_advertisement(&f).is_none());
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
    fn system_probe_falls_back_gracefully_when_iface_missing_or_unprivileged() {
        // Issue #267 (post-fix): the production probe now opens a real
        // AF_PACKET socket and emits a frame. In any environment where
        // either `wlan-bogus-test-iface` doesn't exist (every dev box,
        // every CI runner) OR we lack CAP_NET_RAW (the ordinary user
        // case), the probe must degrade to `Unsupported` rather than
        // crashing or reporting a false collision. We pick a name that's
        // guaranteed not to resolve via `if_nametoindex` so the test is
        // deterministic across all hosts.
        let p = SystemProbe::new();
        let mac = "aa:bb:cc:dd:ee:ff".parse::<Mac>().unwrap();
        let r = p.arp_probe("proteus-no-such-iface", mac, ARP_PROBE_TIMEOUT);
        assert!(
            matches!(r, ProbeOutcome::Unsupported(_)),
            "expected Unsupported for missing iface or no CAP_NET_RAW, got {r:?}"
        );
        let r = p.nd_probe("proteus-no-such-iface", mac, ND_PROBE_TIMEOUT);
        assert!(
            matches!(r, ProbeOutcome::Unsupported(_)),
            "expected Unsupported for missing iface or no CAP_NET_RAW, got {r:?}"
        );
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
