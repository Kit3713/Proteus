// SPDX-License-Identifier: GPL-3.0-or-later

//! `systemd-timesyncd` NTP normalization (Milestone 4a).
//!
//! Writes a single drop-in at `/etc/systemd/timesyncd.conf.d/10-proteus.conf`
//! pinning the NTP pool to a privacy-respecting baseline. Hard-defers when
//! `chronyd` or `ntpd` is present — both have their own configuration
//! layers and Proteus will not fight them.
//!
//! Persona-aware customisation (per-region pools, persona-specific
//! servers) is the follow-up tracked in roadmap Milestone 4a.

use std::ffi::OsStr;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::commands;
use crate::config::NtpConfig;
use crate::dns::apply::sha256_hex;
use crate::version;

pub const PROTEUS_NTP_DROPIN_NAME: &str = "10-proteus.conf";
pub const TIMESYNCD_DROPIN_DIR: &str = "/etc/systemd/timesyncd.conf.d";

/// Roadmap Milestone 4a: persona-aware NTP server selection.
///
/// Real devices ship with vendor-specific NTP pools — iPhones hit
/// `time.apple.com`, Pixels hit `time.google.com`, Surface devices hit
/// `time.windows.com`. A passive observer can fingerprint a device by
/// the NTP queries on the wire alone, so the stealth covers shape this
/// surface alongside everything else. Returns the (primary, fallback)
/// pair to write into the timesyncd drop-in, or `None` when the
/// persona has no opinion (e.g. randomizer mirrors, generic-IoT).
///
/// Mapping is keyed first on persona id (precise covers — `iphone-15`,
/// `macbook-air-m3`) and falls back on category (any phone defaults to
/// a phone-shaped pool). Order matters: a precise id beats the
/// category default.
pub fn servers_for_persona(p: &crate::persona::Persona) -> Option<(Vec<String>, Vec<String>)> {
    use crate::persona::PersonaCategory;
    // Randomizer personas inherit the global pool — overriding to a
    // single-vendor pool would defeat the anonymity goal.
    if p.kind == crate::persona::PersonaKind::Randomizer {
        return None;
    }
    // Precise-id covers. Match by lowercase id stem; the catalogue
    // uses kebab-case ids so this is a stable lookup.
    let id_lc = p.id.to_ascii_lowercase();
    if id_lc.starts_with("iphone-")
        || id_lc.starts_with("ipad-")
        || id_lc.starts_with("macbook-")
    {
        return Some((
            vec!["time.apple.com".into()],
            vec!["time1.apple.com".into(), "time2.apple.com".into()],
        ));
    }
    if id_lc.starts_with("pixel-") || id_lc.starts_with("galaxy-") || id_lc.starts_with("chromecast") {
        return Some((
            vec!["time.google.com".into()],
            vec!["time1.google.com".into(), "time2.google.com".into()],
        ));
    }
    if id_lc.starts_with("surface-") {
        return Some((
            vec!["time.windows.com".into()],
            vec!["time.nist.gov".into()],
        ));
    }
    // Category-default fallback. Phones / tablets / TVs / consoles
    // historically ping vendor-NTP; without a vendor lock we keep the
    // global pool and skip overriding.
    let _ = PersonaCategory::Generic;
    None
}

/// Canonical install paths for the third-party NTP daemons we yield to.
const CHRONYD_BINS: &[&str] = &[
    "/usr/sbin/chronyd",
    "/usr/bin/chronyd",
    "/usr/local/sbin/chronyd",
];
const NTPD_BINS: &[&str] = &[
    "/usr/sbin/ntpd",
    "/usr/bin/ntpd",
    "/usr/local/sbin/ntpd",
];

/// Reasons we might defer. Mirrors the shape of `dns::DeferReason` so the
/// CLI can surface the same status vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail")]
pub enum DeferReason {
    /// A binary on a canonical install path matches.
    BinaryPresent { tool: &'static str, path: String },
    /// A systemd unit named for a third-party NTP tool reports active.
    ServiceActive {
        tool: &'static str,
        unit: &'static str,
    },
    /// A non-Proteus drop-in is present in `timesyncd.conf.d/`. Treated the
    /// same way `dns::detect_defer` treats foreign resolved drop-ins: the
    /// operator already shaped this; leave it alone.
    ForeignDropIn { path: String },
}

impl DeferReason {
    pub fn tool_name(&self) -> &str {
        match self {
            DeferReason::BinaryPresent { tool, .. } | DeferReason::ServiceActive { tool, .. } => {
                tool
            }
            DeferReason::ForeignDropIn { .. } => "foreign timesyncd.conf.d drop-in",
        }
    }
}

/// Filesystem layout the NTP module reads. Production callers always use
/// `Paths::system_default()`; tests pass `Paths::rooted_at(prefix)` to
/// re-route every absolute path under a tempdir.
#[derive(Debug, Clone)]
pub struct Paths {
    root: Option<PathBuf>,
}

impl Default for Paths {
    fn default() -> Self {
        Self::system_default()
    }
}

impl Paths {
    pub fn system_default() -> Self {
        Self { root: None }
    }

