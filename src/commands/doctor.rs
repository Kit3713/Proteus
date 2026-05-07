// SPDX-License-Identifier: GPL-3.0-or-later
//
// `proteus doctor` — read-only self-diagnostic.
//
// A battery of small checks aimed at "something broke, where do I start".
// Each check returns a `Status` plus a short human message and an optional
// remediation pointer (a wiki page or command). No mutations. Works without
// root — checks that need root degrade to `Skip` rather than `Fail`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::commands::status as status_cmd;
use crate::exit;
use crate::state::State;
use crate::version;

const SCHEMA_VERSION: u32 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub category: &'static str,
    pub name: &'static str,
    pub status: Status,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Summary {
    pub ok: u32,
    pub warn: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub proteus_version: &'static str,
    pub phase: char,
    pub checks: Vec<Check>,
    pub summary: Summary,
}

pub struct Options<'a> {
    pub json: bool,
    pub quick: bool,
    pub verbose: bool,
    pub no_color: bool,
    pub state_path: Option<&'a Path>,
    pub config_path: Option<&'a Path>,
}

pub fn run(opts: Options<'_>) -> Result<u8> {
    let report = build_report(&opts);
    if opts.json {
        super::print_json(&report)?;
    } else {
        let style = render_style(opts.no_color);
        print!("{}", render_human(&report, style, opts.verbose));
    }
    Ok(if report.summary.fail > 0 {
        exit::GENERIC_ERROR
    } else {
        exit::SUCCESS
    })
}

fn build_report(opts: &Options<'_>) -> Report {
    // Read daemon-presence paths once and feed both the system and daemons sections.
    let sys = status_cmd::detect_system();

    let mut checks = Vec::new();
    push_system(&mut checks, &sys);
    push_daemons(&mut checks, &sys, opts.quick);
    push_files(&mut checks, opts.config_path, opts.state_path);
    push_detect_and_defer(&mut checks, &sys, opts.quick);
    push_runtime(&mut checks);
    push_proteus_state(&mut checks, opts.state_path);
    let summary = aggregate(&checks);
    Report {
        schema_version: SCHEMA_VERSION,
        proteus_version: version::VERSION,
        phase: version::PHASE,
        checks,
        summary,
    }
}

// --- System checks --------------------------------------------------------

fn push_system(out: &mut Vec<Check>, sys: &status_cmd::SystemInfo) {
    out.push(check_linux_kernel());
    out.push(check_systemd(sys));
    out.push(check_root());
    out.push(check_libc());
    out.push(check_distro());
}

fn check_linux_kernel() -> Check {
    let ostype = read_first_line("/proc/sys/kernel/ostype").unwrap_or_default();
    if ostype.eq_ignore_ascii_case("Linux") {
        let release =
            read_first_line("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".into());
        Check {
            category: "system",
            name: "linux_kernel",
            status: Status::Ok,
            message: format!("Linux {release}"),
            remediation: None,
        }
    } else {
        Check {
            category: "system",
            name: "linux_kernel",
            status: Status::Fail,
            message: "not a Linux kernel — Proteus targets Linux only".into(),
            remediation: Some("proteus wiki concepts".into()),
        }
    }
}

fn check_systemd(sys: &status_cmd::SystemInfo) -> Check {
    if sys.systemd {
        let detail = read_first_line("/proc/1/comm")
            .filter(|c| c == "systemd")
            .map(|_| "running")
            .unwrap_or("present");
        Check {
            category: "system",
            name: "systemd",
            status: Status::Ok,
            message: format!("systemd {detail}"),
            remediation: None,
        }
    } else {
        Check {
            category: "system",
            name: "systemd",
            status: Status::Fail,
            message: "no /run/systemd/system — systemd is required".into(),
            remediation: Some("proteus wiki concepts".into()),
        }
    }
}

fn check_root() -> Check {
    let uid = super::read_uid();
    let (status, message) = match uid {
        Some(0) => (Status::Ok, "running as root".into()),
        Some(other) => (
            Status::Skip,
            format!("running as uid {other} — some checks need root for full detail"),
        ),
        None => (Status::Skip, "could not determine effective uid".into()),
    };
    Check {
        category: "system",
        name: "root",
        status,
        message,
        remediation: None,
    }
}

