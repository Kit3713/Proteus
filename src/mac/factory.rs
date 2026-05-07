// SPDX-License-Identifier: GPL-3.0-or-later

//! Burned-in (factory) MAC capture.
//!
//! Issue #123: `state.original_macs` must record the BURNED-IN factory MAC,
//! not the live kernel-reported address. The kernel surfaces whatever the
//! current cloned MAC is at `/sys/class/net/<iface>/address`, so reading that
//! after a prior rotation captures a non-original value as the "original".
//! `proteus revert` would then "restore" to a value that is not the factory
//! address.
//!
//! Resolution order (each is tried in turn; first hit wins):
//!
//! 1. **`/sys/class/net/<iface>/phy80211/macaddress`** — present for every
//!    cfg80211/mac80211 Wi-Fi driver. Reports the wiphy's burned-in address;
//!    not affected by the netdev's cloned MAC.
//!
//! 2. **`ethtool -P <iface>`** — works for nearly every ethernet driver via
//!    `ETHTOOL_GPERMADDR`. We shell out rather than open a raw socket so the
//!    helper does not require `CAP_NET_ADMIN` at read time. Output line
//!    format: `Permanent address: xx:xx:xx:xx:xx:xx`.
//!
//! 3. **`/sys/class/net/<iface>/address`** — last-resort fallback. We only
//!    accept it when `addr_assign_type` reads as `0` (NET_ADDR_PERM), which
//!    means the kernel has not been told the address is randomly-assigned
//!    or stolen from another iface. If `addr_assign_type` is anything else
//!    (1=random, 2=stolen, 3=set), this path returns `None` so the caller
//!    can decide whether to leave `original_macs` empty rather than caching
//!    a known-cloned value.

use std::path::Path;
use std::process::Command;

/// Default sysfs root. Tests pass an alternate root via the `_under` variants.
const SYSFS_NET: &str = "/sys/class/net";

/// `addr_assign_type` value meaning "permanent / burned-in" per
/// `include/uapi/linux/netdevice.h` (`NET_ADDR_PERM`).
const NET_ADDR_PERM: &str = "0";

/// Look up the burned-in factory MAC for `iface`. Returns `None` when no
/// source can produce an address we trust to be the factory value (rather
/// than a previously cloned one).
pub fn permanent_address(iface: &str) -> Option<String> {
    permanent_address_under(Path::new(SYSFS_NET), iface, &EthtoolBin)
}

/// Same as `permanent_address` but with the sysfs root and `ethtool` runner
/// injected for unit tests.
pub(crate) fn permanent_address_under(
    sysfs_root: &Path,
    iface: &str,
    ethtool: &dyn EthtoolRunner,
) -> Option<String> {
    read_phy80211(sysfs_root, iface)
        .or_else(|| ethtool.permanent(iface).map(|s| s.to_ascii_lowercase()))
        .or_else(|| read_address_if_perm(sysfs_root, iface))
}

fn read_phy80211(root: &Path, iface: &str) -> Option<String> {
    read_mac_file(&root.join(iface).join("phy80211").join("macaddress"))
}

/// Live netdev address, accepted ONLY when the kernel reports
/// `addr_assign_type == NET_ADDR_PERM`. Anything else means the address has
/// been changed since boot and is not safe to cache as "original".
fn read_address_if_perm(root: &Path, iface: &str) -> Option<String> {
    let base = root.join(iface);
    if read_trim(&base.join("addr_assign_type"))? != NET_ADDR_PERM {
        return None;
    }
    read_mac_file(&base.join("address"))
}

fn read_mac_file(p: &Path) -> Option<String> {
    let raw = read_trim(p)?.to_ascii_lowercase();
    if raw == "00:00:00:00:00:00" || raw.is_empty() {
        return None;
    }
    Some(raw)
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_owned())
}

/// Abstraction so tests can stub `ethtool -P <iface>` without invoking the
/// real binary.
pub(crate) trait EthtoolRunner {
    fn permanent(&self, iface: &str) -> Option<String>;
}

/// Production implementation: shells out to `/usr/sbin/ethtool -P <iface>`.
///
/// Issue #202: pinned to the absolute path so a `$PATH` override on a
/// suid-helper or systemd-unit invocation can't redirect us to an
/// attacker-controlled binary. Matches the #121 hardening pattern. If
/// the absolute path doesn't resolve we fall back to the `$PATH` lookup
/// — a Nix or Alpine layout that ships ethtool elsewhere keeps working,
/// just without the absolute-path guarantee.
const ETHTOOL_ABS_PATH: &str = "/usr/sbin/ethtool";