    #[cfg(test)]
    pub fn rooted_at(prefix: &Path) -> Self {
        Self {
            root: Some(prefix.to_path_buf()),
        }
    }

    pub fn resolve(&self, absolute: &str) -> PathBuf {
        match &self.root {
            Some(prefix) => prefix.join(absolute.trim_start_matches('/')),
            None => PathBuf::from(absolute),
        }
    }

    pub fn timesyncd_dropin_dir(&self) -> PathBuf {
        self.resolve(TIMESYNCD_DROPIN_DIR)
    }
}

/// Probe trait for the runtime checks the unit tests stub out. Mirrors
/// `dns::RuntimeProbe`.
pub trait RuntimeProbe {
    fn unit_is_active(&self, unit: &str) -> bool;
}

#[derive(Default)]
pub struct SystemProbe;

impl RuntimeProbe for SystemProbe {
    fn unit_is_active(&self, unit: &str) -> bool {
        let out = match Command::new("systemctl").args(["is-active", unit]).output() {
            Ok(o) => o,
            Err(_) => return false,
        };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        matches!(s.as_str(), "active" | "activating" | "reloading")
    }
}

/// Run all hard-guard checks. Returns the first reason to defer or `None`.
pub fn detect_defer<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    if let Some(r) = check_foreign_dropin(paths) {
        return Some(r);
    }
    if let Some(r) = check_chrony(paths, probe) {
        return Some(r);
    }
    if let Some(r) = check_ntpd(paths, probe) {
        return Some(r);
    }
    None
}

pub fn detect_defer_system(paths: &Paths) -> Option<DeferReason> {
    detect_defer(paths, &SystemProbe)
}

fn check_chrony<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    first_existing_bin(paths, "chrony", CHRONYD_BINS)
        .or_else(|| service_active(probe, "chrony", "chronyd.service"))
        .or_else(|| service_active(probe, "chrony", "chrony.service"))
}

fn check_ntpd<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    first_existing_bin(paths, "ntpd", NTPD_BINS)
        .or_else(|| service_active(probe, "ntpd", "ntpd.service"))
        .or_else(|| service_active(probe, "ntpd", "ntp.service"))
}

fn first_existing_bin(paths: &Paths, tool: &'static str, bins: &[&str]) -> Option<DeferReason> {
    bins.iter()
        .map(|b| paths.resolve(b))
        .find(|p| p.exists())
        .map(|p| DeferReason::BinaryPresent {
            tool,
            path: p.display().to_string(),
        })
}

fn service_active<P: RuntimeProbe>(
    probe: &P,
    tool: &'static str,
    unit: &'static str,
) -> Option<DeferReason> {
    probe
        .unit_is_active(unit)
        .then_some(DeferReason::ServiceActive { tool, unit })
}

fn check_foreign_dropin(paths: &Paths) -> Option<DeferReason> {
    let dir = paths.timesyncd_dropin_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_conf = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("conf"));
        if !is_conf {
            continue;
        }
        // Symlinks are always foreign — see the matching guard in dns/mod.rs.
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let name = entry.file_name();
        let is_proteus_owned = !is_symlink && name.to_string_lossy() == PROTEUS_NTP_DROPIN_NAME;
        if is_proteus_owned {
            continue;
        }
        return Some(DeferReason::ForeignDropIn {
            path: path.display().to_string(),
        });
    }
    None
}

/// Body of the drop-in **without** the management header. Pure function so
/// unit tests can assert formatting without touching `/etc`.
pub fn render_body(cfg: &NtpConfig) -> String {
    let mut out = String::from("[Time]\n");
    if !cfg.ntp_servers.is_empty() {
        out.push_str("NTP=");
        out.push_str(&cfg.ntp_servers.join(" "));
        out.push('\n');
    }
    if !cfg.fallback_servers.is_empty() {
        out.push_str("FallbackNTP=");
        out.push_str(&cfg.fallback_servers.join(" "));
        out.push('\n');
    }
    out
}

/// Full file contents: managed-file header + sha256 of body + body.
pub fn render_dropin(cfg: &NtpConfig) -> String {
    let body = render_body(cfg);
    let sha = sha256_hex(body.as_bytes());
    format!(
        "# managed by proteus v{version}\n# do not edit; manage via /etc/proteus/config.toml or `proteus ntp apply`\n# sha256:{sha}\n{body}",
        version = version::VERSION,
    )
}