fn check_libc() -> Check {
    // Best-effort: probe the well-known dynamic-linker filenames. Distinguishes
    // glibc and musl on the most common targets without shelling out to ldd.
    let glibc = Path::new("/lib64/ld-linux-x86-64.so.2").exists()
        || Path::new("/lib/ld-linux-x86-64.so.2").exists()
        || Path::new("/lib/ld-linux-aarch64.so.1").exists()
        || Path::new("/lib64/ld-linux-aarch64.so.1").exists();
    let musl = std::fs::read_dir("/lib")
        .map(|d| {
            d.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
        })
        .unwrap_or(false);
    let (status, message) = match (glibc, musl) {
        (true, _) => (Status::Ok, "glibc-based".into()),
        (false, true) => (Status::Ok, "musl-based".into()),
        (false, false) => (Status::Skip, "could not determine libc flavor".into()),
    };
    Check {
        category: "system",
        name: "libc",
        status,
        message,
        remediation: None,
    }
}

fn check_distro() -> Check {
    let (id, version) = read_os_release();
    let id_lower = id.to_ascii_lowercase();
    let version_num: u32 = version
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if id.is_empty() {
        return Check {
            category: "system",
            name: "distro",
            status: Status::Skip,
            message: "/etc/os-release missing or unreadable".into(),
            remediation: None,
        };
    }
    let pretty = format!("{id} {version}").trim().to_string();
    if id_lower == "fedora" && version_num >= 43 {
        Check {
            category: "system",
            name: "distro",
            status: Status::Ok,
            message: pretty,
            remediation: None,
        }
    } else if id_lower == "fedora" {
        Check {
            category: "system",
            name: "distro",
            status: Status::Warn,
            message: format!(
                "{pretty} — primary target is Fedora 43+; some features may behave differently"
            ),
            remediation: None,
        }
    } else {
        Check {
            category: "system",
            name: "distro",
            status: Status::Warn,
            message: format!("{pretty} — Fedora 43+ is the primary target"),
            remediation: None,
        }
    }
}

// --- Daemons --------------------------------------------------------------

fn push_daemons(out: &mut Vec<Check>, sys: &status_cmd::SystemInfo, quick: bool) {
    out.push(check_network_manager(sys));
    out.push(check_bluez(sys));
    out.push(check_systemd_resolved(sys));
    out.push(check_nftables(quick));
}

fn check_network_manager(sys: &status_cmd::SystemInfo) -> Check {
    if sys.network_manager {
        Check {
            category: "daemons",
            name: "network_manager",
            status: Status::Ok,
            message: "NetworkManager running".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "daemons",
            name: "network_manager",
            status: Status::Fail,
            message: "NetworkManager not running — required for MAC rotation".into(),
            remediation: Some("systemctl start NetworkManager".into()),
        }
    }
}

fn check_bluez(sys: &status_cmd::SystemInfo) -> Check {
    if sys.bluez {
        Check {
            category: "daemons",
            name: "bluez",
            status: Status::Ok,
            message: "BlueZ running".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "daemons",
            name: "bluez",
            status: Status::Skip,
            message: "BlueZ not running — Bluetooth features will skip".into(),
            remediation: None,
        }
    }
}

fn check_systemd_resolved(sys: &status_cmd::SystemInfo) -> Check {
    if sys.systemd_resolved {
        Check {
            category: "daemons",
            name: "systemd_resolved",
            status: Status::Ok,
            message: "systemd-resolved running".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "daemons",
            name: "systemd_resolved",
            status: Status::Skip,
            message: "systemd-resolved not running — DNS knob will skip".into(),
            remediation: Some("proteus wiki dns".into()),
        }
    }
}

fn check_nftables(quick: bool) -> Check {
    if !binary_exists("nft") {
        return Check {
            category: "daemons",
            name: "nftables",
            status: Status::Warn,
            message: "nft binary not found — discovery blocks will skip".into(),
            remediation: Some("install the nftables package".into()),
        };
    }
    if quick || super::read_uid() != Some(0) {
        return Check {
            category: "daemons",
            name: "nftables",
            status: Status::Skip,
            message: "nft binary present, ruleset hidden (need root to inspect)".into(),
            remediation: None,
        };
    }
    Check {
        category: "daemons",
        name: "nftables",
        status: Status::Ok,
        message: "nft binary present, ruleset readable".into(),
        remediation: None,
    }
}

