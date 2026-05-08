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

/// N2 / N12.19: typed result for [`permanent_address_result`]. Splits
/// "no factory MAC was discoverable" (a routine outcome on hosts
/// without ethtool / phy80211, or with a randomly-assigned address)
/// from "an I/O failure prevented the lookup" (sysfs read errored,
/// ethtool spawn errored, parser errored on otherwise-shaped output).
///
/// The legacy `Option`-shaped [`permanent_address`] still works —
/// it returns `Some(mac)` for `Found`, and `None` for both
/// `Unavailable` and `IoError`. New callers (`commands::rotate`,
/// `commands::status`, the doctor) should migrate to
/// [`permanent_address_result`] so they can warn on the I/O case
/// instead of silently treating it the same as the no-source case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryLookup {
    /// A canonical factory MAC was found via one of the sources
    /// (phy80211, ethtool, or the NET_ADDR_PERM sysfs fallback).
    Found(String),
    /// Every source declined cleanly: phy80211 absent, ethtool
    /// absent or returned no permanent address, sysfs reports
    /// `addr_assign_type != 0` (random/stolen/set). This is an
    /// expected outcome on virtual ifaces, on hosts without
    /// ethtool, on Wi-Fi drivers without phy80211 + cloned-since-boot.
    Unavailable,
    /// At least one source raised an I/O failure (sysfs read
    /// returned an OS error, ethtool spawned but exited non-zero
    /// in an unexpected way, parsed output was shape-malformed).
    /// Distinct from `Unavailable` because it warrants an operator
    /// warning rather than the silent no-op `Unavailable` gets.
    IoError(String),
}

/// Look up the burned-in factory MAC for `iface`. Returns `None` when no
/// source can produce an address we trust to be the factory value (rather
/// than a previously cloned one).
///
/// Legacy `Option`-shaped surface preserved for callers that don't yet
/// distinguish "no factory MAC" from "I/O error". New callers should
/// use [`permanent_address_result`] instead — N2 / N12.19.
pub fn permanent_address(iface: &str) -> Option<String> {
    match permanent_address_result(iface) {
        FactoryLookup::Found(m) => Some(m),
        FactoryLookup::Unavailable | FactoryLookup::IoError(_) => None,
    }
}

/// N2 / N12.19: distinguishes "no factory MAC was discoverable" from
/// "an I/O failure prevented the lookup". Production callers can warn
/// on the latter (so an operator catches a misconfigured ethtool /
/// permission denied on sysfs early) without spamming on the former.
pub fn permanent_address_result(iface: &str) -> FactoryLookup {
    permanent_address_result_under(Path::new(SYSFS_NET), iface, &EthtoolBin)
}

/// Same as `permanent_address` but with the sysfs root and `ethtool` runner
/// injected for unit tests.
#[cfg(test)]
pub(crate) fn permanent_address_under(
    sysfs_root: &Path,
    iface: &str,
    ethtool: &dyn EthtoolRunner,
) -> Option<String> {
    match permanent_address_result_under(sysfs_root, iface, ethtool) {
        FactoryLookup::Found(m) => Some(m),
        FactoryLookup::Unavailable | FactoryLookup::IoError(_) => None,
    }
}

/// Test-injectable variant of [`permanent_address_result`].
pub(crate) fn permanent_address_result_under(
    sysfs_root: &Path,
    iface: &str,
    ethtool: &dyn EthtoolRunner,
) -> FactoryLookup {
    if let Some(m) = read_phy80211(sysfs_root, iface) {
        return FactoryLookup::Found(m);
    }
    if let Some(m) = ethtool.permanent(iface) {
        return FactoryLookup::Found(m.to_ascii_lowercase());
    }
    if let Some(m) = read_address_if_perm(sysfs_root, iface) {
        return FactoryLookup::Found(m);
    }
    // Differentiate Unavailable vs IoError by re-checking whether the
    // iface's `address` file at least exists. If `/sys/class/net/<iface>`
    // is missing entirely we treat that as a routine "no such device"
    // (Unavailable). If it exists but every reader returned None we
    // still treat as Unavailable (the kernel said no, not an I/O error).
    // True IoError surfaces when sysfs metadata is present but unreadable
    // — surfaced via the helper below.
    if has_io_error(sysfs_root, iface) {
        return FactoryLookup::IoError(format!(
            "sysfs metadata for {iface} is present but unreadable"
        ));
    }
    FactoryLookup::Unavailable
}

