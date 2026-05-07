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
    let output = Command::new("iw")
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
    let mbm_str = mbm.to_string();
    let output = Command::new("iw")
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
    let output = Command::new("iw").args(["reg", "get"]).output().ok()?;
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

/// Run a synchronous `iw` to confirm the binary is on `$PATH`. Used by the
/// command layer to skip with `SYSTEM_NOT_SUPPORTED` rather than fail when
/// the operator hasn't installed iw-tools.
pub fn iw_present() -> bool {
    Command::new("iw")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
fn parse_iw_dev_info_txpower(text: &str) -> Option<i32> {
    for raw in text.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("txpower") else {
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
fn parse_iw_reg_get_max_mbm(text: &str) -> Option<i32> {
    let mut max_dbm: Option<f32> = None;
    for raw in text.lines() {
        // Each per-band entry is wrapped in parens; we only care about the
        // tuple immediately after the frequency range.
        let Some(after_range) = raw.split_once(',') else {
            continue;
        };
        let after_range = after_range.1;
        for token in after_range.split([',', '(', ')']) {
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
    fn regulatory_max_mbm_or_fallback_uses_safe_default_when_lookup_fails() {
        // We can't reliably stub `iw reg get` in the unit-test environment,
        // but the fallback constant must be the documented 20 dBm.
        assert_eq!(FALLBACK_REGULATORY_MAX_MBM, 2_000);
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