// --- Files ----------------------------------------------------------------

fn push_files(out: &mut Vec<Check>, config_path: Option<&Path>, state_path: Option<&Path>) {
    out.push(check_config_dir());
    out.push(check_config_file(config_path));
    out.push(check_state_file(state_path));
}

fn check_config_dir() -> Check {
    let p = Path::new("/etc/proteus");
    if p.is_dir() {
        Check {
            category: "files",
            name: "config_dir",
            status: Status::Ok,
            message: "/etc/proteus exists".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "files",
            name: "config_dir",
            status: Status::Skip,
            message: "/etc/proteus missing — first run will create it".into(),
            remediation: None,
        }
    }
}

fn check_config_file(override_path: Option<&Path>) -> Check {
    let path = super::config_path(override_path);
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<crate::config::RawConfig>(&s) {
            Ok(_) => Check {
                category: "files",
                name: "config_file",
                status: Status::Ok,
                message: format!("{} exists, parses", path.display()),
                remediation: None,
            },
            Err(e) => Check {
                category: "files",
                name: "config_file",
                status: Status::Fail,
                message: format!("{} parse error: {e}", path.display()),
                remediation: Some("proteus show-defaults".into()),
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Check {
            category: "files",
            name: "config_file",
            status: Status::Skip,
            message: format!("{} missing — defaults are in effect", path.display()),
            remediation: None,
        },
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Check {
            category: "files",
            name: "config_file",
            status: Status::Skip,
            message: format!("{} unreadable (permission denied)", path.display()),
            remediation: Some("re-run as root for full detail".into()),
        },
        Err(e) => Check {
            category: "files",
            name: "config_file",
            status: Status::Warn,
            message: format!("{}: {e}", path.display()),
            remediation: None,
        },
    }
}

fn check_state_file(override_path: Option<&Path>) -> Check {
    use std::os::unix::fs::PermissionsExt;

    let path = super::state_path(override_path);
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                Check {
                    category: "files",
                    name: "state_file",
                    status: Status::Warn,
                    message: format!("{} mode is 0o{mode:o} — expected 0o600", path.display()),
                    remediation: Some(format!("chmod 0600 {}", path.display())),
                }
            } else {
                Check {
                    category: "files",
                    name: "state_file",
                    status: Status::Ok,
                    message: format!("{} exists, mode 0o600", path.display()),
                    remediation: None,
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Check {
            category: "files",
            name: "state_file",
            status: Status::Skip,
            message: format!("{} missing — first run on this system", path.display()),
            remediation: None,
        },
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Check {
            category: "files",
            name: "state_file",
            status: Status::Skip,
            message: format!("{} unreadable (permission denied)", path.display()),
            remediation: Some("re-run as root for full detail".into()),
        },
        Err(e) => Check {
            category: "files",
            name: "state_file",
            status: Status::Warn,
            message: format!("{}: {e}", path.display()),
            remediation: None,
        },
    }
}

// --- Detect-and-defer -----------------------------------------------------

fn push_detect_and_defer(out: &mut Vec<Check>, sys: &status_cmd::SystemInfo, quick: bool) {
    out.push(check_dns_competitors(quick));
    out.push(check_ntp_competitors());
    out.push(check_iface_managers(sys));
}

fn check_dns_competitors(quick: bool) -> Check {
    let mut found: Vec<String> = Vec::new();
    if binary_exists("dnscrypt-proxy") {
        found.push("dnscrypt-proxy".into());
    }
    if binary_exists("AdGuardHome") {
        found.push("AdGuardHome".into());
    }
    if binary_exists("kresd") {
        found.push("knot-resolver".into());
    }
    if process_exists("pihole-FTL") {
        found.push("Pi-hole".into());
    }
    if !quick && let Some(name) = scan_resolved_drop_ins() {
        found.push(name);
    }
    if !quick && resolv_conf_is_custom() {
        found.push("custom /etc/resolv.conf".into());
    }
    if found.is_empty() {
        Check {
            category: "detect_and_defer",
            name: "dns",
            status: Status::Ok,
            message: "no DNS-privacy tool conflict".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "detect_and_defer",
            name: "dns",
            status: Status::Warn,
            message: format!(
                "detected: {} — Proteus DNS knob will skip",
                found.join(", ")
            ),
            remediation: Some("proteus wiki dns".into()),
        }
    }
}

fn check_ntp_competitors() -> Check {
    let mut found: Vec<String> = Vec::new();
    if binary_exists("chronyd") {
        found.push("chrony".into());
    }
    if binary_exists("ntpd") {
        found.push("ntpd".into());
    }
    if found.is_empty() {
        Check {
            category: "detect_and_defer",
            name: "ntp",
            status: Status::Ok,
            message: "no NTP client conflict".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "detect_and_defer",
            name: "ntp",
            status: Status::Warn,
            message: format!(
                "detected: {} — Proteus NTP normalization will skip",
                found.join(", ")
            ),
            remediation: None,
        }
    }
}

fn check_iface_managers(sys: &status_cmd::SystemInfo) -> Check {
    let mut alt: Vec<&str> = Vec::new();
    if !sys.network_manager && binary_exists("iwd") {
        alt.push("iwd");
    }
    if !sys.network_manager && binary_exists("wpa_supplicant") {
        alt.push("wpa_supplicant");
    }
    if alt.is_empty() {
        Check {
            category: "detect_and_defer",
            name: "iface_manager",
            status: Status::Ok,
            message: if sys.network_manager {
                "NetworkManager is the active interface manager".into()
            } else {
                "no alternate interface manager detected".into()
            },
            remediation: None,
        }
    } else {
        Check {
            category: "detect_and_defer",
            name: "iface_manager",
            status: Status::Warn,
            message: format!(
                "no NetworkManager but found: {} — Proteus needs NM",
                alt.join(", ")
            ),
            remediation: Some("systemctl enable --now NetworkManager".into()),
        }
    }
}

// --- Runtime --------------------------------------------------------------

fn push_runtime(out: &mut Vec<Check>) {
    out.push(Check {
        category: "runtime",
        name: "version",
        status: Status::Ok,
        message: format!("proteus {} (phase {})", version::VERSION, version::PHASE),
        remediation: None,
    });
}

// --- Proteus state --------------------------------------------------------

fn push_proteus_state(out: &mut Vec<Check>, state_path: Option<&Path>) {
    let path = super::state_path(state_path);
    let state = State::load(&path).ok().flatten();
    out.push(check_original_cache(state.as_ref()));
    out.push(check_pinned_interfaces(state.as_ref()));
    out.push(check_last_rotation(state.as_ref()));
}

fn check_original_cache(state: Option<&State>) -> Check {
    match state {
        Some(s) if !s.original_macs.is_empty() => Check {
            category: "proteus_state",
            name: "original_macs",
            status: Status::Ok,
            message: format!("{} original MAC(s) cached", s.original_macs.len()),
            remediation: None,
        },
        Some(_) => Check {
            category: "proteus_state",
            name: "original_macs",
            status: Status::Skip,
            message: "no original MACs cached yet".into(),
            remediation: None,
        },
        None => Check {
            category: "proteus_state",
            name: "original_macs",
            status: Status::Skip,
            message: "no state file — Proteus has not run on this system".into(),
            remediation: None,
        },
    }
}

fn check_pinned_interfaces(state: Option<&State>) -> Check {
    let pins: Vec<String> = state
        .map(|s| {
            s.managed
                .interfaces
                .iter()
                .filter_map(|(name, rec)| rec.pinned.as_ref().map(|p| format!("{name}={p}")))
                .collect()
        })
        .unwrap_or_default();
    if pins.is_empty() {
        Check {
            category: "proteus_state",
            name: "pinned_interfaces",
            status: Status::Skip,
            message: "no pinned interfaces".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "proteus_state",
            name: "pinned_interfaces",
            status: Status::Ok,
            message: format!("{} pinned: {}", pins.len(), pins.join(", ")),
            remediation: None,
        }
    }
}

fn check_last_rotation(state: Option<&State>) -> Check {
    let entries: Vec<String> = state
        .map(|s| {
            s.managed
                .interfaces
                .iter()
                .filter_map(|(name, rec)| rec.last_rotated.as_ref().map(|t| format!("{name}@{t}")))
                .collect()
        })
        .unwrap_or_default();
    if entries.is_empty() {
        Check {
            category: "proteus_state",
            name: "last_rotation",
            status: Status::Skip,
            message: "no rotations recorded".into(),
            remediation: None,
        }
    } else {
        Check {
            category: "proteus_state",
            name: "last_rotation",
            status: Status::Ok,
            message: entries.join(", "),
            remediation: None,
        }
    }
}

// --- Helpers --------------------------------------------------------------

fn read_first_line(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_owned()))
        .filter(|s| !s.is_empty())
}

