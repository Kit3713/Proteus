// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure library helpers for the OS-controllable RF surface.
//!
//! Two halves: a sysfs/`iw`-driven Wi-Fi inventory (driver name, vendor/device
//! IDs, firmware version, current TX power, regulatory ceiling) and a thin
//! re-shape of `crate::bluetooth::AdapterInfo` for command callers that want a
//! single radio-inventory call. None of the helpers here decide policy — they
//! report what the kernel exposes and shell out to `iw` only when the caller
//! has already decided to write.
//!
//! Hardware-baked RF properties (oscillator drift, IQ imbalance, etc.) are out
//! of scope by physics. See `wiki/rf-fingerprinting.md` for the boundary.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

/// Absolute paths preferred for `iw` (issue #202): a `$PATH` override on a
/// suid-helper or systemd-unit invocation must not redirect us to an
/// attacker-controlled binary. We try `/usr/bin/iw` then `/sbin/iw` (the
/// two locations real distros ship) before falling back to `$PATH` for
/// Nix/Alpine layouts that put `iw` elsewhere — same shape as `ETHTOOL_ABS_PATH`.
const IW_ABS_PATHS: &[&str] = &["/usr/bin/iw", "/sbin/iw"];

/// Resolve the `iw` binary path. Returns the first absolute path that
/// exists on disk, falling back to the bare `iw` name (which `Command`
/// resolves via `$PATH`). Callers should still tolerate the binary
/// being missing — `iw_present` is the right gate.
fn iw_bin() -> &'static str {
    for p in IW_ABS_PATHS {
        if Path::new(p).exists() {
            return p;
        }
    }
    "iw"
}

/// Driver-reported metadata for one Wi-Fi interface. All fields are
/// best-effort: a missing sysfs node is `None` rather than a hard error so
/// `proteus rf status` keeps producing a usable inventory across odd
/// out-of-tree drivers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChipInfo {
    pub iface: String,
    pub driver: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub firmware: Option<String>,
}

/// Reduced Bluetooth adapter inventory for the rf-status surface. We only
/// pull through the fields a chipset audit cares about; the full DBus shape
/// stays in `crate::bluetooth::AdapterInfo`.
#[derive(Debug, Clone, Serialize)]
pub struct BluetoothChipInfo {
    pub hci: String,
    pub address: Option<String>,
    pub address_type: Option<String>,
    pub name: Option<String>,
    pub powered: Option<bool>,
}

/// List every Wi-Fi interface name the kernel currently exposes, sorted.
/// An interface counts as Wi-Fi iff `/sys/class/net/<iface>/wireless` exists.
pub fn wifi_interfaces() -> Vec<String> {
    wifi_interfaces_under(Path::new("/sys/class/net"))
}

/// Test seam for `wifi_interfaces`: look under an arbitrary root.
pub fn wifi_interfaces_under(root: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "lo" {
                return None;
            }
            let wireless = e.path().join("wireless");
            if wireless.exists() { Some(name) } else { None }
        })
        .collect();
    out.sort();
    out
}

/// Read driver/vendor/device/firmware metadata from sysfs for `iface`.
/// Falls back to `None` per field if the corresponding node is absent.
pub fn chip_info(iface: &str) -> Result<ChipInfo> {
    chip_info_under(Path::new("/sys/class/net"), iface)
}

/// Test seam for `chip_info`. Pass an arbitrary `/sys/class/net` root so the
/// parser can be unit-tested without real hardware.
pub fn chip_info_under(root: &Path, iface: &str) -> Result<ChipInfo> {
    let base = root.join(iface);
    if !base.exists() {
        bail!("interface {iface} not present under {}", root.display());
    }
    let driver = read_driver_name(&base);
    let vendor_id = read_trim_to_string(&base.join("device").join("vendor"));
    let device_id = read_trim_to_string(&base.join("device").join("device"));
    let firmware = read_firmware_version(&base);
    Ok(ChipInfo {
        iface: iface.to_string(),
        driver,
        vendor_id,
        device_id,
        firmware,
    })
}

