// SPDX-License-Identifier: GPL-3.0-or-later

//! Stack-fingerprint hardening: sysctl drop-in writer.
//!
//! Generates `/etc/sysctl.d/95-proteus.conf` with TCP/ICMP/NDP knobs derived
//! from `[stack]` config plus per-managed-interface NDP entries. The drop-in
//! header carries the standard `# managed by proteus` line plus a
//! `# sha256:<hash>` of the body so `proteus diff` (phase G) can spot manual
//! edits.
//!
//! Only the rendering, sysctl-line mapping, and SHA computation live here —
//! the actual `apply` / `revert` plumbing (root check, file write, `sysctl
//! --system` reload) lives in `crate::commands::stack` so this module stays
//! pure and unit-testable.

pub mod sha256;

use crate::config::StackConfig;
use crate::version;

/// Where the drop-in lives. sysctl.d(5) reads files from
/// `/etc/sysctl.d/`, `/run/sysctl.d/`, and `/usr/lib/sysctl.d/` in that
/// order; numbering at `95-` keeps us late so distro-shipped policies still
/// have the chance to set defaults we override.
pub const DROPIN_PATH: &str = "/etc/sysctl.d/95-proteus.conf";

/// One sysctl line, parameterised over key + value so the rendered file
/// stays a flat `key = value\n` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysctlLine {
    pub key: String,
    pub value: String,
}

impl SysctlLine {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    pub fn render(&self) -> String {
        format!("{} = {}", self.key, self.value)
    }
}

/// Derive the full ordered set of sysctl lines from config + managed
/// interface list. Order is stable so rendered files are byte-identical
/// across runs with the same inputs (idempotency invariant).
pub fn lines_for(cfg: &StackConfig, ifaces: &[String]) -> Vec<SysctlLine> {
    let mut out = Vec::new();

    if cfg.tcp_timestamps_off {
        out.push(SysctlLine::new("net.ipv4.tcp_timestamps", "0"));
    }

    if cfg.icmpv6_hardening {
        // Global IPv6 redirect/source-route guards. The `all` and `default`
        // namespaces both apply: `all` covers existing interfaces, `default`
        // applies to interfaces that show up later (USB tethering, hotplug
        // dongles, NetworkManager bringing up a profile post-boot).
        out.push(SysctlLine::new("net.ipv6.conf.all.accept_redirects", "0"));
        out.push(SysctlLine::new(
            "net.ipv6.conf.default.accept_redirects",
            "0",
        ));
        out.push(SysctlLine::new(
            "net.ipv6.conf.all.accept_source_route",
            "0",
        ));
        out.push(SysctlLine::new(
            "net.ipv6.conf.default.accept_source_route",
            "0",
        ));
        // Per-interface NDP eviction on carrier loss. Without this a stale
        // neighbour cache can bridge networks across an SSID switch.
        let mut sorted = ifaces.to_vec();
        sorted.sort();
        for iface in &sorted {
            out.push(SysctlLine::new(
                format!("net.ipv6.conf.{iface}.ndisc_evict_nocarrier"),
                "1",
            ));
        }
    }

    if cfg.suppress_gratuitous_arp {
        // `arp_announce = 2` makes the kernel pick a same-subnet source IP
        // for ARP requests, which reduces the leak on link-up. Off by
        // default; opt-in because it slows VRRP/keepalived failover detection
        // on some networks.
        out.push(SysctlLine::new("net.ipv4.arp_announce", "2"));
    }

    out
}

/// The portion of the file that gets hashed: sysctl lines, joined with
/// newlines and a trailing newline. The header (with the SHA inside it)
/// hashes only the body so the SHA is deterministic.
pub fn render_body(lines: &[SysctlLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.render());
        out.push('\n');
    }
    out
}

/// Render the standard managed-file header. SHA is the SHA-256 of the body
/// (everything after the header). `proteus diff` (phase G) can recompute
/// this hash from the file on disk and flag drift if it does not match.
pub fn render_header(body_sha_hex: &str) -> String {
    format!(
        "# managed by proteus v{version}\n\
         # do not edit; manage via /etc/proteus/config.toml or `proteus stack apply`\n\
         # sha256:{sha}\n",
        version = version::VERSION,
        sha = body_sha_hex
    )
}

/// Render the complete drop-in file (header + body) for the given config
/// + interfaces. Idempotent: same inputs → byte-identical output.
pub fn render_dropin(cfg: &StackConfig, ifaces: &[String]) -> String {
    let lines = lines_for(cfg, ifaces);
    let body = render_body(&lines);
    let sha = sha256::hex(body.as_bytes());
    let header = render_header(&sha);
    format!("{header}{body}")
}