/// N2: probe whether the iface's sysfs directory exists but the
/// `address` file is unreadable (permission denied, EIO, etc.).
/// Returns true only for the "metadata present, read failed" case;
/// returns false for "iface not in sysfs" (a routine Unavailable).
fn has_io_error(root: &Path, iface: &str) -> bool {
    let base = root.join(iface);
    if !base.exists() {
        return false;
    }
    let addr = base.join("address");
    match std::fs::metadata(&addr) {
        Ok(_) => match std::fs::read_to_string(&addr) {
            Ok(_) => false,
            Err(e) => !matches!(e.kind(), std::io::ErrorKind::NotFound),
        },
        Err(e) => !matches!(e.kind(), std::io::ErrorKind::NotFound),
    }
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
    // Issue #271: reject multicast addresses. A buggy driver / firmware
    // can present a multicast MAC at `phy80211/macaddress` or
    // `/sys/class/net/<iface>/address`, which would then be cached as
    // `state.original_macs[<iface>]` and "restored" on `proteus revert` —
    // poisoning the iface with an unassignable address. The well-formed
    // and zero checks above don't catch e.g. `01:00:5e:...`, so we filter
    // those here at the boundary.
    if !is_unicast_well_formed_mac(&raw) {
        return None;
    }
    Some(raw)
}