fn read_os_release() -> (String, String) {
    let raw = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = String::new();
    let mut version = String::new();
    for line in raw.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("ID=") {
            id = strip_quotes(v).to_string();
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = strip_quotes(v).to_string();
        }
    }
    (id, version)
}

fn strip_quotes(s: &str) -> &str {
    s.trim_matches('"').trim_matches('\'')
}

fn binary_exists(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let p: PathBuf = dir.join(name);
        p.is_file()
    })
}

fn process_exists(name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let s = fname.to_string_lossy();
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(content) = std::fs::read_to_string(&comm_path)
            && content.trim() == name
        {
            return true;
        }
    }
    false
}

fn scan_resolved_drop_ins() -> Option<String> {
    let dir = Path::new("/etc/systemd/resolved.conf.d");
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.ends_with(".conf") && !s.contains("proteus") {
            return Some(format!("non-proteus drop-in {}", s));
        }
    }
    None
}

fn resolv_conf_is_custom() -> bool {
    // systemd-resolved owns /etc/resolv.conf as a symlink to /run/systemd/resolve/...
    // anything else is a custom resolver.
    match std::fs::read_link("/etc/resolv.conf") {
        Ok(target) => !target.to_string_lossy().contains("/run/systemd/resolve/"),
        Err(_) => Path::new("/etc/resolv.conf").exists(),
    }
}