pub fn dropin_path(paths: &Paths) -> PathBuf {
    paths.timesyncd_dropin_dir().join(PROTEUS_NTP_DROPIN_NAME)
}

pub fn write_dropin(paths: &Paths, cfg: &NtpConfig) -> Result<PathBuf> {
    let dir = paths.timesyncd_dropin_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating drop-in dir {}", dir.display()))?;
    let path = dropin_path(paths);
    let body = render_dropin(cfg);
    commands::write_atomic(&path, body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn remove_dropin(paths: &Paths) -> Result<bool> {
    let path = dropin_path(paths);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow::Error::new(e).context(format!("removing {}", path.display()))),
    }
}

pub fn dropin_present(paths: &Paths) -> bool {
    dropin_path(paths).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[derive(Default)]
    struct MockProbe {
        active_units: Vec<&'static str>,
    }

    impl RuntimeProbe for MockProbe {
        fn unit_is_active(&self, unit: &str) -> bool {
            self.active_units.contains(&unit)
        }
    }

    fn clean_root() -> crate::testing::TempRoot {
        let root = crate::testing::TempRoot::new("ntp");
        fs::create_dir_all(root.path.join("etc/systemd/timesyncd.conf.d")).unwrap();
        root
    }

    fn cfg() -> NtpConfig {
        NtpConfig::default()
    }

    #[test]
    fn clean_system_does_not_defer() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        assert!(detect_defer(&paths, &probe).is_none());
    }

    #[test]
    fn chronyd_binary_trips_the_guard() {
        let root = clean_root();
        let bin_dir = root.path.join("usr/sbin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("chronyd"), "").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        assert!(matches!(reason, DeferReason::BinaryPresent { tool, .. } if tool == "chrony"));
    }

    #[test]
    fn ntpd_binary_trips_the_guard() {
        let root = clean_root();
        let bin_dir = root.path.join("usr/sbin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("ntpd"), "").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        assert!(matches!(reason, DeferReason::BinaryPresent { tool, .. } if tool == "ntpd"));
    }

    #[test]
    fn chronyd_active_unit_trips_the_guard() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe {
            active_units: vec!["chronyd.service"],
        };
        let reason = detect_defer(&paths, &probe).expect("should defer");
        match reason {
            DeferReason::ServiceActive { tool, unit } => {
                assert_eq!(tool, "chrony");
                assert_eq!(unit, "chronyd.service");
            }
            other => panic!("expected ServiceActive, got {other:?}"),
        }
    }

    #[test]
    fn foreign_dropin_trips_the_guard() {
        let root = clean_root();
        let dir = root.path.join("etc/systemd/timesyncd.conf.d");
        fs::write(dir.join("99-mine.conf"), "[Time]\nNTP=time.example.com\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        match reason {
            DeferReason::ForeignDropIn { path } => assert!(path.ends_with("99-mine.conf")),
            other => panic!("expected ForeignDropIn, got {other:?}"),
        }
    }

    #[test]
    fn proteus_dropin_does_not_trip_the_guard() {
        let root = clean_root();
        let dir = root.path.join("etc/systemd/timesyncd.conf.d");
        fs::write(
            dir.join(PROTEUS_NTP_DROPIN_NAME),
            "# managed by proteus\n[Time]\nNTP=2.fedora.pool.ntp.org\n",
        )
        .unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        assert!(detect_defer(&paths, &probe).is_none());
    }

    #[test]
    fn render_body_emits_ntp_and_fallback_lines() {
        let body = render_body(&cfg());
        assert!(body.contains("[Time]"));
        assert!(body.contains("NTP=2.fedora.pool.ntp.org"));
        assert!(body.contains("FallbackNTP=time.cloudflare.com"));
    }

    #[test]
    fn render_body_with_empty_lists_skips_keys() {
        let c = NtpConfig {
            enabled: true,
            ntp_servers: vec![],
            fallback_servers: vec![],
        };
        let body = render_body(&c);
        assert_eq!(body, "[Time]\n");
        assert!(!body.contains("NTP="));
        assert!(!body.contains("FallbackNTP="));
    }

    #[test]
    fn render_body_joins_multiple_servers_with_spaces() {
        let c = NtpConfig {
            enabled: true,
            ntp_servers: vec!["a".into(), "b".into(), "c".into()],
            fallback_servers: vec![],
        };
        let body = render_body(&c);
        assert!(body.contains("NTP=a b c"));
    }

    #[test]
    fn render_dropin_includes_sha_of_body() {
        let c = cfg();
        let body = render_body(&c);
        let expected = sha256_hex(body.as_bytes());
        let full = render_dropin(&c);
        assert!(full.contains(&format!("sha256:{expected}")));
        assert!(full.contains("# managed by proteus"));
        assert!(full.ends_with(&body));
    }

    #[test]
    fn write_then_remove_round_trips() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let c = cfg();

        assert!(!dropin_present(&paths));
        let path = write_dropin(&paths, &c).expect("write");
        assert!(dropin_present(&paths));
        assert!(path.ends_with(PROTEUS_NTP_DROPIN_NAME));

        // Idempotent remove.
        assert!(remove_dropin(&paths).expect("remove"));
        assert!(!dropin_present(&paths));
        assert!(!remove_dropin(&paths).expect("remove-again"));
    }

    #[test]
    fn dropin_path_uses_proteus_filename() {
        let paths = Paths::default();
        let p = dropin_path(&paths);
        assert!(p.ends_with(PROTEUS_NTP_DROPIN_NAME));
        // Lives under timesyncd, not resolved.
        assert!(p.to_string_lossy().contains("timesyncd.conf.d"));
    }

    #[test]
    fn revert_only_removes_proteus_managed_file() {
        // Apply path writes one specific name; revert must not touch the
        // adjacent third-party drop-in even when both are in the directory.
        let root = clean_root();
        let dir = root.path.join("etc/systemd/timesyncd.conf.d");
        fs::write(dir.join("99-third-party.conf"), "[Time]\nNTP=other\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        // Bypass the foreign-drop-in guard for this test by writing the
        // managed file via the helper directly — the apply path itself
        // would refuse to write here.
        write_dropin(&paths, &cfg()).expect("write");

        assert!(remove_dropin(&paths).expect("remove proteus file"));
        // Proteus-managed file is gone.
        assert!(!dir.join(PROTEUS_NTP_DROPIN_NAME).exists());
        // Third-party file is untouched.
        assert!(dir.join("99-third-party.conf").exists());
    }

    /// Roadmap Milestone 4a: persona-aware NTP server selection.
    /// Apple persona ids map to time.apple.com, Pixel/Galaxy/Chromecast
    /// to time.google.com, Surface to time.windows.com.
    #[test]
    fn servers_for_persona_maps_apple_ids_to_apple_pool() {
        let p = sample_stealth("iphone-15", crate::persona::PersonaCategory::Phone);
        let (primary, fallback) = servers_for_persona(&p).expect("apple mapping");
        assert!(primary.iter().any(|s| s == "time.apple.com"));
        assert!(fallback.iter().any(|s| s.contains("apple.com")));
    }

    #[test]
    fn servers_for_persona_maps_google_ids_to_google_pool() {
        for id in ["pixel-8", "galaxy-s24", "chromecast"] {
            let p = sample_stealth(id, crate::persona::PersonaCategory::Phone);
            let (primary, _) = servers_for_persona(&p).expect("google mapping");
            assert!(primary.iter().any(|s| s == "time.google.com"));
        }
    }

    #[test]
    fn servers_for_persona_returns_none_for_randomizer_personas() {
        let mut p = sample_stealth("iphone-15", crate::persona::PersonaCategory::Phone);
        p.kind = crate::persona::PersonaKind::Randomizer;
        assert!(servers_for_persona(&p).is_none());
    }

    #[test]
    fn servers_for_persona_returns_none_for_unmapped_personas() {
        let p = sample_stealth("router-tplink", crate::persona::PersonaCategory::Router);
        assert!(servers_for_persona(&p).is_none());
    }

    fn sample_stealth(
        id: &str,
        category: crate::persona::PersonaCategory,
    ) -> crate::persona::Persona {
        crate::persona::Persona {
            id: id.into(),
            display_name: id.into(),
            kind: crate::persona::PersonaKind::Stealth,
            category,
            oui_pool: vec![],
            mac_byte_pattern: None,
            hostname_template: "{owner}".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: false,
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        }
    }

    #[test]
    fn defer_reason_tool_name_covers_every_variant() {
        let bin = DeferReason::BinaryPresent {
            tool: "chrony",
            path: "/usr/sbin/chronyd".into(),
        };
        let svc = DeferReason::ServiceActive {
            tool: "ntpd",
            unit: "ntpd.service",
        };
        let foreign = DeferReason::ForeignDropIn {
            path: "/etc/systemd/timesyncd.conf.d/99-x.conf".into(),
        };
        assert_eq!(bin.tool_name(), "chrony");
        assert_eq!(svc.tool_name(), "ntpd");
        assert!(foreign.tool_name().contains("foreign"));
    }
}