/// Enumerate non-virtual, non-loopback interfaces by reading
/// `/sys/class/net/`. Mirrors the logic in `commands::status` but returns
/// just the names — the stack drop-in only needs them for the per-iface
/// NDP entries.
pub fn detect_managed_interfaces() -> Vec<String> {
    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("skip /sys/class/net: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        let base = entry.path();
        if let Ok(target) = std::fs::read_link(&base)
            && target.to_string_lossy().contains("devices/virtual")
        {
            continue;
        }
        out.push(name);
    }
    out.sort();
    out
}

/// Read the live value of a sysctl by translating its dotted name to the
/// equivalent `/proc/sys/...` path. Returns `None` if the key does not
/// exist on this kernel — that's a normal outcome on systems missing IPv6
/// or specific drivers.
pub fn read_sysctl(key: &str) -> Option<String> {
    let path = format!("/proc/sys/{}", key.replace('.', "/"));
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysctl_line_renders_as_key_equals_value() {
        let line = SysctlLine::new("net.ipv4.tcp_timestamps", "0");
        assert_eq!(line.render(), "net.ipv4.tcp_timestamps = 0");
    }

    #[test]
    fn lines_for_default_config_emits_full_baseline() {
        let cfg = StackConfig::default();
        let ifaces = vec!["wlan0".to_string(), "eth0".to_string()];
        let lines = lines_for(&cfg, &ifaces);
        let keys: Vec<&str> = lines.iter().map(|l| l.key.as_str()).collect();
        // tcp_timestamps + the four global ipv6 guards + per-iface NDP entries
        // (sorted: eth0 before wlan0).
        assert_eq!(
            keys,
            vec![
                "net.ipv4.tcp_timestamps",
                "net.ipv6.conf.all.accept_redirects",
                "net.ipv6.conf.default.accept_redirects",
                "net.ipv6.conf.all.accept_source_route",
                "net.ipv6.conf.default.accept_source_route",
                "net.ipv6.conf.eth0.ndisc_evict_nocarrier",
                "net.ipv6.conf.wlan0.ndisc_evict_nocarrier",
            ]
        );
    }

    #[test]
    fn arp_suppression_appears_when_opted_in() {
        let cfg = StackConfig {
            suppress_gratuitous_arp: true,
            ..StackConfig::default()
        };
        let lines = lines_for(&cfg, &[]);
        assert!(
            lines
                .iter()
                .any(|l| l.key == "net.ipv4.arp_announce" && l.value == "2"),
            "arp_announce=2 should appear when suppress_gratuitous_arp is on"
        );
    }

    #[test]
    fn arp_suppression_absent_by_default() {
        let cfg = StackConfig::default();
        let lines = lines_for(&cfg, &[]);
        assert!(
            !lines.iter().any(|l| l.key == "net.ipv4.arp_announce"),
            "arp_announce must not appear in the default rendering"
        );
    }

    #[test]
    fn ipv6_hardening_off_strips_global_and_per_iface() {
        let cfg = StackConfig {
            icmpv6_hardening: false,
            ..StackConfig::default()
        };
        let lines = lines_for(&cfg, &["wlan0".to_string()]);
        assert!(!lines.iter().any(|l| l.key.starts_with("net.ipv6.conf.")));
    }

    #[test]
    fn header_carries_version_and_sha() {
        let header = render_header("deadbeef");
        assert!(header.contains("# managed by proteus v"));
        assert!(header.contains(version::VERSION));
        assert!(header.contains("# sha256:deadbeef"));
        assert!(header.contains("manage via /etc/proteus/config.toml"));
    }

    #[test]
    fn render_dropin_is_idempotent_for_same_inputs() {
        let cfg = StackConfig::default();
        let ifaces = vec!["wlan0".to_string()];
        let a = render_dropin(&cfg, &ifaces);
        let b = render_dropin(&cfg, &ifaces);
        assert_eq!(a, b);
    }

    #[test]
    fn render_dropin_sha_matches_body() {
        let cfg = StackConfig::default();
        let ifaces = vec!["wlan0".to_string()];
        let body = render_body(&lines_for(&cfg, &ifaces));
        let expected = sha256::hex(body.as_bytes());
        let rendered = render_dropin(&cfg, &ifaces);
        assert!(
            rendered.contains(&format!("# sha256:{expected}")),
            "rendered drop-in must embed sha of the body"
        );
        // Body must also be present verbatim.
        assert!(rendered.ends_with(&body));
    }

    #[test]
    fn empty_iface_list_skips_per_iface_entries_only() {
        let cfg = StackConfig::default();
        let lines = lines_for(&cfg, &[]);
        // The four global IPv6 guards still appear.
        assert!(lines.iter().any(|l| l.key.contains("all.accept_redirects")));
        // No per-iface entries.
        assert!(
            !lines
                .iter()
                .any(|l| l.key.contains("ndisc_evict_nocarrier"))
        );
    }
}