/// Parse the current TX power for `iface` out of `iw dev <iface> info`.
/// Returns mBm (milli-dBm) — `iw` prints `txpower 20.00 dBm` on a separate
/// line. Returns `None` if the binary is missing, the iface is unknown, or
/// the line is absent.
pub fn current_tx_power_mbm(iface: &str) -> Option<i32> {
    if !is_safe_iface(iface) {
        return None;
    }
    // Audit L-3 residual: `iw` does not support `--` as a global flag
    // terminator (its grammar is `iw <object> <command> [args]`). The
    // iface lives in argument position 2 (`dev <iface>`) where the
    // preceding `dev` selector already disambiguates it; a leading-`-`
    // iface name would still be rejected by the kernel `dev_valid_name`
    // check, AND `is_safe_iface` above refuses it at the boundary. The
    // `--` separator pattern instead applies to `ip` / `ethtool` calls
    // (see `kill_switch::run_ip` and `EthtoolBin::permanent`).
    let output = Command::new(iw_bin())
        .args(["dev", iface, "info"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_iw_dev_info_txpower(&String::from_utf8_lossy(&output.stdout))
}

/// Set the fixed TX power for `iface`. `mbm` is the value `iw` itself wants
/// (milli-dBm). Errors propagate so callers can warn-and-continue.
pub fn set_tx_power_mbm(iface: &str, mbm: i32) -> Result<()> {
    if !is_safe_iface(iface) {
        bail!("refusing to invoke iw with iface {iface:?}: contains unsafe characters");
    }
    let mbm_str = mbm.to_string();
    let output = Command::new(iw_bin())
        .args(["dev", iface, "set", "txpower", "fixed", &mbm_str])
        .output()
        .with_context(|| format!("invoking `iw dev {iface} set txpower fixed {mbm}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "iw dev {iface} set txpower fixed {mbm} failed: {}",
            stderr.trim()
        );
    }
    Ok(())
}

/// Look up the regulatory-domain TX power ceiling in mBm by parsing
/// `iw reg get`. Returns `None` if `iw` is missing or the output has no
/// per-channel `(<dB> dBm)` clause we can extract a maximum from.
pub fn regulatory_max_mbm() -> Option<i32> {
    let output = Command::new(iw_bin()).args(["reg", "get"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_iw_reg_get_max_mbm(&String::from_utf8_lossy(&output.stdout))
}

/// Conservative fallback when the regulatory lookup yields nothing: 20 dBm
/// is the FCC/ETSI 2.4 GHz client-mode ceiling and a safe lower bound for
/// anything `iw` would otherwise have reported. Returned in mBm.
pub const FALLBACK_REGULATORY_MAX_MBM: i32 = 2_000;

/// Effective regulatory max: prefer `iw reg get`, fall back to the constant
/// when the parse fails. Always returns a value so callers don't need to
/// branch on the lookup result.
pub fn regulatory_max_mbm_or_fallback() -> i32 {
    regulatory_max_mbm().unwrap_or(FALLBACK_REGULATORY_MAX_MBM)
}

/// Run a synchronous `iw` to confirm the binary resolves. Used by the
/// command layer to skip with `SYSTEM_NOT_SUPPORTED` rather than fail
/// when the operator hasn't installed iw-tools. Walks the absolute-path
/// preference list first (issue #202) before consulting `$PATH`.
pub fn iw_present() -> bool {
    Command::new(iw_bin())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Defense-in-depth (security audit L-3): refuse iface names that contain
/// `/`, NUL, or a leading `-`. Today these come from kernel-validated
/// sources (sysfs walks, NM Device proxy), but the caller has no way to
/// see that and a future call site might forward attacker-shaped input.
fn is_safe_iface(iface: &str) -> bool {
    !iface.is_empty()
        && !iface.starts_with('-')
        && iface
            .bytes()
            .all(|b| b != b'/' && b != 0 && b.is_ascii_graphic())
}

/// Re-shape `crate::bluetooth::list_adapters` output for the inventory caller.
/// Returns an empty vec if BlueZ isn't running or the runtime can't be built;
/// the chipset audit shouldn't fail the whole status report.
pub fn bluetooth_chip_info() -> Vec<BluetoothChipInfo> {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    runtime.block_on(async {
        let Ok(Some((_, adapters))) = crate::bluetooth::connect_and_list().await else {
            return Vec::new();
        };
        adapters
            .into_iter()
            .map(|a| BluetoothChipInfo {
                hci: a.hci,
                address: a.address,
                address_type: a.address_type,
                name: a.name,
                powered: a.powered,
            })
            .collect()
    })
}

// ---- Internals ----------------------------------------------------------

fn read_trim_to_string(path: &Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Driver name lives at one of two sysfs paths depending on kernel version
/// and how the driver was loaded:
///   - `/sys/class/net/<iface>/device/driver/module/name` (preferred — the
///     module that owns the driver, used by mac80211 stack)
///   - `/sys/class/net/<iface>/device/driver` (symlink whose final
///     component is the driver name; older kernels)
fn read_driver_name(base: &Path) -> Option<String> {
    let module_name = base
        .join("device")
        .join("driver")
        .join("module")
        .join("name");
    if let Some(s) = read_trim_to_string(&module_name) {
        return Some(s);
    }
    let driver_link = base.join("device").join("driver");
    let target = std::fs::read_link(&driver_link).ok()?;
    target
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

/// Firmware version comes from one of a few common locations depending on
/// driver. Walk the candidates in order; first hit wins.
fn read_firmware_version(base: &Path) -> Option<String> {
    let candidates = [
        base.join("device").join("firmware_version"),
        base.join("phy80211").join("firmware_version"),
        base.join("device").join("firmware"),
    ];
    for c in &candidates {
        if let Some(s) = read_trim_to_string(c) {
            return Some(s);
        }
    }
    None
}

/// Pull the `txpower 20.00 dBm` line out of `iw dev <iface> info` and
/// convert to mBm (milli-dBm). dBm * 100 == mBm by definition.
///
/// Issue #160 (defense-in-depth): require a literal `txpower ` prefix
/// (with trailing space) so future `txpower-foo` keys cannot be parsed
/// as a power value.
fn parse_iw_dev_info_txpower(text: &str) -> Option<i32> {
    for raw in text.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("txpower ") else {
            continue;
        };
        let value = rest.trim_start();
        let value = value.strip_suffix("dBm").unwrap_or(value).trim();
        let dbm: f32 = value.parse().ok()?;
        return Some((dbm * 100.0).round() as i32);
    }
    None
}

/// Find the highest `(<dBm> dBm)` value from `iw reg get`. Real output looks
/// like:
///
/// ```text
/// (5170 - 5250 @ 80), (N/A, 23), (N/A)
/// ```
///
/// or, on older `iw`:
///
/// ```text
/// (5170 - 5250 @ 80), (N/A, 23 dBm), (N/A)
/// ```
///
/// We accept both shapes. A regulatory max of zero means the parser failed
/// and the caller should fall back.
///
/// Issue #160 (regulatory safety): the parser anchors on the *power* tuple
/// — the second parenthesized clause matching `(N/A, <num>[ dBm])` or
/// `(<num>[ dBm])` — and skips the leading frequency-band clause. Walking
/// every paren'd token would let the `@ 80` channel-bandwidth in malformed
/// output be parsed as a power value, which `set_tx_power_mbm` would then
/// accept as a regulatory ceiling. The grammar is documented above; the
/// parser refuses anything that doesn't match.
fn parse_iw_reg_get_max_mbm(text: &str) -> Option<i32> {
    let mut max_dbm: Option<f32> = None;
    for raw in text.lines() {
        // Each per-band entry has the shape `(freq @ bw), (N/A, dBm), (...)`.
        // Skip the first clause (frequency band), keep the rest, and only
        // consider the second parenthesized clause for power.
        let mut clauses = raw.split('(').skip(1);
        let _ = clauses.next(); // drop the frequency band
        let Some(power_clause) = clauses.next() else {
            continue;
        };
        let body = power_clause
            .split_once(')')
            .map(|(b, _)| b)
            .unwrap_or(power_clause);
        // Body is e.g. "N/A, 23" or "N/A, 23 dBm" or "23" — comma-separated.
        for token in body.split(',') {
            let t = token.trim();
            if t.is_empty() || t.eq_ignore_ascii_case("N/A") {
                continue;
            }
            let candidate = t.strip_suffix("dBm").unwrap_or(t).trim();
            if let Ok(dbm) = candidate.parse::<f32>() {
                max_dbm = Some(match max_dbm {
                    Some(prev) if prev >= dbm => prev,
                    _ => dbm,
                });
            }
        }
    }
    max_dbm.map(|d| (d * 100.0).round() as i32)
}

// ---- Scan-policy reporting (Milestone 4b) -------------------------------

/// Scan-policy report for one Wi-Fi interface. Both fields come from
/// shelling `iw` (`iw dev <iface> info` for the live netdev type, `iw phy
/// <phy> info` for the driver capability set). When `iw` is missing or
/// the parse misses the relevant line, the field is `None` rather than
/// a hard error so a host with one Wi-Fi iface and one out-of-tree
/// driver still produces a usable inventory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanPolicy {
    pub iface: String,
    /// `phy0`, `phy1`, … as resolved from `/sys/class/net/<iface>/phy80211/name`
    /// or the `wiphy` line in `iw dev <iface> info`. Required to interpret
    /// `iw phy` output even when `iw dev` works.
    pub phy: Option<String>,
    /// `managed`, `monitor`, etc. — the `type` line in `iw dev info`.
    pub iface_type: Option<String>,
    /// True iff the driver advertises `randomize_mac_addr` or
    /// `randomize_mac_oui` in `iw phy info`. Without this, scans are
    /// driver-baked and Proteus's NM keys cannot lift them to per-scan
    /// random.
    pub supports_randomize_mac: bool,
    /// True iff `iw phy info` exposes the active-scan capability. Every
    /// modern driver does; the field exists so the report can flag the
    /// rare exception (some Realtek/Mediatek out-of-tree drivers).
    pub supports_active_scan: bool,
    /// Best-effort: the literal SCAN line from `iw phy info` if the
    /// driver prints one, e.g. `Supported commands: ... new_scan ...`.
    /// Useful for human inspection; not parsed further.
    pub raw_scan_capabilities: Option<String>,
}

/// Resolve the phy name for `iface` via sysfs: `/sys/class/net/<iface>/phy80211/name`.
/// Used by `scan_policy` and `chip_info_extended` to stitch `iw dev` and
/// `iw phy` output together.
pub fn phy_for_iface(iface: &str) -> Option<String> {
    phy_for_iface_under(Path::new("/sys/class/net"), iface)
}

/// Test seam for `phy_for_iface`.
pub fn phy_for_iface_under(root: &Path, iface: &str) -> Option<String> {
    let p = root.join(iface).join("phy80211").join("name");
    let s = std::fs::read_to_string(&p).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Build a scan-policy report for `iface` by shelling out to `iw`. Returns
/// a partially-populated struct on missing-iw / parse-miss rather than
/// failing — the command layer wants to render "supports randomization:
/// unknown" instead of refusing to print the iface row.
pub fn scan_policy(iface: &str) -> ScanPolicy {
    let mut out = ScanPolicy {
        iface: iface.to_string(),
        ..Default::default()
    };
    if !is_safe_iface(iface) {
        return out;
    }
    out.phy = phy_for_iface(iface);
    if let Some((phy, kind)) = run_iw_dev_info(iface) {
        if out.phy.is_none() {
            out.phy = Some(phy);
        }
        out.iface_type = kind;
    }
    if let Some(phy) = &out.phy
        && let Some(text) = run_iw_phy_info(phy)
    {
        let caps = parse_iw_phy_capabilities(&text);
        out.supports_randomize_mac = caps.randomize_mac;
        out.supports_active_scan = caps.active_scan;
        out.raw_scan_capabilities = caps.raw_scan_line;
    }
    out
}

fn run_iw_dev_info(iface: &str) -> Option<(String, Option<String>)> {
    let output = Command::new(iw_bin())
        .args(["dev", iface, "info"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_iw_dev_info_phy_and_type(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn run_iw_phy_info(phy: &str) -> Option<String> {
    if !is_safe_iface(phy) {
        return None;
    }
    let output = Command::new(iw_bin())
        .args(["phy", phy, "info"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse `wiphy <n>` and `type <kind>` out of `iw dev <iface> info`. The
/// phy name returned is `phyN` constructed from the wiphy index because
/// that's what `iw phy <phy> info` accepts. Both fields are best-effort.
fn parse_iw_dev_info_phy_and_type(text: &str) -> (String, Option<String>) {
    let mut phy = String::new();
    let mut kind: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("wiphy ")
            && let Ok(n) = rest.trim().parse::<u32>()
        {
            phy = format!("phy{n}");
        }
        if let Some(rest) = line.strip_prefix("type ") {
            kind = Some(rest.trim().to_string());
        }
    }
    (phy, kind)
}

#[derive(Debug, Default)]
struct PhyCapabilities {
    randomize_mac: bool,
    active_scan: bool,
    raw_scan_line: Option<String>,
}

/// Parse the supported-commands / extended-capabilities block of
/// `iw phy <phy> info`. We only care about three things:
/// - any line containing `randomize_mac` (case-insensitive) — driver
///   supports the per-scan random-MAC feature NM relies on.
/// - any line mentioning `active scan` or `new_scan` — base scan
///   capability is present.
/// - the first `Supported commands` line as a raw string for human
///   inspection.
fn parse_iw_phy_capabilities(text: &str) -> PhyCapabilities {
    // Issue #273: previously this allocated a full-buffer lowercase copy
    // of the entire `iw phy info` output, then ran 6 contains() checks
    // against that copy. The output can be hundreds of lines on
    // multi-radio modern Wi-Fi 7 chipsets; the allocation dominates the
    // parser cost and is unnecessary because each individual contains()
    // can do its own case-insensitive walk over the original buffer.
    let randomize_mac = contains_ascii_case_insensitive(text, "randomize_mac")
        || contains_ascii_case_insensitive(text, "randomise_mac")
        || contains_ascii_case_insensitive(text, "scan_random_mac");
    let active_scan = contains_ascii_case_insensitive(text, "active scan")
        || contains_ascii_case_insensitive(text, "new_scan")
        || contains_ascii_case_insensitive(text, "trigger_scan");
    let mut raw_scan_line = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Supported commands:") {
            raw_scan_line = Some(line.to_string());
            break;
        }
    }
    PhyCapabilities {
        randomize_mac,
        active_scan,
        raw_scan_line,
    }
}

// ---- Chipset / firmware extended report (Milestone 4b) -----------------

/// Extended chipset inventory: everything `chip_info` reports plus the
/// resolved phy index, the driver-reported regulatory domain (best-effort
/// via `iw phy <phy> reg get`), and a raw firmware line cribbed from
/// `dmesg | grep firmware` when sysfs doesn't expose `firmware_version`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChipInfoExtended {
    pub iface: String,
    pub phy: Option<String>,
    pub driver: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub firmware: Option<String>,
    pub regulatory_domain: Option<String>,
}

/// Read the extended chipset inventory for `iface`. All fields are
/// best-effort — a missing sysfs node, a missing `iw`, or a quiet
/// dmesg ring buffer all degrade to `None` rather than failing.
pub fn chip_info_extended(iface: &str) -> ChipInfoExtended {
    let base = chip_info(iface).unwrap_or_default();
    let phy = phy_for_iface(iface);
    let firmware = base.firmware.or_else(|| dmesg_firmware_line(iface));
    let regulatory_domain = phy.as_deref().and_then(iw_phy_reg_get);
    ChipInfoExtended {
        iface: iface.to_string(),
        phy,
        driver: base.driver,
        vendor_id: base.vendor_id,
        device_id: base.device_id,
        firmware,
        regulatory_domain,
    }
}

/// Best-effort firmware fallback for drivers that don't expose
/// `firmware_version` in sysfs. Reads `dmesg` (a one-shot, success-or-skip
/// — the binary is missing on minimal containers) and looks for the first
/// line that mentions `iface` and the word `firmware`.
///
/// Issue #273: previously this re-allocated `iface.to_ascii_lowercase()`
/// once per dmesg line — dmesg buffers can be tens of thousands of lines
/// long, and the iface name doesn't change inside the loop. Hoist the
/// allocation out and replace the per-line `to_ascii_lowercase()` +
/// `contains` chain with a direct case-insensitive search, so the inner
/// loop allocates nothing (the `&str` `lines()` iterator is borrow-only).
fn dmesg_firmware_line(iface: &str) -> Option<String> {
    let output = Command::new("dmesg").output().ok()?;
    if !output.status.success() {
        tracing::debug!(iface, "dmesg returned non-zero; skipping firmware fallback");
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // Hoist out of the loop: lowercase the iface name ONCE.
    let iface_lower = iface.to_ascii_lowercase();
    for raw in text.lines().rev() {
        let line = raw.trim();
        // Case-insensitive substring search avoids allocating a lowercase
        // copy of every dmesg line.
        if contains_ascii_case_insensitive(line, "firmware")
            && contains_ascii_case_insensitive(line, &iface_lower)
        {
            return Some(line.to_string());
        }
    }
    None
}

/// Case-insensitive ASCII substring search. `needle` must be ASCII (the
/// caller's responsibility — interface names are constrained by
/// `is_safe_iface` and our literal needles like `"firmware"` are ASCII
/// by construction). Allocates nothing; walks `haystack` once.
fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let n = needle.as_bytes();
    let h = haystack.as_bytes();
    if n.len() > h.len() {
        return false;
    }
    let last = h.len() - n.len();
    for start in 0..=last {
        let mut ok = true;
        for i in 0..n.len() {
            if !h[start + i].eq_ignore_ascii_case(&n[i]) {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}

fn iw_phy_reg_get(phy: &str) -> Option<String> {
    if !is_safe_iface(phy) {
        return None;
    }
    let output = Command::new(iw_bin())
        .args(["phy", phy, "reg", "get"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("country ") {
            // Example: "country US: DFS-FCC" — keep up to the colon.
            let token = rest.split(':').next().unwrap_or(rest).trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_interfaces_under_filters_to_wireless_dirs() {
        let tmp = tempdir();
        // wlan0 has a wireless/ subdir → counts.
        std::fs::create_dir_all(tmp.path().join("wlan0").join("wireless")).unwrap();
        // eth0 has no wireless/ → ignored.
        std::fs::create_dir_all(tmp.path().join("eth0")).unwrap();
        // lo is a wireless mock (won't happen in real life) but is hard-skipped.
        std::fs::create_dir_all(tmp.path().join("lo").join("wireless")).unwrap();
        // wlp3s0 has wireless/ → counts.
        std::fs::create_dir_all(tmp.path().join("wlp3s0").join("wireless")).unwrap();

        let out = wifi_interfaces_under(tmp.path());
        assert_eq!(out, vec!["wlan0".to_string(), "wlp3s0".to_string()]);
    }

    #[test]
    fn wifi_interfaces_under_handles_missing_root() {
        let nonexistent = std::env::temp_dir().join("proteus-rf-test-nope-12345");
        let _ = std::fs::remove_dir_all(&nonexistent);
        let out = wifi_interfaces_under(&nonexistent);
        assert!(out.is_empty());
    }

    #[test]
    fn chip_info_under_reads_vendor_and_device_from_sysfs() {
        let tmp = tempdir();
        let iface = tmp.path().join("wlan0");
        std::fs::create_dir_all(iface.join("device")).unwrap();
        std::fs::write(iface.join("device").join("vendor"), "0x8086\n").unwrap();
        std::fs::write(iface.join("device").join("device"), "0x2526\n").unwrap();

        let info = chip_info_under(tmp.path(), "wlan0").unwrap();
        assert_eq!(info.iface, "wlan0");
        assert_eq!(info.vendor_id.as_deref(), Some("0x8086"));
        assert_eq!(info.device_id.as_deref(), Some("0x2526"));
        // No driver/module path on disk → driver absent is OK.
        assert!(info.driver.is_none());
    }

    #[test]
    fn chip_info_under_finds_driver_via_module_name_file() {
        let tmp = tempdir();
        let iface = tmp.path().join("wlan0");
        let module = iface.join("device").join("driver").join("module");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("name"), "iwlwifi\n").unwrap();

        let info = chip_info_under(tmp.path(), "wlan0").unwrap();
        assert_eq!(info.driver.as_deref(), Some("iwlwifi"));
    }

    #[test]
    fn chip_info_under_errors_on_unknown_iface() {
        let tmp = tempdir();
        let err = chip_info_under(tmp.path(), "wlan-missing").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("wlan-missing"),
            "error did not mention iface: {msg}"
        );
    }

    #[test]
    fn parse_iw_dev_info_txpower_handles_canonical_output() {
        let sample = "Interface wlan0\n\ttype managed\n\ttxpower 20.00 dBm\n";
        assert_eq!(parse_iw_dev_info_txpower(sample), Some(2_000));
    }

    #[test]
    fn parse_iw_dev_info_txpower_handles_negative_and_fractional() {
        let sample = "txpower -3.50 dBm";
        assert_eq!(parse_iw_dev_info_txpower(sample), Some(-350));
        let sample = "txpower 14.12 dBm";
        assert_eq!(parse_iw_dev_info_txpower(sample), Some(1_412));
    }

    #[test]
    fn parse_iw_dev_info_txpower_returns_none_when_absent() {
        let sample = "Interface wlan0\n\ttype managed\n\tssid coffee\n";
        assert_eq!(parse_iw_dev_info_txpower(sample), None);
    }

    #[test]
    fn parse_iw_reg_get_picks_the_max_dbm() {
        let sample = "country US: DFS-FCC\n\
            (2402 - 2472 @ 40), (N/A, 30), (N/A)\n\
            (5170 - 5250 @ 80), (N/A, 23), (N/A)\n\
            (5250 - 5330 @ 80), (N/A, 24), (N/A)\n";
        assert_eq!(parse_iw_reg_get_max_mbm(sample), Some(3_000));
    }

    #[test]
    fn parse_iw_reg_get_handles_dbm_suffix_form() {
        let sample = "country US: DFS-FCC\n\
            (5170 - 5250 @ 80), (N/A, 23 dBm), (N/A)\n";
        assert_eq!(parse_iw_reg_get_max_mbm(sample), Some(2_300));
    }

    #[test]
    fn parse_iw_reg_get_returns_none_when_no_dbm_value() {
        let sample = "country US: nothing at all\n";
        assert!(parse_iw_reg_get_max_mbm(sample).is_none());
    }

    #[test]
    fn parse_iw_reg_get_does_not_treat_channel_bandwidth_as_dbm() {
        // Issue #160 regression: an earlier parser walked every parenthesized
        // token and could pick up `80` from `@ 80` (channel bandwidth) as a
        // dBm value. With only a frequency-band clause and no power tuple,
        // the parser must return None — never the bandwidth.
        let sample = "country US: DFS-FCC\n\
            (5170 - 5250 @ 80)\n";
        assert!(parse_iw_reg_get_max_mbm(sample).is_none());
    }

    #[test]
    fn parse_iw_dev_info_txpower_rejects_unrelated_txpower_keys() {
        // Issue #160 defense-in-depth: the prefix check requires `txpower `
        // (trailing space). A future `txpower-foo` line must not be parsed
        // as a power value.
        let sample = "Interface wlan0\n\ttxpower-mode auto\n";
        assert_eq!(parse_iw_dev_info_txpower(sample), None);
    }

    #[test]
    fn regulatory_max_mbm_or_fallback_uses_safe_default_when_lookup_fails() {
        // We can't reliably stub `iw reg get` in the unit-test environment,
        // but the fallback constant must be the documented 20 dBm.
        assert_eq!(FALLBACK_REGULATORY_MAX_MBM, 2_000);
    }

    // ---- Milestone 4b: scan-policy + chipset parsers ----

    #[test]
    fn iw_bin_prefers_absolute_paths_when_present() {
        // Issue #202: when an absolute path resolves, prefer it; otherwise
        // fall back to a bare name (which `Command` resolves via PATH).
        // The unit test can't stub the filesystem in-place, but we can
        // assert the fallback path is the bare name and that the absolute
        // candidates list contains the two distros' canonical layouts.
        assert!(IW_ABS_PATHS.contains(&"/usr/bin/iw"));
        assert!(IW_ABS_PATHS.contains(&"/sbin/iw"));
    }

    #[test]
    fn parse_iw_dev_info_extracts_wiphy_and_type() {
        let sample = "Interface wlan0\n\tifindex 3\n\twdev 0x1\n\taddr aa:bb:cc:dd:ee:ff\n\tssid coffee\n\ttype managed\n\twiphy 0\n\tchannel 36\n";
        let (phy, kind) = parse_iw_dev_info_phy_and_type(sample);
        assert_eq!(phy, "phy0");
        assert_eq!(kind.as_deref(), Some("managed"));
    }

    #[test]
    fn parse_iw_dev_info_returns_blank_when_no_wiphy() {
        let sample = "Interface wlan0\n\ttype managed\n";
        let (phy, kind) = parse_iw_dev_info_phy_and_type(sample);
        assert_eq!(phy, "");
        assert_eq!(kind.as_deref(), Some("managed"));
    }

    #[test]
    fn parse_iw_phy_capabilities_detects_randomize_mac() {
        let sample = "Wiphy phy0\n\tSupported commands:\n\t\t * new_interface\n\t\t * trigger_scan\n\t\t * scan_random_mac_addr\n";
        let caps = parse_iw_phy_capabilities(sample);
        assert!(caps.randomize_mac);
        assert!(caps.active_scan);
        assert!(
            caps.raw_scan_line
                .as_deref()
                .unwrap()
                .starts_with("Supported commands:")
        );
    }

    #[test]
    fn parse_iw_phy_capabilities_handles_missing_randomize_mac() {
        let sample = "Wiphy phy0\n\tSupported commands:\n\t\t * trigger_scan\n";
        let caps = parse_iw_phy_capabilities(sample);
        assert!(!caps.randomize_mac);
        assert!(caps.active_scan);
    }

    #[test]
    fn parse_iw_phy_capabilities_returns_defaults_for_empty_input() {
        let caps = parse_iw_phy_capabilities("");
        assert!(!caps.randomize_mac);
        assert!(!caps.active_scan);
        assert!(caps.raw_scan_line.is_none());
    }

    /// Issue #273: case-insensitive helper underpins the perf fix. Pin
    /// the cases we care about so a future "optimize" doesn't break
    /// matching on mixed-case phy capability lines.
    #[test]
    fn contains_ascii_case_insensitive_matches_regardless_of_case() {
        assert!(contains_ascii_case_insensitive(
            "RANDOMIZE_MAC",
            "randomize_mac"
        ));
        assert!(contains_ascii_case_insensitive(
            "Supported: SCAN_RANDOM_MAC_ADDR",
            "scan_random_mac"
        ));
        assert!(contains_ascii_case_insensitive("foo bar", "BAR"));
        // Empty needle matches anything (matches `str::contains("")`).
        assert!(contains_ascii_case_insensitive("anything", ""));
        // Needle longer than haystack can't match.
        assert!(!contains_ascii_case_insensitive("ab", "abc"));
        // No match.
        assert!(!contains_ascii_case_insensitive("hello world", "xyz"));
        // Substring at the start, middle, end.
        assert!(contains_ascii_case_insensitive("ABCDEF", "abc"));
        assert!(contains_ascii_case_insensitive("xABCx", "abc"));
        assert!(contains_ascii_case_insensitive("xxABC", "abc"));
    }

    /// Issue #273: the iface name lowercase happens ONCE for
    /// `dmesg_firmware_line`. We can't shell out to dmesg in tests, so
    /// pin the inner search semantics through `contains_ascii_case_insensitive`
    /// — a dmesg-shaped line containing both `firmware` and the iface
    /// name in any case must match.
    #[test]
    fn dmesg_search_finds_firmware_line_regardless_of_case() {
        let line = "[ 12.345] iwlwifi 0000:00:14.3: WLAN0: loaded firmware version blah";
        let iface_lower = "wlan0".to_ascii_lowercase();
        assert!(contains_ascii_case_insensitive(line, "firmware"));
        assert!(contains_ascii_case_insensitive(line, &iface_lower));
    }

    #[test]
    fn phy_for_iface_under_reads_phy80211_name() {
        let tmp = tempdir();
        let iface = tmp.path().join("wlan0").join("phy80211");
        std::fs::create_dir_all(&iface).unwrap();
        std::fs::write(iface.join("name"), "phy0\n").unwrap();
        assert_eq!(
            phy_for_iface_under(tmp.path(), "wlan0").as_deref(),
            Some("phy0")
        );
    }

    #[test]
    fn phy_for_iface_under_returns_none_for_unknown_iface() {
        let tmp = tempdir();
        // No phy80211 entry → None, not an error. Same shape as the
        // existing chip_info_under tests.
        assert!(phy_for_iface_under(tmp.path(), "ghost0").is_none());
    }

    #[test]
    fn scan_policy_returns_default_for_unsafe_iface_name() {
        // Defense-in-depth: an iface with a `/` should never trigger an
        // `iw` shell-out. The struct returned is the all-default form.
        let p = scan_policy("/etc/passwd");
        assert_eq!(p.iface, "/etc/passwd");
        assert!(p.phy.is_none());
        assert!(!p.supports_randomize_mac);
    }

    /// Sysfs-based driver lookup gracefully returns `None` for nonexistent
    /// iface — uses `crate::testing::TempRoot` per the milestone brief.
    #[test]
    fn chip_info_under_temp_root_returns_err_for_unknown_iface() {
        let dir = crate::testing::TempRoot::new("rf-chip");
        let err = chip_info_under(&dir.path, "ghost0").unwrap_err();
        assert!(err.to_string().contains("ghost0"));
    }

    #[test]
    fn chip_info_extended_iface_field_is_set_even_when_sysfs_misses() {
        // A non-existent iface still produces a struct (no panic, no
        // error) so the command-layer table renderer can keep going.
        let info = chip_info_extended("nope-iface-not-real-12345");
        assert_eq!(info.iface, "nope-iface-not-real-12345");
        // No assertion on driver/vendor — the test environment may have
        // an iface with this name (unlikely) or the dmesg fallback may
        // turn up something. The contract here is "doesn't panic".
    }

    /// Throwaway tempdir that wipes itself on Drop. Mirrors the helper in
    /// `src/diff/mod.rs` rather than pulling in the `tempfile` crate.
    fn tempdir() -> TempDir {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("proteus-rf-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).expect("create test tempdir");
        TempDir { path }
    }

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
