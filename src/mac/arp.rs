// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use super::Mac;

const DEFAULT_ARP_PATH: &str = "/proc/net/arp";

pub fn read_arp_macs() -> HashSet<Mac> {
    read_arp_macs_from(Path::new(DEFAULT_ARP_PATH)).unwrap_or_else(|e| {
        tracing::debug!("ARP read failed ({e}); proceeding with empty ARP set");
        HashSet::new()
    })
}

pub fn read_arp_macs_from(path: &Path) -> Result<HashSet<Mac>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading ARP table {}", path.display()))?;
    Ok(parse_arp(&body))
}

pub fn parse_arp(body: &str) -> HashSet<Mac> {
    let mut out = HashSet::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            // Header row.
            continue;
        }
        // Layout: IPaddr HWtype HWaddr Flags Mask Device
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        let hw = cols[3];
        if hw == "00:00:00:00:00:00" {
            continue;
        }
        if let Ok(m) = hw.parse::<Mac>()
            && m.validate_assignable().is_ok()
        {
            out.insert(m);
        }
    }
    out
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
    // Destination=00000000 means "default route"; Gateway is little-endian hex.
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        if cols[1] != "00000000" {
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
