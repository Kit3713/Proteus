// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus session` — current network session at a glance.
//!
//! Read-only, no root. Different from `proteus status` (system-wide overview)
//! and `proteus current` (raw identifier table): tells the user the
//! session-level story for the active network — interface, SSID, when joined,
//! captive-portal state, what Proteus rotated and how recently, and when the
//! next scheduled rotation fires.
//!
//! All data sources are best-effort. Missing pieces render as `unknown`/null
//! rather than failing the command, so this stays useful even when NM isn't
//! on the bus or no Proteus state has been written yet.
//!
//! Sources:
//! - active interface + chipset: `/sys/class/net/<if>/{wireless,phy80211,device/driver}`
//! - SSID + connection profile: NetworkManager DBus
//! - joined timestamp: NM connection settings `connection.timestamp`
//! - MAC: live from sysfs; rotated_at from `state.json` (managed.interfaces)
//! - hostname: cached `originals.hostname` + current via `hostnamed` DBus
//! - bluetooth: same DBus path used by `proteus bluetooth status`
//! - DUID + auto triggers: derived from `Config`
//! - next rotation: `systemctl list-timers --no-legend proteus-rotate.timer`

use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Serialize;

use crate::commands::status as status_cmd;
use crate::config::Config;
use crate::exit;
use crate::mac::Mac;
use crate::mac::oui::{APPLE, DELL, INTEL, OuiPrefix, SAMSUNG};
use crate::state::State;

const SCHEMA_VERSION: u32 = 1;
const SYSTEMD_MARKER: &str = "/run/systemd/system";

#[derive(Debug, Serialize)]
struct SessionReport {
    schema_version: u32,
    network: Option<NetworkBlock>,
    captive_portal: Option<CaptivePortalBlock>,
    mac: Option<MacBlock>,
    hostname: Option<HostnameBlock>,
    bluetooth: Option<BluetoothBlock>,
    duid: Option<String>,
    auto_triggers: Vec<String>,
    next_rotation_at: Option<String>,
    next_rotation_in: Option<String>,
}