// --- Aggregation ----------------------------------------------------------

fn aggregate(checks: &[Check]) -> Summary {
    let mut s = Summary::default();
    for c in checks {
        match c.status {
            Status::Ok => s.ok += 1,
            Status::Warn => s.warn += 1,
            Status::Fail => s.fail += 1,
            Status::Skip => s.skip += 1,
        }
    }
    s
}

// --- Human renderer -------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RenderStyle {
    Unicode,
    Bracket,
}

fn render_style(no_color: bool) -> RenderStyle {
    if no_color || std::env::var_os("NO_COLOR").is_some() {
        RenderStyle::Bracket
    } else {
        RenderStyle::Unicode
    }
}

fn glyph(s: Status, style: RenderStyle) -> &'static str {
    match (style, s) {
        (RenderStyle::Unicode, Status::Ok) => "\u{2713}",
        (RenderStyle::Unicode, Status::Warn) => "\u{26A0}",
        (RenderStyle::Unicode, Status::Fail) => "\u{2717}",
        (RenderStyle::Unicode, Status::Skip) => "-",
        (RenderStyle::Bracket, Status::Ok) => "[ok]  ",
        (RenderStyle::Bracket, Status::Warn) => "[warn]",
        (RenderStyle::Bracket, Status::Fail) => "[fail]",
        (RenderStyle::Bracket, Status::Skip) => "[skip]",
    }
}

fn category_label(cat: &str) -> &'static str {
    match cat {
        "system" => "System",
        "daemons" => "Daemons",
        "files" => "Files",
        "detect_and_defer" => "Detect-and-defer",
        "runtime" => "Runtime",
        "proteus_state" => "Proteus state",
        _ => "Other",
    }
}

