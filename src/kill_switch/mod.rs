// SPDX-License-Identifier: GPL-3.0-or-later

//! Network kill switch — emergency network shutdown.
//!
//! This module enumerates managed interfaces from `/sys/class/net`, brings
//! them down via `ip link set <iface> down`, and disables NetworkManager
//! radios (Wi-Fi / WWAN) plus BlueZ adapters via DBus.
//!
//! "Kill" means: drop all packets at L2 — interfaces administratively down,
//! radios off. That is stronger than any L3 firewall rule because the kernel
//! never even sees frames on the radio. In-flight TCP/TLS sessions just
//! time out; nothing leaks while the switch is active.
//!
//! State is recorded under `state.kill_switch` so `proteus resume` knows
//! what to bring back up. The set of interfaces is captured at kill time,
//! so plugging in a new device after activating the switch will not
//! auto-disable it (which is fine — `proteus kill` again to extend).
//!
//! Pure functions live in this module so we can unit-test the interface
//! filter without touching netlink. The real work happens in
//! `commands::kill`.
//!
//! See `proteus wiki kill-switch` for the operator-facing doc.
//!
//! Mission scope: this is squarely a network-layer fingerprint defense.
//! When you do not trust the environment (suspected compromise, border
//! crossing, hostile hotspot), having one command that gets your laptop
//! off every wire and every radio is the safety hatch. It does not pretend
//! to be a hardening tool — see the wiki page for the boundaries.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Snapshot of every interface the kill switch acted on. Persisted so
/// `proteus resume` can restore exactly the set we brought down rather
/// than guessing at runtime (which would silently miss a USB-Ethernet
/// adapter that was unplugged between kill and resume).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KillSwitchState {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<String>,
    pub interfaces: Vec<String>,
    pub nm_wireless_disabled: bool,
    pub nm_wwan_disabled: bool,
    pub bluetooth_disabled: bool,
}

/// Where to read interface metadata. Overridable for tests so we never
/// touch `/sys` from a unit test.
pub const SYSFS_NET: &str = "/sys/class/net";

/// Names we never bring down. `lo` keeps localhost services functional
/// (the entire point of a kill switch is L2/L3 isolation, not breaking
/// the loopback for local IPC). The other prefixes are virtual / runtime
/// interfaces that are not "yours" in any meaningful sense.
pub const SKIP_PREFIXES: &[&str] = &[
    "lo",
    "docker",
    "podman",
    "veth",
    "virbr",
    "br-",
    "tun",
    "tap",
    "tailscale",
    "wg",
    "zt",
    "kube",
    "cni",
];

/// Filter `/sys/class/net` down to interfaces we should bring down. Public
/// (and pure) so it is unit-testable — the live function `enumerate_managed`
/// just adds the directory listing on top.
pub fn should_manage(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name == "lo" {
        return false;
    }
    for p in SKIP_PREFIXES {
        if name.starts_with(p) {
            return false;
        }
    }
    true
}

/// Walk `/sys/class/net` and return the names of interfaces the kill
/// switch should bring down. Quietly skips everything in `SKIP_PREFIXES`
/// plus virtual interfaces (`/sys/class/net/<n>/device` missing).
pub fn enumerate_managed(sysfs_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(sysfs_root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !should_manage(&name) {
            continue;
        }
        // A backing device link (PCI, USB, etc.) is the cleanest signal that
        // this is a real NIC rather than a software-only interface — virtual
        // bridges, dummy interfaces, and the like never have one. Use
        // `symlink_metadata` so we detect the link itself rather than asking
        // whether the link's target exists (which would skip dangling
        // symlinks under sysfs in some distros and in our tests).
        let device_link = sysfs_root.join(&name).join("device");
        if fs::symlink_metadata(&device_link).is_err() {
            continue;
        }
        out.push(name);
    }
    out.sort();
    out
}

