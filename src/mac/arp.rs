// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use super::Mac;

const DEFAULT_ARP_PATH: &str = "/proc/net/arp";

/// Roadmap M2: default sliding window for the recent-neighbour exclusion.
/// The active probe catches *now*; the recent-table catches "this neighbour
/// was here a minute ago and is plausibly still around the corner". 5
/// minutes mirrors the kernel's stale-neighbour timeout (300s for
/// `gc_stale_time`) without leaning on it directly. Configurable via
/// `RecentNeighbourTable::with_window`.
pub const DEFAULT_RECENT_WINDOW: Duration = Duration::from_secs(300);

/// Bit values for the `Flags` column in `/proc/net/route` (see
/// `<linux/route.h>`). RTF_UP marks the route as live, RTF_GATEWAY says the
/// `Gateway` field actually holds the next-hop address rather than 0.
const RTF_UP: u32 = 0x0001;
const RTF_GATEWAY: u32 = 0x0002;

pub fn read_arp_macs() -> HashSet<Mac> {
    read_arp_macs_from(Path::new(DEFAULT_ARP_PATH)).unwrap_or_else(|e| {
        tracing::debug!("ARP read failed ({e}); proceeding with empty ARP set");
        HashSet::new()
    })
}

pub fn read_arp_macs_from(path: &Path) -> Result<HashSet<Mac>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading ARP table {}", path.display()))?;
    let (macs, errors) = parse_arp_with_errors(&body);
    for e in errors {
        // Bad rows usually mean a kernel format change or a transient
        // tear-down race. Surface them at debug rather than swallowing
        // silently — `journalctl -t proteus` can pick them up if a user
        // reports an empty arp set (issue #146).
        tracing::debug!("ARP row ignored: {e}");
    }
    Ok(macs)
}

pub fn parse_arp(body: &str) -> HashSet<Mac> {
    parse_arp_with_errors(body).0
}

/// Parse `/proc/net/arp`, returning the assignable MACs plus any parse
/// errors encountered along the way. The error list lets callers surface
/// row-level problems instead of silently dropping them.
fn parse_arp_with_errors(body: &str) -> (HashSet<Mac>, Vec<String>) {
    let mut out = HashSet::new();
    let mut errors = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            // Header row.
            continue;
        }
        // Layout: IPaddr HWtype HWaddr Flags Mask Device
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            if !cols.is_empty() {
                errors.push(format!("row {i}: only {} columns (need ≥4)", cols.len()));
            }
            continue;
        }
        let hw = cols[3];
        if hw == "00:00:00:00:00:00" {
            continue;
        }
        match hw.parse::<Mac>() {
            Ok(m) if m.validate_assignable().is_ok() => {
                out.insert(m);
            }
            Ok(_) => {
                // Multicast / non-assignable. Not an error — just not a
                // candidate for collision-avoidance.
            }
            Err(e) => errors.push(format!("row {i}: bad MAC '{hw}': {e}")),
        }
    }
    (out, errors)
}

pub fn read_default_gateway_mac() -> Option<Mac> {
    read_default_gateway_mac_with(Path::new("/proc/net/route"), Path::new(DEFAULT_ARP_PATH))
}

pub fn read_default_gateway_mac_with(route_path: &Path, arp_path: &Path) -> Option<Mac> {
    let route = std::fs::read_to_string(route_path).ok()?;
    let arp = std::fs::read_to_string(arp_path).ok()?;
    let gw_ips = parse_default_gateways(&route);
    let pairs = parse_arp_pairs(&arp);
    for ip in gw_ips {
        for (arp_ip, mac) in &pairs {
            if arp_ip == &ip {
                return Some(*mac);
            }
        }
    }
    None
}

fn parse_default_gateways(body: &str) -> Vec<String> {
    // /proc/net/route columns: Iface Destination Gateway Flags ...
    // A real default-gateway entry has Destination=00000000 *and* the
    // RTF_GATEWAY (0x2) flag set. Without the flag check we'd also match
    // on-link default-target entries (e.g. point-to-point tun devices) and
    // pick up the wrong neighbour MAC. The Flags column is a hex-encoded
    // `unsigned short` per `<linux/route.h>` (issue #146).
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        if cols[1] != "00000000" {
            continue;
        }
        let flags = match u32::from_str_radix(cols[3], 16) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if flags & RTF_GATEWAY == 0 || flags & RTF_UP == 0 {
            continue;
        }
        if let Some(ip) = hex_le_to_ipv4(cols[2]) {
            out.push(ip);
        }
    }
    out
}

fn parse_arp_pairs(body: &str) -> Vec<(String, Mac)> {
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        if let Ok(m) = cols[3].parse::<Mac>() {
            out.push((cols[0].to_string(), m));
        }
    }
    out
}

fn hex_le_to_ipv4(hex: &str) -> Option<String> {
    if hex.len() != 8 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    let b1 = n & 0xFF;
    let b2 = (n >> 8) & 0xFF;
    let b3 = (n >> 16) & 0xFF;
    let b4 = (n >> 24) & 0xFF;
    Some(format!("{b1}.{b2}.{b3}.{b4}"))
}