fn render_human(report: &Report, style: RenderStyle, verbose: bool) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(&format!(
        "proteus doctor — system health check (v{}, phase {})\n\n",
        report.proteus_version, report.phase
    ));

    let order = [
        "system",
        "daemons",
        "files",
        "detect_and_defer",
        "runtime",
        "proteus_state",
    ];
    for cat in order {
        let group: Vec<&Check> = report.checks.iter().filter(|c| c.category == cat).collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(category_label(cat));
        out.push('\n');
        for c in group {
            out.push_str("  ");
            out.push_str(glyph(c.status, style));
            out.push(' ');
            out.push_str(&c.message);
            out.push('\n');
            if verbose {
                out.push_str(&format!("      ({}::{})\n", c.category, c.name));
            }
            if let Some(r) = &c.remediation {
                out.push_str("      see: ");
                out.push_str(r);
                out.push('\n');
            }
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Summary: {} ok, {} warn, {} fail, {} skip\n",
        report.summary.ok, report.summary.warn, report.summary.fail, report.summary.skip
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(status: Status) -> Check {
        Check {
            category: "system",
            name: "x",
            status,
            message: "msg".into(),
            remediation: None,
        }
    }

    #[test]
    fn aggregate_counts_each_status() {
        let checks = vec![
            mk(Status::Ok),
            mk(Status::Ok),
            mk(Status::Warn),
            mk(Status::Fail),
            mk(Status::Skip),
            mk(Status::Skip),
            mk(Status::Skip),
        ];
        let s = aggregate(&checks);
        assert_eq!(s.ok, 2);
        assert_eq!(s.warn, 1);
        assert_eq!(s.fail, 1);
        assert_eq!(s.skip, 3);
    }

    #[test]
    fn glyph_bracket_variants_used_when_no_color() {
        assert_eq!(glyph(Status::Ok, RenderStyle::Bracket), "[ok]  ");
        assert_eq!(glyph(Status::Fail, RenderStyle::Bracket), "[fail]");
        assert_eq!(glyph(Status::Warn, RenderStyle::Bracket), "[warn]");
        assert_eq!(glyph(Status::Skip, RenderStyle::Bracket), "[skip]");
    }

    #[test]
    fn glyph_unicode_has_no_ansi() {
        for s in [Status::Ok, Status::Warn, Status::Fail, Status::Skip] {
            let g = glyph(s, RenderStyle::Unicode);
            assert!(!g.contains('\x1b'));
        }
    }

    #[test]
    fn human_render_groups_by_category_and_includes_summary() {
        let report = Report {
            schema_version: SCHEMA_VERSION,
            proteus_version: "9.9.9",
            phase: 'Z',
            checks: vec![
                Check {
                    category: "system",
                    name: "linux",
                    status: Status::Ok,
                    message: "Linux x".into(),
                    remediation: None,
                },
                Check {
                    category: "daemons",
                    name: "nm",
                    status: Status::Fail,
                    message: "down".into(),
                    remediation: Some("systemctl start NetworkManager".into()),
                },
            ],
            summary: Summary {
                ok: 1,
                warn: 0,
                fail: 1,
                skip: 0,
            },
        };
        let rendered = render_human(&report, RenderStyle::Bracket, false);
        assert!(rendered.contains("System"));
        assert!(rendered.contains("Daemons"));
        assert!(rendered.contains("[ok]"));
        assert!(rendered.contains("[fail]"));
        assert!(rendered.contains("see: systemctl start NetworkManager"));
        assert!(rendered.contains("Summary: 1 ok, 0 warn, 1 fail, 0 skip"));
    }

    #[test]
    fn verbose_includes_check_id() {
        let report = Report {
            schema_version: SCHEMA_VERSION,
            proteus_version: "9.9.9",
            phase: 'Z',
            checks: vec![Check {
                category: "system",
                name: "linux_kernel",
                status: Status::Ok,
                message: "Linux".into(),
                remediation: None,
            }],
            summary: Summary {
                ok: 1,
                warn: 0,
                fail: 0,
                skip: 0,
            },
        };
        let rendered = render_human(&report, RenderStyle::Bracket, true);
        assert!(rendered.contains("(system::linux_kernel)"));
    }

    #[test]
    fn strip_quotes_handles_double_and_single() {
        assert_eq!(strip_quotes("\"fedora\""), "fedora");
        assert_eq!(strip_quotes("'fedora'"), "fedora");
        assert_eq!(strip_quotes("fedora"), "fedora");
    }

    #[test]
    fn render_style_respects_no_color_flag() {
        assert_eq!(render_style(true), RenderStyle::Bracket);
    }
}