pub(crate) struct EthtoolBin;

impl EthtoolRunner for EthtoolBin {
    fn permanent(&self, iface: &str) -> Option<String> {
        let bin = if Path::new(ETHTOOL_ABS_PATH).exists() {
            ETHTOOL_ABS_PATH
        } else {
            "ethtool"
        };
        let out = Command::new(bin).args(["-P", iface]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        parse_ethtool_permanent(&String::from_utf8_lossy(&out.stdout))
    }
}

/// Issue #206-E: validate that a candidate string is a colon-formatted MAC
/// (`xx:xx:xx:xx:xx:xx`, all lowercase hex, no surprises). Used by
/// [`parse_ethtool_permanent`] to refuse malformed driver output rather
/// than caching it as a factory address. The previous implementation only
/// rejected the all-zero variant.
fn is_well_formed_mac(s: &str) -> bool {
    if s.len() != 17 {
        return false;
    }
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i % 3 == 2 {
            if *b != b':' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Parse `Permanent address: xx:xx:xx:xx:xx:xx` (case-insensitive header).
/// `00:00:00:00:00:00` and any non-MAC-shaped string are rejected — the
/// former is what the driver reports when it doesn't actually expose a
/// permanent address; the latter is defence against a quirk where ethtool
/// prints translated text or a different layout. Issue #206-E.
fn parse_ethtool_permanent(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("permanent address:") {
            let mac = rest.trim();
            if mac.is_empty() || mac == "00:00:00:00:00:00" {
                return None;
            }
            if !is_well_formed_mac(mac) {
                return None;
            }
            return Some(mac.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    struct StubEthtool {
        map: HashMap<String, String>,
    }

    impl EthtoolRunner for StubEthtool {
        fn permanent(&self, iface: &str) -> Option<String> {
            self.map.get(iface).cloned()
        }
    }

    struct TestSysfs {
        root: PathBuf,
    }

    impl TestSysfs {
        fn new(label: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let root = std::env::temp_dir().join(format!("proteus-{label}-{pid}-{nanos}"));
            fs::create_dir_all(&root).expect("mkdir test root");
            Self { root }
        }

        fn write(&self, iface: &str, name: &str, value: &str) {
            let p = self.root.join(iface).join(name);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).expect("mkdir parent");
            }
            fs::write(&p, value).expect("write fixture");
        }

        fn write_phy(&self, iface: &str, mac: &str) {
            self.write(iface, "phy80211/macaddress", &format!("{mac}\n"));
        }

        fn write_address(&self, iface: &str, mac: &str, assign_type: &str) {
            self.write(iface, "address", &format!("{mac}\n"));
            self.write(iface, "addr_assign_type", &format!("{assign_type}\n"));
        }
    }

    impl Drop for TestSysfs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn no_ethtool() -> StubEthtool {
        StubEthtool {
            map: HashMap::new(),
        }
    }

    #[test]
    fn priority_order_phy80211_wins_over_ethtool_and_sysfs() {
        // Wi-Fi case: phy80211/macaddress is the burned-in wiphy address.
        // Even if `ethtool -P` and the live address say something different,
        // phy80211 must win.
        let s = TestSysfs::new("factory-prio-phy");
        s.write_phy("wlan0", "AA:BB:CC:DD:EE:FF");
        s.write_address("wlan0", "11:22:33:44:55:66", "3"); // cloned (random)
        let mut m = HashMap::new();
        m.insert("wlan0".to_string(), "99:88:77:66:55:44".to_string());
        let stub = StubEthtool { map: m };

        let got = permanent_address_under(&s.root, "wlan0", &stub);
        assert_eq!(got.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn priority_order_ethtool_wins_over_sysfs_when_phy_missing() {
        // Ethernet case: phy80211 doesn't exist, ethtool -P returns the
        // permanent address. Live address might be cloned — ethtool must
        // be preferred over the live-address fallback.
        let s = TestSysfs::new("factory-prio-ethtool");
        s.write_address("eth0", "11:22:33:44:55:66", "3");
        let mut m = HashMap::new();
        m.insert("eth0".to_string(), "AA:BB:CC:DD:EE:FF".to_string());
        let stub = StubEthtool { map: m };

        let got = permanent_address_under(&s.root, "eth0", &stub);
        assert_eq!(got.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn fallback_address_only_when_kernel_says_perm() {
        // No phy80211, no ethtool. Live address is acceptable iff
        // addr_assign_type == 0 (NET_ADDR_PERM).
        let s = TestSysfs::new("factory-fallback-perm");
        s.write_address("eth0", "AA:BB:CC:DD:EE:FF", "0");

        let got = permanent_address_under(&s.root, "eth0", &no_ethtool());
        assert_eq!(got.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn fallback_refused_when_kernel_flags_random_assignment() {
        // No phy80211, no ethtool, AND the kernel reports the address as
        // randomly-assigned (assign_type=1) or set-by-userspace (3). We
        // refuse to cache a known-cloned value as "original".
        let s = TestSysfs::new("factory-fallback-refused");
        s.write_address("eth0", "11:22:33:44:55:66", "3");

        let got = permanent_address_under(&s.root, "eth0", &no_ethtool());
        assert!(got.is_none(), "expected None, got {got:?}");
    }

    #[test]
    fn fallback_is_the_last_resort() {
        // Documents the tier order: when phy80211 is present, the live
        // address should NOT be consulted even if it would have passed
        // the addr_assign_type check.
        let s = TestSysfs::new("factory-last-resort");
        s.write_phy("wlan0", "AA:BB:CC:DD:EE:FF");
        s.write_address("wlan0", "11:22:33:44:55:66", "0");

        let got = permanent_address_under(&s.root, "wlan0", &no_ethtool());
        assert_eq!(
            got.as_deref(),
            Some("aa:bb:cc:dd:ee:ff"),
            "phy80211 must win even though sysfs/address would also pass NET_ADDR_PERM"
        );
    }

    #[test]
    fn returns_none_when_iface_unknown() {
        let s = TestSysfs::new("factory-unknown");
        assert!(permanent_address_under(&s.root, "ghost0", &no_ethtool()).is_none());
    }

    #[test]
    fn rejects_all_zero_address() {
        let s = TestSysfs::new("factory-allzero");
        s.write_phy("wlan0", "00:00:00:00:00:00");

        let got = permanent_address_under(&s.root, "wlan0", &no_ethtool());
        assert!(got.is_none());
    }

    #[test]
    fn parse_ethtool_permanent_extracts_mac() {
        let stdout = "Permanent address: aa:bb:cc:dd:ee:ff\n";
        assert_eq!(
            parse_ethtool_permanent(stdout).as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
    }

    #[test]
    fn parse_ethtool_permanent_handles_uppercase_header() {
        // Older ethtool prints `Permanent address:` with capital P; the parser
        // is case-insensitive for the header label.
        let stdout = "PERMANENT ADDRESS: AA:BB:CC:DD:EE:FF\n";
        assert_eq!(
            parse_ethtool_permanent(stdout).as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
    }

    #[test]
    fn parse_ethtool_permanent_rejects_all_zero() {
        let stdout = "Permanent address: 00:00:00:00:00:00\n";
        assert!(parse_ethtool_permanent(stdout).is_none());
    }

    #[test]
    fn parse_ethtool_permanent_returns_none_on_unrelated_output() {
        let stdout = "Some other ethtool line\n";
        assert!(parse_ethtool_permanent(stdout).is_none());
    }

    /// Issue #206-E: malformed values after the header don't pass through
    /// even if they're non-empty and non-zero. Defends against a quirky
    /// driver that prints "Permanent address: <unsupported>" or a
    /// translated string after the canonical header.
    #[test]
    fn parse_ethtool_permanent_rejects_non_mac_shaped_value() {
        for stdout in [
            "Permanent address: not-a-mac\n",
            "Permanent address: aa:bb:cc:dd:ee\n", // too short
            "Permanent address: aa:bb:cc:dd:ee:ff:00\n", // too long
            "Permanent address: aa-bb-cc-dd-ee-ff\n", // dashes, not colons
            "Permanent address: zz:bb:cc:dd:ee:ff\n", // non-hex
        ] {
            assert!(
                parse_ethtool_permanent(stdout).is_none(),
                "should reject: {stdout:?}"
            );
        }
    }

    #[test]
    fn is_well_formed_mac_accepts_canonical_form() {
        assert!(is_well_formed_mac("aa:bb:cc:dd:ee:ff"));
        assert!(is_well_formed_mac("00:11:22:33:44:55"));
    }
}