#[derive(Debug, Serialize)]
struct NetworkBlock {
    iface: String,
    ssid: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    chipset: Option<String>,
    profile: Option<String>,
    joined_at: Option<String>,
    joined_seconds_ago: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CaptivePortalBlock {
    classification: String,
    checked_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct MacBlock {
    current: Option<String>,
    rotated_at: Option<String>,
    rotated_seconds_ago: Option<u64>,
    rotation_count: u64,
    oui_vendor: Option<String>,
    pinned: Option<String>,
}

#[derive(Debug, Serialize)]
struct HostnameBlock {
    current: Option<String>,
    mode: String,
    rotated_at: Option<String>,
    rotated_seconds_ago: Option<u64>,
}

#[derive(Debug, Serialize)]
struct BluetoothBlock {
    hci: String,
    alias: Option<String>,
    rpa_active: bool,
}

pub fn run(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let report = build_report(state_path, config_path);
    if json {
        super::print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(exit::SUCCESS)
}

fn build_report(state_path: Option<&Path>, config_path: Option<&Path>) -> SessionReport {
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let state = State::load_or_default(&state_path).unwrap_or_default();
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let now = now_unix();

    let ifaces = status_cmd::enumerate_interfaces();
    let active = pick_active_interface(&ifaces);
    let nm_data = active.and_then(|i| gather_nm_session(&i.name).ok().flatten());

    let network = active.map(|iface| NetworkBlock {
        iface: iface.name.clone(),
        ssid: nm_data.as_ref().and_then(|n| n.ssid.clone()),
        kind: iface.kind.clone(),
        chipset: read_chipset(&iface.name),
        profile: nm_data.as_ref().and_then(|n| n.profile.clone()),
        joined_at: nm_data.as_ref().and_then(|n| n.joined_at_iso.clone()),
        joined_seconds_ago: nm_data.as_ref().and_then(|n| n.joined_seconds_ago(now)),
    });

    let mac_block = active.map(|iface| build_mac_block(&state, iface, now));
    let hostname_block = build_hostname_block(&state, &config, now);
    let bluetooth_block = gather_bluetooth_block();

    let duid = config.ipv6.enabled.then(|| "link-layer".to_string());

    let (next_rotation_at, next_rotation_in) = next_rotation_pair();

    SessionReport {
        schema_version: SCHEMA_VERSION,
        network,
        // Captive-portal classifier lands later; the field stays so the
        // schema is forward-compatible.
        captive_portal: None,
        mac: mac_block,
        hostname: hostname_block,
        bluetooth: bluetooth_block,
        duid,
        auto_triggers: enabled_auto_triggers(),
        next_rotation_at,
        next_rotation_in,
    }
}

/// Pick the "active" interface to report on. Prefer the first wireless
/// interface that has a MAC; otherwise the first wired one.
fn pick_active_interface(ifaces: &[status_cmd::Iface]) -> Option<&status_cmd::Iface> {
    ifaces
        .iter()
        .find(|i| i.kind == "wifi" && i.mac.is_some())
        .or_else(|| {
            ifaces
                .iter()
                .find(|i| i.kind == "ethernet" && i.mac.is_some())
        })
        .or_else(|| ifaces.first())
}

/// Read the kernel driver short name (e.g. `iwlwifi`) and combine with a
/// vendor tag pulled from `device/vendor` if available. Returns `None` when
/// neither is readable.
fn read_chipset(iface: &str) -> Option<String> {
    let driver_link = Path::new("/sys/class/net")
        .join(iface)
        .join("device")
        .join("driver");
    let driver = std::fs::read_link(&driver_link)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let vendor = vendor_label(iface);
    match (vendor, driver) {
        (Some(v), Some(d)) => Some(format!("{v} {d}")),
        (v, d) => v.or(d),
    }
}

/// Best-effort vendor label using the PCI vendor id or ueventized marker.
fn vendor_label(iface: &str) -> Option<String> {
    let vendor_path = Path::new("/sys/class/net")
        .join(iface)
        .join("device")
        .join("vendor");
    let raw = std::fs::read_to_string(&vendor_path).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "0x8086" => Some("intel".into()),
        "0x10de" => Some("nvidia".into()),
        "0x14e4" => Some("broadcom".into()),
        "0x168c" | "0x17cb" | "0x17e8" => Some("qualcomm".into()),
        "0x1969" => Some("atheros".into()),
        "0x10ec" => Some("realtek".into()),
        "0x1814" => Some("ralink".into()),
        "0x14c3" => Some("mediatek".into()),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct NmSession {
    ssid: Option<String>,
    profile: Option<String>,
    joined_at_iso: Option<String>,
    joined_at_unix: Option<u64>,
}

impl NmSession {
    fn joined_seconds_ago(&self, now: u64) -> Option<u64> {
        self.joined_at_unix
            .and_then(|t| if now >= t { Some(now - t) } else { None })
    }
}

/// Fetch SSID + profile + connect timestamp for `iface` from NetworkManager.
/// Returns `Ok(None)` when the bus or the NM service isn't available, so the
/// session command stays usable on systems without NM.
fn gather_nm_session(iface: &str) -> Result<Option<NmSession>> {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return Ok(None),
    };
    runtime.block_on(async move {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let devices = match crate::nm::list_devices(&conn).await {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };
        let Some(dev) = devices.iter().find(|d| d.interface == iface) else {
            return Ok(None);
        };
        let path = match dev.connections.first() {
            Some(p) => p.clone(),
            None => return Ok(None),
        };
        let proxy = match crate::nm::ConnectionProxy::builder(&conn)
            .path(path.clone())?
            .build()
            .await
        {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        let settings = match proxy.get_settings().await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut session = NmSession {
            profile: extract_str(&settings, "connection", "id"),
            ..Default::default()
        };
        if let Some(ssid_bytes) = extract_byte_array(&settings, "802-11-wireless", "ssid") {
            session.ssid = Some(String::from_utf8_lossy(&ssid_bytes).into_owned());
        }
        if let Some(ts) = extract_u64(&settings, "connection", "timestamp") {
            session.joined_at_unix = Some(ts);
            session.joined_at_iso = Some(unix_to_iso8601(ts));
        }
        Ok(Some(session))
    })
}

fn extract_str(
    settings: &crate::nm::ConnectionSettings,
    section: &str,
    key: &str,
) -> Option<String> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &zbus::zvariant::Value = val;
    if let zbus::zvariant::Value::Str(s) = v {
        Some(s.as_str().to_string())
    } else {
        None
    }
}

fn extract_u64(settings: &crate::nm::ConnectionSettings, section: &str, key: &str) -> Option<u64> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &zbus::zvariant::Value = val;
    match v {
        zbus::zvariant::Value::U64(n) => Some(*n),
        zbus::zvariant::Value::U32(n) => Some(u64::from(*n)),
        zbus::zvariant::Value::I64(n) if *n >= 0 => Some(*n as u64),
        zbus::zvariant::Value::I32(n) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

fn extract_byte_array(
    settings: &crate::nm::ConnectionSettings,
    section: &str,
    key: &str,
) -> Option<Vec<u8>> {
    let sec = settings.get(section)?;
    let val = sec.get(key)?;
    let v: &zbus::zvariant::Value = val;
    if let zbus::zvariant::Value::Array(arr) = v {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr.iter() {
            if let zbus::zvariant::Value::U8(b) = item {
                out.push(*b);
            } else {
                return None;
            }
        }
        return Some(out);
    }
    None
}

fn build_mac_block(state: &State, iface: &status_cmd::Iface, now: u64) -> MacBlock {
    let rec = state.managed.interfaces.get(&iface.name);
    let rotated_at = rec.and_then(|r| r.last_rotated.clone());
    let oui_vendor = iface
        .mac
        .as_deref()
        .and_then(|s| Mac::from_str(s).ok())
        .map(|m| classify_oui(&m.octets()));
    MacBlock {
        current: iface.mac.clone(),
        rotated_seconds_ago: seconds_since(rotated_at.as_deref(), now),
        rotated_at,
        rotation_count: rec.map(|r| r.rotation_count).unwrap_or(0),
        oui_vendor,
        pinned: rec.and_then(|r| r.pinned.clone()),
    }
}

fn build_hostname_block(state: &State, config: &Config, now: u64) -> Option<HostnameBlock> {
    if !config.hostname.enabled {
        return None;
    }
    let rotated_at = state
        .captured_at
        .clone()
        .filter(|_| state.originals.hostname.is_some());
    Some(HostnameBlock {
        current: read_current_hostname(),
        mode: config.hostname.mode.clone(),
        rotated_seconds_ago: seconds_since(rotated_at.as_deref(), now),
        rotated_at,
    })
}

/// `now - parse_iso8601(at)`, clamped to `None` when `at` is missing,
/// unparseable, or in the future.
fn seconds_since(at: Option<&str>, now: u64) -> Option<u64> {
    let t = parse_iso8601(at?)?;
    (now >= t).then(|| now - t)
}

fn read_current_hostname() -> Option<String> {
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn gather_bluetooth_block() -> Option<BluetoothBlock> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(async move {
        let (_, adapters) = crate::bluetooth::connect_and_list().await.ok().flatten()?;
        let first = adapters.into_iter().next()?;
        Some(BluetoothBlock {
            hci: first.hci,
            alias: first.alias,
            rpa_active: first.privacy_active,
        })
    })
}

fn enabled_auto_triggers() -> Vec<String> {
    if !Path::new(SYSTEMD_MARKER).is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if unit_active("proteus-rotate.timer") {
        out.push("scheduled".to_string());
    }
    if unit_active("proteus-check.timer") {
        out.push("probe-fail".to_string());
    }
    if unit_active("proteus-resume.timer") {
        out.push("resume".to_string());
    }
    if unit_enabled("proteus-boot.service") {
        out.push("boot".to_string());
    }
    out
}

fn unit_active(unit: &str) -> bool {
    matches!(
        systemctl_state("is-active", unit).as_deref(),
        Some("active" | "activating" | "reloading")
    )
}

fn unit_enabled(unit: &str) -> bool {
    matches!(
        systemctl_state("is-enabled", unit).as_deref(),
        Some("enabled" | "enabled-runtime" | "static" | "alias" | "indirect")
    )
}

fn systemctl_state(verb: &str, unit: &str) -> Option<String> {
    // Pin the C locale so the output we parse stays in the canonical
    // English form regardless of the operator's session.
    let out = Command::new(crate::process::systemctl())
        .env("LC_ALL", "C")
        .args([verb, unit])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn next_rotation_pair() -> (Option<String>, Option<String>) {
    if !Path::new(SYSTEMD_MARKER).is_dir() {
        return (None, None);
    }
    // `list-timers` formats dates and the "left" duration via the process
    // locale; `parse_next_rotation` below assumes the C-locale English
    // layout ("Wed 2026-05-07 16:00:00 UTC 1h 46min …"). Force the locale
    // so a French/German/Japanese installation produces parseable output.
    let out = match Command::new(crate::process::systemctl())
        .env("LC_ALL", "C")
        .args([
            "list-timers",
            "--all",
            "--no-pager",
            "--no-legend",
            "proteus-rotate.timer",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return (None, None),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_next_rotation(&text)
}

/// Parse the `NEXT  LEFT  ...` row from `systemctl list-timers --no-legend`.
/// Returns `(absolute, "1h 46min")`-shaped pair. Caller is expected to have
/// invoked systemctl with `LC_ALL=C` so the column layout is the canonical
/// English form documented here.
///
/// Layout (--no-legend, C locale):
///   `Wed 2026-05-07 16:00:00 UTC  1h 46min  Wed 2026-05-07 14:00:00 UTC  14min ago  proteus-rotate.timer  …`
///
/// The NEXT column is four whitespace-separated fields (weekday, date,
/// time, timezone). LEFT is everything between NEXT and the next
/// weekday-or-date field that starts the LAST column.
pub(crate) fn parse_next_rotation(text: &str) -> (Option<String>, Option<String>) {
    const NEXT_FIELDS: usize = 4;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Need NEXT (4 fields) + at least one LEFT field.
        if fields.len() < NEXT_FIELDS + 1 {
            return (None, None);
        }
        let absolute = fields[0..NEXT_FIELDS].join(" ");
        // LEFT runs until the next column marker (weekday abbrev, a
        // `YYYY-MM-DD` field, or systemd's `n/a` placeholder for an
        // unfired timer) — that's the start of the LAST column. Skip one
        // field past NEXT so we don't immediately re-trigger on the
        // weekday inside NEXT itself.
        let mut split_at = fields.len();
        for (i, f) in fields.iter().enumerate().skip(NEXT_FIELDS + 1) {
            if is_weekday_abbrev(f) || looks_like_iso_date(f) || *f == "n/a" {
                split_at = i;
                break;
            }
        }
        let left = fields[NEXT_FIELDS..split_at].join(" ");
        let absolute = if absolute.contains('-') {
            Some(absolute)
        } else {
            None
        };
        let left = if left.is_empty() { None } else { Some(left) };
        return (absolute, left);
    }
    (None, None)
}

fn looks_like_iso_date(s: &str) -> bool {
    // YYYY-MM-DD: dashes at positions 4 and 7, ASCII digits elsewhere.
    // Distinguishes a date field from a duration token like "1h"/"46min".
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn is_weekday_abbrev(s: &str) -> bool {
    matches!(s, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")
}

fn classify_oui(octets: &[u8; 6]) -> String {
    let prefix: OuiPrefix = [octets[0], octets[1], octets[2]];
    if octets[0] & 0x02 != 0 && octets[0] & 0x01 == 0 {
        return "locally-administered".into();
    }
    if APPLE.contains(&prefix) {
        return "Apple".into();
    }
    if INTEL.contains(&prefix) {
        return "Intel".into();
    }
    if SAMSUNG.contains(&prefix) {
        return "Samsung".into();
    }
    if DELL.contains(&prefix) {
        return "Dell".into();
    }
    "unknown".into()
}

/// Parse the same ISO-8601 shape `commands::now_iso8601` emits. Returns the
/// unix timestamp (seconds since epoch). Returns `None` for anything that
/// doesn't look like our own format.
pub(crate) fn parse_iso8601(s: &str) -> Option<u64> {
    // Format: YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim_end_matches('Z');
    let bytes = s.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let mo: u32 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let d: u32 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let h: u32 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let mi: u32 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let se: u32 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
    Some(ymdhms_to_unix(year, mo, d, h, mi, se))
}

fn ymdhms_to_unix(year: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    // Reverse of `commands::unix_to_ymdhms` — same Howard Hinnant
    // days_from_civil routine.
    let y = if mo <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp: u64 = if mo > 2 {
        u64::from(mo) - 3
    } else {
        u64::from(mo) + 9
    };
    let doy = (153 * mp + 2) / 5 + u64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let total = days as i128 * 86_400 + i128::from(h) * 3600 + i128::from(mi) * 60 + i128::from(s);
    if total < 0 { 0 } else { total as u64 }
}

fn unix_to_iso8601(t: u64) -> String {
    let (y, mo, d, h, mi, s) = super::unix_to_ymdhms(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render "X minutes ago" style relative time. Public so the test module can
/// drive it deterministically.
pub(crate) fn humanize_seconds_ago(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs} second{} ago", plural(secs));
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins} minute{} ago", plural(mins));
    }
    let hours = mins / 60;
    if hours < 24 {
        let rem = mins % 60;
        if rem == 0 {
            return format!("{hours} hour{} ago", plural(hours));
        }
        return format!("{hours}h {rem}m ago");
    }
    let days = hours / 24;
    let rem_h = hours % 24;
    if rem_h == 0 {
        return format!("{days} day{} ago", plural(days));
    }
    format!("{days}d {rem_h}h ago")
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn print_human(r: &SessionReport) {
    println!("proteus session");
    println!();

    match &r.network {
        Some(net) => {
            let label = format_network_label(net);
            print_row("Network", &label);
            if let Some(profile) = &net.profile {
                if Some(profile) != net.ssid.as_ref() {
                    print_row("Profile", profile);
                }
            }
            if let Some(secs) = net.joined_seconds_ago {
                print_row("Joined", &humanize_seconds_ago(secs));
            } else if let Some(at) = &net.joined_at {
                print_row("Joined", at);
            } else {
                print_row("Joined", "unknown");
            }
        }
        None => print_row("Network", "no active interface detected"),
    }

    match &r.captive_portal {
        Some(cp) => print_row("Captive portal", &cp.classification),
        None => print_row("Captive portal", "unknown (run `proteus probe`)"),
    }

    if let Some(mac) = &r.mac {
        print_row("MAC", &format_mac_line(mac));
    }

    if let Some(host) = &r.hostname {
        print_row("Hostname", &format_hostname_line(host));
    }

    if let Some(bt) = &r.bluetooth {
        let rpa = if bt.rpa_active {
            "RPA active"
        } else {
            "RPA inactive"
        };
        let alias = bt.alias.as_deref().unwrap_or("(unset)");
        print_row(
            "Bluetooth",
            &format!("{} alias \"{}\" ({})", bt.hci, alias, rpa),
        );
    }

    if let Some(duid) = &r.duid {
        print_row("DUID", &format!("{duid} (rotates with MAC)"));
    }

    let triggers = if r.auto_triggers.is_empty() {
        "none enabled".to_string()
    } else {
        format!("{} enabled", r.auto_triggers.join(", "))
    };
    print_row("Auto triggers", &triggers);

    println!();
    match (&r.next_rotation_in, &r.next_rotation_at) {
        (Some(left), _) => println!("Next scheduled rotation in {left}."),
        (None, Some(at)) => println!("Next scheduled rotation at {at}."),
        (None, None) => println!("Next scheduled rotation: not scheduled."),
    }
}

const ROW_LABEL_WIDTH: usize = 14;

fn print_row(label: &str, value: &str) {
    println!("{label:<ROW_LABEL_WIDTH$} {value}");
}

fn format_network_label(net: &NetworkBlock) -> String {
    let kind_label = match net.kind.as_str() {
        "wifi" => "Wi-Fi",
        "ethernet" => "Ethernet",
        other => other,
    };
    let chip = net
        .chipset
        .as_deref()
        .map(|c| format!(", {c}"))
        .unwrap_or_default();
    match &net.ssid {
        Some(ssid) => format!("{} \u{2192} {ssid} ({kind_label}{chip})", net.iface),
        None => format!("{} ({kind_label}{chip})", net.iface),
    }
}

fn format_mac_line(mac: &MacBlock) -> String {
    let current = mac.current.as_deref().unwrap_or("unknown");
    let mut suffix = String::new();
    if let Some(secs) = mac.rotated_seconds_ago {
        suffix.push_str(&format!(" (rotated {}", humanize_seconds_ago(secs)));
        if let Some(v) = &mac.oui_vendor {
            suffix.push_str(&format!(", OUI: {v}"));
        }
        suffix.push(')');
    } else if let Some(v) = &mac.oui_vendor {
        suffix.push_str(&format!(" (OUI: {v})"));
    }
    if let Some(p) = &mac.pinned {
        suffix.push_str(&format!(" [pinned={p}]"));
    }
    format!("{current}{suffix}")
}

fn format_hostname_line(host: &HostnameBlock) -> String {
    let name = host.current.as_deref().unwrap_or("unknown");
    let mut detail = format!("(mode: {}", host.mode);
    if let Some(secs) = host.rotated_seconds_ago {
        detail.push_str(&format!(", rotated {}", humanize_seconds_ago(secs)));
    }
    detail.push(')');
    format!("{name} {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{InterfaceRecord, ManagedState};

    #[test]
    fn humanize_seconds_ago_renders_singular_and_plural() {
        assert_eq!(humanize_seconds_ago(0), "0 seconds ago");
        assert_eq!(humanize_seconds_ago(1), "1 second ago");
        assert_eq!(humanize_seconds_ago(45), "45 seconds ago");
        assert_eq!(humanize_seconds_ago(60), "1 minute ago");
        assert_eq!(humanize_seconds_ago(12 * 60), "12 minutes ago");
        assert_eq!(humanize_seconds_ago(60 * 60), "1 hour ago");
        assert_eq!(humanize_seconds_ago(60 * 60 + 30 * 60), "1h 30m ago");
        assert_eq!(humanize_seconds_ago(2 * 60 * 60), "2 hours ago");
        assert_eq!(humanize_seconds_ago(25 * 60 * 60), "1d 1h ago");
        assert_eq!(humanize_seconds_ago(2 * 24 * 60 * 60), "2 days ago");
    }

    #[test]
    fn parse_and_format_iso8601_round_trip() {
        let ts = parse_iso8601("2026-05-07T14:32:10Z").unwrap();
        let back = unix_to_iso8601(ts);
        assert_eq!(back, "2026-05-07T14:32:10Z");
    }

    #[test]
    fn classify_oui_handles_known_and_laa() {
        // Apple prefix from src/mac/oui.rs APPLE list.
        let m: [u8; 6] = [0x00, 0x03, 0x93, 0x12, 0x34, 0x56];
        assert_eq!(classify_oui(&m), "Apple");
        // Locally-administered (bit 0x02 set, multicast clear).
        let m: [u8; 6] = [0x02, 0xAB, 0xCD, 0xEF, 0x01, 0x02];
        assert_eq!(classify_oui(&m), "locally-administered");
        // Random universal-administered prefix (0x70 has neither LAA nor
        // multicast bit) not in our table.
        let m: [u8; 6] = [0x70, 0x70, 0x70, 0x00, 0x00, 0x01];
        assert_eq!(classify_oui(&m), "unknown");
    }

    #[test]
    fn parse_next_rotation_pulls_left_column() {
        // Real `systemctl list-timers --no-legend --all proteus-rotate.timer`
        // output, C locale: NEXT (4 fields, includes timezone) then LEFT.
        let line = "Wed 2026-05-07 16:00:00 CDT 1h 46min Wed 2026-05-07 14:00:00 CDT 14min ago proteus-rotate.timer proteus-rotate.service";
        let (next, left) = parse_next_rotation(line);
        // NEXT must include the timezone — splitting it off was the bug
        // behind issue #140.
        assert_eq!(next.as_deref(), Some("Wed 2026-05-07 16:00:00 CDT"));
        // LEFT is just the duration; the timezone abbrev belongs to NEXT.
        assert_eq!(left.as_deref(), Some("1h 46min"));
    }

    #[test]
    fn parse_next_rotation_handles_utc_format() {
        let line = "Wed 2026-05-07 16:00:00 UTC 1h 46min n/a n/a proteus-rotate.timer proteus-rotate.service";
        let (next, left) = parse_next_rotation(line);
        assert_eq!(next.as_deref(), Some("Wed 2026-05-07 16:00:00 UTC"));
        assert_eq!(left.as_deref(), Some("1h 46min"));
    }

    #[test]
    fn parse_next_rotation_returns_none_on_short_lines() {
        // Truncated output (NEXT only, no LEFT) shouldn't return half-baked
        // data.
        assert_eq!(
            parse_next_rotation("Wed 2026-05-07 16:00:00 CDT"),
            (None, None)
        );
        assert_eq!(parse_next_rotation(""), (None, None));
    }

    #[test]
    fn report_serializes_with_schema_version_one() {
        let report = SessionReport {
            schema_version: SCHEMA_VERSION,
            network: Some(NetworkBlock {
                iface: "wlan0".into(),
                ssid: Some("CoffeeShopWiFi".into()),
                kind: "wifi".into(),
                chipset: Some("intel iwlwifi".into()),
                profile: Some("CoffeeShopWiFi".into()),
                joined_at: Some("2026-05-07T14:32:10Z".into()),
                joined_seconds_ago: Some(720),
            }),
            captive_portal: None,
            mac: Some(MacBlock {
                current: Some("aa:bb:cc:dd:ee:ff".into()),
                rotated_at: Some("2026-05-07T14:30:00Z".into()),
                rotated_seconds_ago: Some(840),
                rotation_count: 2,
                oui_vendor: Some("Apple".into()),
                pinned: None,
            }),
            hostname: Some(HostnameBlock {
                current: Some("linksys-wrt-1900".into()),
                mode: "wordlist".into(),
                rotated_at: Some("2026-05-07T14:30:00Z".into()),
                rotated_seconds_ago: Some(840),
            }),
            bluetooth: None,
            duid: Some("link-layer".into()),
            auto_triggers: vec!["scheduled".into(), "probe-fail".into()],
            next_rotation_at: Some("Wed 2026-05-07 16:00:00".into()),
            next_rotation_in: Some("1h 46min".into()),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schema_version"].as_u64(), Some(1));
        assert_eq!(json["network"]["ssid"].as_str(), Some("CoffeeShopWiFi"));
        assert_eq!(json["mac"]["oui_vendor"].as_str(), Some("Apple"));
        assert!(
            json["auto_triggers"]
                .as_array()
                .map(|a| a.iter().any(|v| v == "scheduled"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn build_mac_block_pulls_state_metadata() {
        let mut state = State {
            managed: ManagedState::default(),
            ..Default::default()
        };
        state.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                pinned: None,
                last_rotated: Some("2026-05-07T14:30:00Z".into()),
                rotation_count: 5,
            },
        );
        let iface = status_cmd::Iface {
            name: "wlan0".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            kind: "wifi".into(),
            wireless: true,
            chipset: None,
        };
        let now = parse_iso8601("2026-05-07T14:44:00Z").unwrap();
        let block = build_mac_block(&state, &iface, now);
        assert_eq!(block.rotation_count, 5);
        assert_eq!(block.rotated_seconds_ago, Some(14 * 60));
    }
}