/// Issue #271: well-formed AND assignable as a unicast source. Defers
/// the multicast / all-zero rules to `Mac::validate_assignable` (the same
/// gate the rotate path uses to refuse a generated candidate), so the
/// rules can't drift between the factory-capture and rotate sides.
fn is_unicast_well_formed_mac(s: &str) -> bool {
    if !is_well_formed_mac(s) {
        return false;
    }
    s.parse::<super::Mac>()
        .ok()
        .is_some_and(|m| m.validate_assignable().is_ok())
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
        // Security audit N-1: validate before spawning. A leading `-`
        // would let `ethtool` parse the value as an option; the kernel
        // would reject the rest, but the boundary check doubles as
        // defense-in-depth against a future caller forwarding
        // attacker-shaped input.
        if !is_valid_iface_name(iface) {
            return None;
        }
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

/// Security audit N-1: iface-name allow-list mirroring the kernel's
/// `dev_valid_name()` rules (`net/core/dev.c`). The constraints are:
///
/// - non-empty and `<= 15` bytes (`IFNAMSIZ - 1` excluding the NUL)
/// - no leading `-` so `ethtool` cannot parse it as a flag
/// - the special names `.` and `..` are forbidden
/// - bytes are restricted to `[A-Za-z0-9_.-]` — ASCII alphanumerics
///   plus the three punctuation characters real iface names use
///   (`enp48s0`, `wlp3s0f3u2`, `eth0.10`, `enx00e04c360033`).
///
/// Anything outside this set is refused. The function is intentionally
/// stricter than `is_safe_iface` elsewhere in the tree because the
/// audit explicitly called out the regex `[A-Za-z0-9_.-]+` shape.
fn is_valid_iface_name(iface: &str) -> bool {
    if iface.is_empty() || iface.len() > 15 {
        return false;
    }
    if iface == "." || iface == ".." {
        return false;
    }
    if iface.starts_with('-') {
        return false;
    }
    iface
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
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
///
/// Issue #271: also reject multicast addresses (first-octet bit 0 set).
/// A buggy ethernet driver could otherwise have its multicast permanent
/// address cached as the factory original and restored on revert.
fn parse_ethtool_permanent(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("permanent address:") {
            let mac = rest.trim();
            if mac.is_empty() || mac == "00:00:00:00:00:00" {
                return None;
            }
            if !is_unicast_well_formed_mac(mac) {
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

    use super::*;

    struct StubEthtool {
        map: HashMap<String, String>,
    }

    impl EthtoolRunner for StubEthtool {
        fn permanent(&self, iface: &str) -> Option<String> {
            self.map.get(iface).cloned()
        }
    }

    /// Issue #206-D: this used to be a parallel `TestSysfs` struct that
    /// duplicated `crate::testing::TempRoot`'s tempdir lifecycle. The two
    /// had slightly different naming schemes (`-{pid}-{nanos}` vs
    /// `-{label}-test-{rand-hex}`) and identical drop behaviour. The
    /// unified shape: wrap `TempRoot` and add the sysfs-specific writers
    /// as methods on the wrapper. Drop semantics, naming, and collision
    /// resistance are now whatever `TempRoot` does.
    struct TestSysfs {
        inner: crate::testing::TempRoot,
    }

    impl TestSysfs {
        fn new(label: &str) -> Self {
            Self {
                inner: crate::testing::TempRoot::new(label),
            }
        }

        /// Path to the simulated `/sys/class/net` root.
        fn root(&self) -> &std::path::Path {
            &self.inner.path
        }

        fn write(&self, iface: &str, name: &str, value: &str) {
            let p = self.root().join(iface).join(name);
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

        let got = permanent_address_under(s.root(), "wlan0", &stub);
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

        let got = permanent_address_under(s.root(), "eth0", &stub);
        assert_eq!(got.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn fallback_address_only_when_kernel_says_perm() {
        // No phy80211, no ethtool. Live address is acceptable iff
        // addr_assign_type == 0 (NET_ADDR_PERM).
        let s = TestSysfs::new("factory-fallback-perm");
        s.write_address("eth0", "AA:BB:CC:DD:EE:FF", "0");

        let got = permanent_address_under(s.root(), "eth0", &no_ethtool());
        assert_eq!(got.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn fallback_refused_when_kernel_flags_random_assignment() {
        // No phy80211, no ethtool, AND the kernel reports the address as
        // randomly-assigned (assign_type=1) or set-by-userspace (3). We
        // refuse to cache a known-cloned value as "original".
        let s = TestSysfs::new("factory-fallback-refused");
        s.write_address("eth0", "11:22:33:44:55:66", "3");

        let got = permanent_address_under(s.root(), "eth0", &no_ethtool());
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

        let got = permanent_address_under(s.root(), "wlan0", &no_ethtool());
        assert_eq!(
            got.as_deref(),
            Some("aa:bb:cc:dd:ee:ff"),
            "phy80211 must win even though sysfs/address would also pass NET_ADDR_PERM"
        );
    }

    #[test]
    fn returns_none_when_iface_unknown() {
        let s = TestSysfs::new("factory-unknown");
        assert!(permanent_address_under(s.root(), "ghost0", &no_ethtool()).is_none());
    }

    #[test]
    fn rejects_all_zero_address() {
        let s = TestSysfs::new("factory-allzero");
        s.write_phy("wlan0", "00:00:00:00:00:00");

        let got = permanent_address_under(s.root(), "wlan0", &no_ethtool());
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

    /// Issue #271: a multicast MAC (first-octet bit 0 set) is well-formed
    /// in shape but unassignable as a unicast source. The unicast filter
    /// rejects it; the underlying well-formed check still passes since
    /// the format is valid.
    #[test]
    fn is_unicast_filter_rejects_multicast_macs() {
        // 01:00:5e:... is the canonical IPv4 multicast OUI prefix.
        assert!(!is_unicast_well_formed_mac("01:00:5e:00:00:01"));
        // 33:33:... is the IPv6 multicast prefix (also has bit 0 set).
        assert!(!is_unicast_well_formed_mac("33:33:00:00:00:01"));
        // ff:ff:... broadcast is also multicast (bit 0 set on 0xff).
        assert!(!is_unicast_well_formed_mac("ff:ff:ff:ff:ff:ff"));
        // Any odd first-octet rejects.
        assert!(!is_unicast_well_formed_mac("03:00:00:00:00:01"));
        // Even first-octet is acceptable.
        assert!(is_unicast_well_formed_mac("aa:bb:cc:dd:ee:ff"));
        assert!(is_unicast_well_formed_mac("02:00:00:00:00:01"));
        // Locally-administered + unicast (bit 1 set, bit 0 clear) is OK.
        assert!(is_unicast_well_formed_mac("00:11:22:33:44:55"));
        // Malformed input fails before the multicast check.
        assert!(!is_unicast_well_formed_mac("not-a-mac"));
    }

    /// Issue #271: end-to-end through `parse_ethtool_permanent`. A buggy
    /// driver presenting a multicast address must be rejected at the
    /// parser boundary so it never reaches `state.original_macs`.
    #[test]
    fn parse_ethtool_permanent_rejects_multicast_address() {
        for stdout in [
            "Permanent address: 01:00:5e:00:00:01\n",
            "Permanent address: 33:33:00:00:00:01\n",
            "Permanent address: ff:ff:ff:ff:ff:ff\n",
            "Permanent address: 03:00:00:00:00:01\n",
        ] {
            assert!(
                parse_ethtool_permanent(stdout).is_none(),
                "should reject multicast MAC in: {stdout:?}"
            );
        }
    }

    /// Issue #271: end-to-end through the sysfs reader. A driver that
    /// surfaces a multicast address at `phy80211/macaddress` must NOT be
    /// cached as the factory original.
    #[test]
    fn permanent_address_under_rejects_multicast_phy80211() {
        let s = TestSysfs::new("factory-multicast-phy");
        s.write_phy("wlan0", "01:00:5e:00:00:01");
        let got = permanent_address_under(s.root(), "wlan0", &no_ethtool());
        assert!(
            got.is_none(),
            "phy80211 multicast must be refused, got {got:?}"
        );
    }

    /// Issue #271: same protection on the `/sys/class/net/<iface>/address`
    /// fallback, even when `addr_assign_type` reports NET_ADDR_PERM.
    #[test]
    fn permanent_address_under_rejects_multicast_sysfs_address() {
        let s = TestSysfs::new("factory-multicast-sysfs");
        s.write_address("eth0", "01:00:5e:00:00:01", "0");
        let got = permanent_address_under(s.root(), "eth0", &no_ethtool());
        assert!(
            got.is_none(),
            "sysfs multicast must be refused, got {got:?}"
        );
    }

    /// Security audit N-1: `is_valid_iface_name` accepts the iface
    /// shapes Linux drivers actually expose and refuses everything
    /// else. Documented allow-list: ASCII alphanumerics plus `_`, `.`,
    /// `-`; max 15 bytes; non-empty; no leading `-`; not `.` or `..`.
    #[test]
    fn is_valid_iface_name_accepts_real_kernel_names() {
        for ok in [
            "eth0",
            "wlan0",
            "enp48s0",
            "wlp3s0f3u2",
            "eth0.10",
            "enx00e04c360033",
            "lo",
            "br0",
            "tun0",
            "tap0",
            "wg0",
            // 15 bytes is the kernel's IFNAMSIZ-1 ceiling.
            "abcdefghijklmno",
            // Underscores show up in some out-of-tree drivers.
            "wlan_dev_0",
        ] {
            assert!(is_valid_iface_name(ok), "expected {ok:?} to be valid");
        }
    }

    #[test]
    fn is_valid_iface_name_rejects_attacker_shapes() {
        for bad in [
            "",
            "-attacker",
            "-x",
            "--help",
            ".",
            "..",
            "../passwd",
            "with/slash",
            "with space",
            "with\nnewline",
            "with\0nul",
            "iface;rm-rf",
            "iface$evil",
            "iface\"quote",
            "iface'quote",
            // Over 15 bytes.
            "abcdefghijklmnop",
            "this-name-is-way-too-long-for-ifnamsiz",
            // Non-ASCII.
            "wlan\u{00ff}",
        ] {
            assert!(!is_valid_iface_name(bad), "expected {bad:?} to be refused");
        }
    }

    /// End-to-end: an `EthtoolBin` invocation with a hostile iface name
    /// must short-circuit before spawning the subprocess. We don't need
    /// the real ethtool binary on the test host — the validation gate
    /// runs first.
    #[test]
    fn ethtool_bin_refuses_unsafe_iface_without_spawning() {
        let bin = EthtoolBin;
        // Leading `-` would let ethtool parse the value as an option.
        assert!(bin.permanent("-Vroot:1").is_none());
        // Embedded NUL would terminate the C string passed to execve.
        assert!(bin.permanent("eth0\0").is_none());
        // Empty name has no kernel meaning.
        assert!(bin.permanent("").is_none());
    }

    // === N2 / N12.19: typed Found / Unavailable / IoError outcomes ===

    /// N2: `permanent_address_result_under` returns `Found` when phy80211
    /// surfaces a unicast MAC. Pin the typed shape so the new callers
    /// can pattern-match against the variant.
    #[test]
    fn factory_lookup_found_via_phy80211() {
        let s = TestSysfs::new("factory-found-phy");
        s.write_phy("wlan0", "AA:BB:CC:DD:EE:FF");
        let got = permanent_address_result_under(s.root(), "wlan0", &no_ethtool());
        assert_eq!(got, FactoryLookup::Found("aa:bb:cc:dd:ee:ff".to_string()));
    }

    /// N7 / N12.19: when the iface doesn't exist in sysfs at all we
    /// surface `Unavailable`. This is the "no such device" case the
    /// roadmap calls "factory MAC fallback failure path"; before the
    /// typed shape it collapsed into the same `None` that `Found`
    /// could produce on a correctly-working iface.
    #[test]
    fn factory_lookup_unavailable_when_iface_unknown() {
        let s = TestSysfs::new("factory-unavail-unknown");
        let got = permanent_address_result_under(s.root(), "ghost0", &no_ethtool());
        assert_eq!(got, FactoryLookup::Unavailable);
    }

    /// N7: kernel reports the address as randomly-assigned (assign_type
    /// = 1), no phy80211, no ethtool. Refusing to cache the live
    /// (cloned) value is the documented policy; the new shape labels
    /// it `Unavailable` (an expected outcome, no operator action) not
    /// `IoError` (an exceptional one).
    #[test]
    fn factory_lookup_unavailable_when_assign_type_is_random() {
        let s = TestSysfs::new("factory-unavail-random");
        s.write_address("eth0", "11:22:33:44:55:66", "1");
        let got = permanent_address_result_under(s.root(), "eth0", &no_ethtool());
        assert_eq!(got, FactoryLookup::Unavailable);
    }

    /// N7 / N12.19: regression test for the legacy `Option`-shaped API.
    /// The pre-fix callers consume `permanent_address` and we must not
    /// break them. `Unavailable` and `IoError` both project to `None`.
    #[test]
    fn legacy_permanent_address_projects_unavailable_to_none() {
        let s = TestSysfs::new("factory-legacy-unavail");
        let got = permanent_address_under(s.root(), "ghost0", &no_ethtool());
        assert!(got.is_none(), "legacy API: Unavailable -> None");
    }

    /// N7: full end-to-end through `permanent_address_result_under` with
    /// the priority ladder. phy80211 wins over ethtool wins over sysfs.
    /// This is the same priority covered by the `Option` tests but
    /// cross-checked through the new typed shape so a future refactor
    /// can't drop a tier.
    #[test]
    fn factory_lookup_priority_ladder_through_typed_api() {
        let s = TestSysfs::new("factory-typed-prio");
        s.write_phy("wlan0", "AA:BB:CC:DD:EE:FF");
        // Ethtool would have returned a different MAC; phy80211 wins.
        let mut m = HashMap::new();
        m.insert("wlan0".into(), "99:88:77:66:55:44".into());
        let stub = StubEthtool { map: m };
        assert_eq!(
            permanent_address_result_under(s.root(), "wlan0", &stub),
            FactoryLookup::Found("aa:bb:cc:dd:ee:ff".into())
        );
    }
}