/// Roadmap M2: in-memory MAC last-seen ledger. Layered ON TOP of the
/// one-shot `/proc/net/arp` parse so a neighbour that came online, dropped
/// off the kernel's neighbour cache, then briefly came back doesn't get
/// re-collided with on the next rotation.
///
/// The `Instant`-equivalent here is a Unix-epoch second: makes the
/// last-seen field cheap to serialize/log later if we ever need to persist
/// it. Pruning runs lazily on `current_macs` so tests can drive time
/// without a separate clean-up call.
#[derive(Debug, Default)]
pub struct RecentNeighbourTable {
    entries: Mutex<HashMap<Mac, u64>>,
    window: Duration,
}

impl RecentNeighbourTable {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            window: DEFAULT_RECENT_WINDOW,
        }
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            window,
        }
    }

    /// Record a sighting of `mac` at the current wall-clock time.
    pub fn record(&self, mac: Mac) {
        self.record_at(mac, now_unix_secs());
    }

    /// Test/integration entry point — records a sighting at a specific
    /// epoch second. Production code uses [`record`].
    pub fn record_at(&self, mac: Mac, when_unix_secs: u64) {
        let mut g = self.entries.lock().unwrap();
        // Late entries don't move the timestamp backwards; only forwards.
        let slot = g.entry(mac).or_insert(when_unix_secs);
        if when_unix_secs > *slot {
            *slot = when_unix_secs;
        }
    }

    /// Bulk-insert every MAC in the kernel's current neighbour table.
    /// Surfaces a "we already know about everyone the kernel knows about"
    /// baseline before the per-rotation passive snapshot runs.
    pub fn record_all(&self, macs: impl IntoIterator<Item = Mac>) {
        let now = now_unix_secs();
        for mac in macs {
            self.record_at(mac, now);
        }
    }

    /// Drop entries whose last-seen is older than the window. Idempotent.
    pub fn prune(&self) {
        self.prune_at(now_unix_secs());
    }

    pub fn prune_at(&self, now_unix_secs: u64) {
        let cutoff = now_unix_secs.saturating_sub(self.window.as_secs());
        let mut g = self.entries.lock().unwrap();
        g.retain(|_, ts| *ts >= cutoff);
    }

    /// MACs currently inside the window. Prunes lazily so callers can
    /// always trust the result is current without a manual `prune` call.
    pub fn current_macs(&self) -> HashSet<Mac> {
        self.current_macs_at(now_unix_secs())
    }

    pub fn current_macs_at(&self, now_unix_secs: u64) -> HashSet<Mac> {
        self.prune_at(now_unix_secs);
        self.entries.lock().unwrap().keys().copied().collect()
    }

    /// Window currently in effect. Surfaced for `--explain` so the operator
    /// can see "we're excluding everyone we saw in the last 300s".
    pub fn window(&self) -> Duration {
        self.window
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARP_SAMPLE: &str = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.1.1      0x1         0x2         aa:bb:cc:dd:ee:ff     *        wlan0
192.168.1.42     0x1         0x2         12:34:56:78:9a:bc     *        wlan0
192.168.1.99     0x1         0x0         00:00:00:00:00:00     *        wlan0
";

    #[test]
    fn parses_arp_table_skipping_zero() {
        let macs = parse_arp(ARP_SAMPLE);
        assert_eq!(macs.len(), 2);
        assert!(macs.contains(&"aa:bb:cc:dd:ee:ff".parse().unwrap()));
        assert!(macs.contains(&"12:34:56:78:9a:bc".parse().unwrap()));
        assert!(!macs.contains(&"00:00:00:00:00:00".parse::<Mac>().unwrap_or(Mac([0; 6]))));
    }

    #[test]
    fn ignores_malformed_arp_lines() {
        let body = "\
IP address       HW type     Flags       HW address            Mask     Device
not enough cols
";
        let macs = parse_arp(body);
        assert!(macs.is_empty());
    }

    #[test]
    fn hex_le_decodes_default_gateway_form() {
        // 0101A8C0 little-endian = 192.168.1.1
        assert_eq!(hex_le_to_ipv4("0101A8C0"), Some("192.168.1.1".to_string()));
        assert_eq!(hex_le_to_ipv4("00000000"), Some("0.0.0.0".to_string()));
    }

    #[test]
    fn extracts_default_gateway_ip() {
        let route = "\
Iface   Destination     Gateway         Flags   RefCnt  Use     Metric  Mask            MTU     Window  IRTT
wlan0   00000000        0101A8C0        0003    0       0       100     00000000        0       0       0
wlan0   0000FEA9        00000000        0001    0       0       1000    0000FFFF        0       0       0
";
        let gws = parse_default_gateways(route);
        assert_eq!(gws, vec!["192.168.1.1".to_string()]);
    }

    #[test]
    fn skips_destination_zero_without_gateway_flag() {
        // A route with Destination=00000000 but no RTF_GATEWAY (0x2) bit
        // set is an on-link default target (point-to-point), not a real
        // default gateway. Issue #146.
        let route = "\
Iface   Destination     Gateway         Flags   RefCnt  Use     Metric  Mask            MTU     Window  IRTT
tun0    00000000        00000000        0001    0       0       100     00000000        0       0       0
wlan0   00000000        0101A8C0        0003    0       0       100     00000000        0       0       0
";
        let gws = parse_default_gateways(route);
        assert_eq!(gws, vec!["192.168.1.1".to_string()]);
    }

    #[test]
    fn skips_routes_with_unknown_flag_encoding() {
        // A non-hex Flags column shouldn't match — better to drop the row
        // than to silently treat malformed kernel output as a gateway.
        let route = "\
Iface   Destination     Gateway         Flags
wlan0   00000000        0101A8C0        notahex
";
        assert!(parse_default_gateways(route).is_empty());
    }

    #[test]
    fn surfaces_arp_parse_errors_via_with_errors_helper() {
        // Mixing valid + invalid rows: the valid MAC still lands in the
        // set, and the bad row produces an error string that callers can
        // log instead of silently dropping (issue #146).
        let body = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.1.1      0x1         0x2         aa:bb:cc:dd:ee:ff     *        wlan0
192.168.1.5      0x1         0x2         not:a:mac:address     *        wlan0
192.168.1.7      0x1         0x2         zz:zz:zz:zz:zz:zz     *        wlan0
";
        let (macs, errors) = parse_arp_with_errors(body);
        assert_eq!(macs.len(), 1);
        assert!(macs.contains(&"aa:bb:cc:dd:ee:ff".parse().unwrap()));
        assert!(
            errors.iter().any(|e| e.contains("not:a:mac:address")),
            "expected error for malformed MAC, got {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("zz:zz:zz:zz:zz:zz")),
            "expected error for invalid hex MAC, got {errors:?}"
        );
    }

    #[test]
    fn recent_neighbour_table_records_and_returns_within_window() {
        let table = RecentNeighbourTable::with_window(Duration::from_secs(300));
        let m1: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let m2: Mac = "12:34:56:78:9a:bc".parse().unwrap();
        let t0 = 1_000_000u64;
        table.record_at(m1, t0);
        table.record_at(m2, t0 + 50);
        let seen = table.current_macs_at(t0 + 100);
        assert_eq!(seen.len(), 2);
        assert!(seen.contains(&m1));
        assert!(seen.contains(&m2));
    }

    #[test]
    fn recent_neighbour_table_prunes_outside_window() {
        let table = RecentNeighbourTable::with_window(Duration::from_secs(60));
        let stale: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let fresh: Mac = "12:34:56:78:9a:bc".parse().unwrap();
        table.record_at(stale, 1_000_000);
        table.record_at(fresh, 1_000_120);
        // Now is 121 seconds after `stale` was last seen — outside the
        // 60s window — and 1 second after `fresh`.
        let seen = table.current_macs_at(1_000_121);
        assert!(!seen.contains(&stale), "stale entry must have been pruned");
        assert!(seen.contains(&fresh), "fresh entry must remain");
    }

    #[test]
    fn recent_neighbour_table_record_all_baselines_kernel_set() {
        // Bulk-record from a current `read_arp_macs` snapshot.
        let table = RecentNeighbourTable::new();
        let kernel = parse_arp(ARP_SAMPLE);
        table.record_all(kernel.iter().copied());
        let seen = table.current_macs();
        for m in &kernel {
            assert!(seen.contains(m), "expected {m} to be in recent table");
        }
    }

    #[test]
    fn recent_neighbour_table_record_does_not_move_ts_backwards() {
        // Late-arriving observations from a slow probe must not extend the
        // window backwards — otherwise an old sighting could "revive" a
        // pruned MAC. Going forward, however, IS the whole point.
        let table = RecentNeighbourTable::with_window(Duration::from_secs(60));
        let m: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        table.record_at(m, 1_000_100);
        table.record_at(m, 1_000_050); // older
        table.record_at(m, 1_000_200); // newer
        // After the window from t=1_000_200 is 60s, t=1_000_140 is the
        // cutoff. The forward update must have stuck.
        let seen = table.current_macs_at(1_000_259);
        assert!(seen.contains(&m), "newest record_at should win");
    }

    #[test]
    fn finds_gateway_mac_from_arp_and_route() {
        let dir = std::env::temp_dir();
        let route_path = dir.join("proteus_test_route.txt");
        let arp_path = dir.join("proteus_test_arp.txt");
        std::fs::write(
            &route_path,
            "\
Iface   Destination     Gateway         Flags
wlan0   00000000        0101A8C0        0003
",
        )
        .unwrap();
        std::fs::write(&arp_path, ARP_SAMPLE).unwrap();
        let mac = read_default_gateway_mac_with(&route_path, &arp_path);
        assert_eq!(mac, Some("aa:bb:cc:dd:ee:ff".parse().unwrap()));
        let _ = std::fs::remove_file(&route_path);
        let _ = std::fs::remove_file(&arp_path);
    }
}