/// Bring a single interface down via `ip link set <iface> down`. Returns
/// `Ok(true)` on success, `Ok(false)` if `ip` is missing (caller can
/// surface a remediation), and the captured stderr otherwise.
pub fn link_down(iface: &str) -> Result<bool, String> {
    run_ip(&["link", "set", iface, "down"])
}

/// Bring a single interface back up via `ip link set <iface> up`.
pub fn link_up(iface: &str) -> Result<bool, String> {
    run_ip(&["link", "set", iface, "up"])
}

fn run_ip(args: &[&str]) -> Result<bool, String> {
    let output = match Command::new("ip").args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("spawning ip: {e}")),
    };
    if output.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("ip {} exited non-zero", args.join(" ")))
        } else {
            Err(stderr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn should_manage_skips_loopback_and_virtuals() {
        assert!(!should_manage("lo"));
        assert!(!should_manage("docker0"));
        assert!(!should_manage("docker_gwbridge"));
        assert!(!should_manage("podman0"));
        assert!(!should_manage("veth1234"));
        assert!(!should_manage("virbr0"));
        assert!(!should_manage("br-12345"));
        assert!(!should_manage("tun0"));
        assert!(!should_manage("tap0"));
        assert!(!should_manage("tailscale0"));
        assert!(!should_manage("wg0"));
        assert!(!should_manage(""));
    }

    #[test]
    fn should_manage_keeps_real_nics() {
        assert!(should_manage("wlan0"));
        assert!(should_manage("wlo1"));
        assert!(should_manage("eth0"));
        assert!(should_manage("enp0s3"));
        assert!(should_manage("enp48s0"));
        assert!(should_manage("eno1"));
    }

    #[test]
    fn enumerate_managed_picks_up_devices_with_backing_link() {
        let root = std::env::temp_dir().join("proteus-kill-enum-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // wlan0: real NIC (has a `device` link).
        let wlan = root.join("wlan0");
        fs::create_dir_all(&wlan).unwrap();
        // Symlink target need not exist for symlink_metadata to find it; this
        // mirrors the way `/sys/class/net/<iface>/device` shows up.
        symlink("../../devices/pci0000:00", wlan.join("device")).unwrap();

        // virtual0: no `device` link — should be skipped.
        fs::create_dir_all(root.join("virtual0")).unwrap();

        // lo: name-blacklisted even with a fake link.
        let lo = root.join("lo");
        fs::create_dir_all(&lo).unwrap();
        symlink("../../devices/loopback", lo.join("device")).unwrap();

        // tailscale0: prefix-blacklisted.
        let ts = root.join("tailscale0");
        fs::create_dir_all(&ts).unwrap();
        symlink("../../devices/tap", ts.join("device")).unwrap();

        let found = enumerate_managed(&root);
        assert_eq!(found, vec!["wlan0".to_string()]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn kill_switch_state_round_trips_through_serde() {
        let s = KillSwitchState {
            active: true,
            activated_at: Some("2026-05-06T12:34:56Z".to_string()),
            interfaces: vec!["wlan0".to_string(), "eth0".to_string()],
            nm_wireless_disabled: true,
            nm_wwan_disabled: false,
            bluetooth_disabled: true,
        };
        let bytes = serde_json::to_vec(&s).unwrap();
        let back: KillSwitchState = serde_json::from_slice(&bytes).unwrap();
        assert!(back.active);
        assert_eq!(back.activated_at.as_deref(), Some("2026-05-06T12:34:56Z"));
        assert_eq!(back.interfaces, vec!["wlan0", "eth0"]);
        assert!(back.nm_wireless_disabled);
        assert!(!back.nm_wwan_disabled);
        assert!(back.bluetooth_disabled);
    }

    #[test]
    fn kill_switch_state_default_is_inactive_and_empty() {
        let s = KillSwitchState::default();
        assert!(!s.active);
        assert!(s.activated_at.is_none());
        assert!(s.interfaces.is_empty());
        assert!(!s.nm_wireless_disabled);
        assert!(!s.nm_wwan_disabled);
        assert!(!s.bluetooth_disabled);
    }
}
