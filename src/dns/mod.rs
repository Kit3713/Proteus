// SPDX-License-Identifier: GPL-3.0-or-later

//! DNS feature: ECS-strip on systemd-resolved with detect-and-defer.
//!
//! Phase D ships exactly one knob: `EDNSClientSubnet=no` written to a
//! drop-in under `/etc/systemd/resolved.conf.d/`. Anything that suggests
//! the user already runs a more-specialized DNS-privacy tool causes
//! Proteus to bow out cleanly — the user's setup wins, every time.
//!
//! See `wiki/dns.md` and `wiki/concepts.md` (detect-and-defer) for the
//! design rationale.

pub mod apply;
pub mod resolved;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Marker prefix for drop-in filenames Proteus is allowed to own.
pub const PROTEUS_DROPIN_PREFIX: &str = "10-proteus-";

/// The single drop-in filename Proteus writes.
pub const PROTEUS_DROPIN_NAME: &str = "10-proteus-no-ecs.conf";

/// Standard location of systemd-resolved drop-ins.
pub const RESOLVED_DROPIN_DIR: &str = "/etc/systemd/resolved.conf.d";

/// systemd-resolved's stub resolver target. If `/etc/resolv.conf` is a
/// symlink to this path, resolved is in charge — anything else means the
/// user's resolver setup deviates and Proteus must defer.
pub const RESOLVED_STUB_PATH: &str = "/run/systemd/resolve/stub-resolv.conf";

/// Reasons we might defer. Each variant carries the tool name that wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail")]
pub enum DeferReason {
    /// A binary on the user's PATH or canonical install location matches.
    BinaryPresent { tool: &'static str, path: String },
    /// A systemd unit named for a third-party DNS tool reports active.
    ServiceActive {
        tool: &'static str,
        unit: &'static str,
    },
    /// A bare process named for a third-party DNS tool is running.
    ProcessRunning {
        tool: &'static str,
        process: &'static str,
    },
    /// `/etc/resolv.conf` is not the systemd-resolved stub symlink.
    CustomResolvConf { detail: String },
    /// A non-Proteus drop-in is present in `resolved.conf.d/`.
    ForeignDropIn { path: String },
    /// Something other than systemd-resolved is bound to localhost:53.
    LocalhostResolverBound { detail: String },
}

impl DeferReason {
    /// Short human-readable name of the tool that wins.
    pub fn tool_name(&self) -> &str {
        match self {
            DeferReason::BinaryPresent { tool, .. }
            | DeferReason::ServiceActive { tool, .. }
            | DeferReason::ProcessRunning { tool, .. } => tool,
            DeferReason::CustomResolvConf { .. } => "custom /etc/resolv.conf",
            DeferReason::ForeignDropIn { .. } => "foreign resolved.conf.d drop-in",
            DeferReason::LocalhostResolverBound { .. } => "local DNS listener",
        }
    }
}

/// Static lookup tables for the third-party tools we detect. Kept as
/// `&'static [&'static str]` so they cost nothing at runtime and stay
/// trivially auditable.
pub const DNSCRYPT_PROXY_BINS: &[&str] = &[
    "/usr/bin/dnscrypt-proxy",
    "/usr/local/bin/dnscrypt-proxy",
    "/usr/sbin/dnscrypt-proxy",
];
pub const ADGUARDHOME_BINS: &[&str] = &[
    "/usr/bin/AdGuardHome",
    "/usr/local/bin/AdGuardHome",
    "/opt/AdGuardHome/AdGuardHome",
];
pub const KNOT_RESOLVER_BIN: &str = "/usr/sbin/kresd";
pub const UNBOUND_BIN: &str = "/usr/sbin/unbound";
pub const BIND_NAMED_BIN: &str = "/usr/sbin/named";
pub const PIHOLE_FTL_BIN: &str = "/usr/bin/pihole-FTL";

/// Filesystem layout the DNS module reads. Production code always uses
/// `Paths::system_default()`; tests use `Paths::rooted_at(prefix)` to point
/// every absolute path at a tempdir.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Empty in the system layout, set to the tempdir root in tests. Every
    /// path probe is `root.join(absolute_path.strip_prefix("/"))`.
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

    /// Re-root every path under `prefix`. Used in tests to mock fs layout
    /// without granting tests the right to scribble on `/etc`.
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

    pub fn resolved_dropin_dir(&self) -> PathBuf {
        self.resolve(RESOLVED_DROPIN_DIR)
    }
    pub fn resolv_conf(&self) -> PathBuf {
        self.resolve("/etc/resolv.conf")
    }
    pub fn resolved_stub(&self) -> PathBuf {
        self.resolve(RESOLVED_STUB_PATH)
    }
}

/// Hooks for runtime probes the unit tests stub out. Real callers use
/// `RuntimeProbe::system()`.
pub trait RuntimeProbe {
    /// Returns true if the named unit is active (`systemctl is-active`).
    fn unit_is_active(&self, unit: &str) -> bool;
    /// Returns true if a process with this exact comm name is running.
    fn process_is_running(&self, name: &str) -> bool;
    /// Returns true if anything other than systemd-resolved is bound to
    /// 127.0.0.1:53 / [::1]:53. Returning a description on `Some` lets
    /// status surface what we found.
    fn foreign_localhost_dns_listener(&self) -> Option<String>;
}

/// Real systemd / procfs / `ss` based probe.
#[derive(Default)]
pub struct SystemProbe;

impl RuntimeProbe for SystemProbe {
    fn unit_is_active(&self, unit: &str) -> bool {
        let out = match Command::new(crate::process::systemctl()).args(["is-active", unit]).output() {
            Ok(o) => o,
            Err(_) => return false,
        };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        matches!(s.as_str(), "active" | "activating" | "reloading")
    }

    fn process_is_running(&self, name: &str) -> bool {
        // Read /proc/*/comm rather than spawning pgrep — fewer dependencies,
        // works in a minimal container.
        let dir = match std::fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => return false,
        };
        for entry in dir.flatten() {
            let fname = entry.file_name();
            let s = fname.to_string_lossy();
            if !s.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let comm_path = entry.path().join("comm");
            if let Ok(text) = std::fs::read_to_string(&comm_path) {
                if text.trim() == name {
                    return true;
                }
            }
        }
        false
    }

    fn foreign_localhost_dns_listener(&self) -> Option<String> {
        // Try `ss -tnlpH 'sport = :53'`. If ss is missing, return None — the
        // other guard checks (binary presence, drop-ins) carry the load.
        let out = Command::new(crate::process::ss_bin())
            .args(["-tnlpH", "sport = :53"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        parse_ss_for_foreign_listener(&text)
    }
}

/// Parser for `ss -tnlpH 'sport = :53'` output. Kept pure so unit tests can
/// feed it canned strings without spawning ss.
pub(crate) fn parse_ss_for_foreign_listener(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // ss columns vary across versions; we only need to know whether the
        // local address is one of the loopback :53 sockets and whether the
        // process column mentions systemd-resolve.
        if !trimmed.contains("127.0.0.1:53") && !trimmed.contains("::1]:53") {
            continue;
        }
        if trimmed.contains("systemd-resolve") {
            continue;
        }
        // Extract the users=... segment if present for a useful detail.
        let detail = trimmed
            .split("users:")
            .nth(1)
            .map(|s| s.trim().trim_matches(|c: char| c == '(' || c == ')'))
            .unwrap_or(trimmed);
        return Some(detail.to_string());
    }
    None
}

/// Run all hard-guard checks. Returns the first reason to defer or `None`
/// if the system is clear.
pub fn detect_defer<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    if let Some(r) = check_resolv_conf(paths) {
        return Some(r);
    }
    if let Some(r) = check_foreign_dropin(paths) {
        return Some(r);
    }
    if let Some(r) = check_dnscrypt_proxy(paths, probe) {
        return Some(r);
    }
    if let Some(r) = check_pihole(paths, probe) {
        return Some(r);
    }
    if let Some(r) = check_adguard_home(paths, probe) {
        return Some(r);
    }
    if let Some(r) = check_other_resolvers(paths, probe) {
        return Some(r);
    }
    if let Some(detail) = probe.foreign_localhost_dns_listener() {
        return Some(DeferReason::LocalhostResolverBound { detail });
    }
    None
}

/// Convenience wrapper used by all callers outside tests.
pub fn detect_defer_system(paths: &Paths) -> Option<DeferReason> {
    detect_defer(paths, &SystemProbe)
}

fn check_resolv_conf(paths: &Paths) -> Option<DeferReason> {
    let p = paths.resolv_conf();
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(_) => {
            // No /etc/resolv.conf at all is unusual; treat as deferred so
            // we don't paper over a misconfigured system.
            return Some(DeferReason::CustomResolvConf {
                detail: format!("{} missing", p.display()),
            });
        }
    };
    if !meta.file_type().is_symlink() {
        return Some(DeferReason::CustomResolvConf {
            detail: format!("{} is a regular file", p.display()),
        });
    }
    let target = match std::fs::read_link(&p) {
        Ok(t) => t,
        Err(_) => {
            return Some(DeferReason::CustomResolvConf {
                detail: format!("{} is a symlink but unreadable", p.display()),
            });
        }
    };
    if !points_to_resolved_stub(&target, &paths.resolved_stub(), &p) {
        return Some(DeferReason::CustomResolvConf {
            detail: format!("{} -> {}", p.display(), target.display()),
        });
    }
    None
}

fn points_to_resolved_stub(target: &Path, expected: &Path, link_path: &Path) -> bool {
    // Tail of the well-known stub path. Fedora's default layout ships
    // `/etc/resolv.conf -> ../run/systemd/resolve/stub-resolv.conf`, so a
    // literal absolute-path check on `target` (the read_link result) would
    // miss the relative variant.
    const STUB_TAIL: &str = "run/systemd/resolve/stub-resolv.conf";

    let _ = target; // kept in the signature so callers don't have to change
    if target == expected {
        return true;
    }
    // Canonicalize to follow the full symlink chain. Defends against an
    // attacker who chains symlinks so the first read_link looks like the
    // stub but actually resolves to attacker-controlled content.
    //
    // Issue #210: previously, a `canonicalize` failure (broken link, missing
    // target, dangling chain) fell back to a literal tail-string match on
    // the read_link result. That fall-open behaviour bypassed the DNS
    // detect-and-defer guard — an attacker who plants
    // `/etc/resolv.conf -> /var/lib/proteus-evil/run/systemd/resolve/stub-resolv.conf`
    // pointing at a non-existent target would canon-fail and then pass the
    // tail check. We now refuse: if canonicalize errors, the link does NOT
    // count as pointing at the well-known stub, so the caller defers.
    match std::fs::canonicalize(link_path) {
        Ok(canon_link) => canon_link == *expected || canon_link.to_string_lossy().ends_with(STUB_TAIL),
        Err(_) => false,
    }
}

fn check_foreign_dropin(paths: &Paths) -> Option<DeferReason> {
    let dir = paths.resolved_dropin_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        // systemd-resolved only reads `.conf` files, so anything else is noise.
        let is_conf = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("conf"));
        if !is_conf {
            continue;
        }
        // Proteus's atomic writer only ever produces regular files, so a
        // symlink in this directory is by construction not ours — even if
        // its filename matches our prefix. Treating any symlink as foreign
        // closes the redirection where an attacker plants
        // `10-proteus-X.conf -> /attacker/payload` and slips past the
        // prefix-skip below.
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let name = entry.file_name();
        let is_proteus_owned =
            !is_symlink && name.to_string_lossy().starts_with(PROTEUS_DROPIN_PREFIX);
        if is_proteus_owned {
            continue;
        }
        return Some(DeferReason::ForeignDropIn {
            path: path.display().to_string(),
        });
    }
    None
}

/// Check a list of candidate binaries; return `BinaryPresent` for the first
/// one that exists. Used by every per-tool guard so the matching pattern
/// stays in one place.
fn first_existing_bin(paths: &Paths, tool: &'static str, bins: &[&str]) -> Option<DeferReason> {
    bins.iter()
        .map(|b| paths.resolve(b))
        .find(|p| p.exists())
        .map(|p| DeferReason::BinaryPresent {
            tool,
            path: p.display().to_string(),
        })
}

fn service_active_reason<P: RuntimeProbe>(
    probe: &P,
    tool: &'static str,
    unit: &'static str,
) -> Option<DeferReason> {
    probe
        .unit_is_active(unit)
        .then_some(DeferReason::ServiceActive { tool, unit })
}

fn check_dnscrypt_proxy<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    first_existing_bin(paths, "dnscrypt-proxy", DNSCRYPT_PROXY_BINS)
        .or_else(|| service_active_reason(probe, "dnscrypt-proxy", "dnscrypt-proxy.service"))
}

fn check_pihole<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    first_existing_bin(paths, "pi-hole", &[PIHOLE_FTL_BIN])
        .or_else(|| {
            probe
                .process_is_running("pihole-FTL")
                .then_some(DeferReason::ProcessRunning {
                    tool: "pi-hole",
                    process: "pihole-FTL",
                })
        })
        .or_else(|| service_active_reason(probe, "pi-hole", "pihole-FTL.service"))
}

fn check_adguard_home<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    first_existing_bin(paths, "AdGuardHome", ADGUARDHOME_BINS)
        .or_else(|| service_active_reason(probe, "AdGuardHome", "AdGuardHome.service"))
}

fn check_other_resolvers<P: RuntimeProbe>(paths: &Paths, probe: &P) -> Option<DeferReason> {
    // (tool, [binary candidates], [unit candidates]). One row per resolver.
    // `kresd@1.service` is the typical templated unit name on Fedora.
    const RESOLVERS: &[(&str, &[&str], &[&str])] = &[
        (
            "knot-resolver",
            &[KNOT_RESOLVER_BIN],
            &["kresd@1.service", "kresd.service"],
        ),
        ("unbound", &[UNBOUND_BIN], &["unbound.service"]),
        ("bind", &[BIND_NAMED_BIN], &["named.service"]),
    ];
    for (tool, bins, units) in RESOLVERS {
        if let Some(r) = first_existing_bin(paths, tool, bins) {
            return Some(r);
        }
        for unit in *units {
            if probe.unit_is_active(unit) {
                return Some(DeferReason::ServiceActive { tool, unit });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    /// Test probe that returns canned answers — keeps unit tests off the
    /// real systemctl / procfs.
    #[derive(Default)]
    struct MockProbe {
        active_units: Vec<&'static str>,
        running_processes: Vec<&'static str>,
        listener: Option<String>,
    }

    impl RuntimeProbe for MockProbe {
        fn unit_is_active(&self, unit: &str) -> bool {
            self.active_units.contains(&unit)
        }
        fn process_is_running(&self, name: &str) -> bool {
            self.running_processes.contains(&name)
        }
        fn foreign_localhost_dns_listener(&self) -> Option<String> {
            self.listener.clone()
        }
    }

    /// Set up a tempdir that simulates a clean Fedora 43 system: stub
    /// symlink in place, no foreign drop-ins, no third-party binaries.
    fn clean_root() -> tempdir::TempRoot {
        let root = tempdir::TempRoot::new();
        let etc = root.path.join("etc");
        let dropin = etc.join("systemd/resolved.conf.d");
        fs::create_dir_all(&dropin).unwrap();
        let stub_dir = root.path.join("run/systemd/resolve");
        fs::create_dir_all(&stub_dir).unwrap();
        let stub_file = stub_dir.join("stub-resolv.conf");
        fs::write(&stub_file, "# stub\n").unwrap();
        // /etc/resolv.conf -> ../run/systemd/resolve/stub-resolv.conf
        symlink(
            "../run/systemd/resolve/stub-resolv.conf",
            etc.join("resolv.conf"),
        )
        .unwrap();
        root
    }

    #[test]
    fn clean_system_defers_to_nothing() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        assert!(detect_defer(&paths, &probe).is_none());
    }

    #[test]
    fn dnscrypt_proxy_binary_trips_the_guard() {
        let root = clean_root();
        let bin_dir = root.path.join("usr/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("dnscrypt-proxy"), "").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        match reason {
            DeferReason::BinaryPresent { tool, .. } => assert_eq!(tool, "dnscrypt-proxy"),
            other => panic!("expected BinaryPresent, got {other:?}"),
        }
    }

    #[test]
    fn pihole_ftl_process_trips_the_guard() {
        let root = clean_root();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe {
            running_processes: vec!["pihole-FTL"],
            ..Default::default()
        };
        let reason = detect_defer(&paths, &probe).expect("should defer");
        assert!(matches!(reason, DeferReason::ProcessRunning { tool, .. } if tool == "pi-hole"));
    }

    #[test]
    fn foreign_dropin_trips_the_guard() {
        let root = clean_root();
        let dropin_dir = root.path.join("etc/systemd/resolved.conf.d");
        fs::write(dropin_dir.join("99-mine.conf"), "[Resolve]\nDNS=1.1.1.1\n").unwrap();
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
        let dropin_dir = root.path.join("etc/systemd/resolved.conf.d");
        fs::write(
            dropin_dir.join(PROTEUS_DROPIN_NAME),
            "# managed by proteus\n[Resolve]\nEDNSClientSubnet=no\n",
        )
        .unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        assert!(detect_defer(&paths, &probe).is_none());
    }

    /// Issue #130: a symlink in the drop-in dir whose name matches the
    /// proteus prefix must still be flagged as foreign — Proteus's atomic
    /// writer never produces symlinks, so any symlink there is attacker
    /// (or admin) content masquerading as managed.
    #[test]
    fn proteus_prefixed_symlink_dropin_trips_the_guard() {
        let root = clean_root();
        let dropin_dir = root.path.join("etc/systemd/resolved.conf.d");
        let attacker_payload = root.path.join("attacker.conf");
        fs::write(&attacker_payload, "[Resolve]\nDNS=10.0.0.1\n").unwrap();
        symlink(&attacker_payload, dropin_dir.join("10-proteus-evil.conf")).unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        match reason {
            DeferReason::ForeignDropIn { path } => {
                assert!(path.ends_with("10-proteus-evil.conf"));
            }
            other => panic!("expected ForeignDropIn, got {other:?}"),
        }
    }

    #[test]
    fn custom_resolv_conf_real_file_trips_the_guard() {
        let root = clean_root();
        // Replace symlink with a real file.
        fs::remove_file(root.path.join("etc/resolv.conf")).unwrap();
        fs::write(root.path.join("etc/resolv.conf"), "nameserver 1.1.1.1\n").unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        assert!(matches!(reason, DeferReason::CustomResolvConf { .. }));
    }

    /// Issue #210: a symlink chain that *names* the well-known stub but
    /// canonicalize-fails (e.g. dangling target) must NOT be accepted as
    /// "points at the stub". Previously the suffix-match fallback let an
    /// attacker plant such a chain and pass the DNS guard. The fix drops
    /// the fallback; the guard now defers when canonicalize errors.
    #[test]
    fn dangling_resolv_conf_chain_trips_the_guard() {
        let root = clean_root();
        // Replace the legitimate stub symlink with one whose tail matches
        // STUB_TAIL but whose target does not exist.
        fs::remove_file(root.path.join("etc/resolv.conf")).unwrap();
        let dangling = root.path.join("does/not/exist/run/systemd/resolve/stub-resolv.conf");
        symlink(&dangling, root.path.join("etc/resolv.conf")).unwrap();
        let paths = Paths::rooted_at(&root.path);
        let probe = MockProbe::default();
        let reason = detect_defer(&paths, &probe).expect("should defer");
        assert!(matches!(reason, DeferReason::CustomResolvConf { .. }));
    }

    #[test]
    fn ss_parser_ignores_systemd_resolve() {
        let line =
            "LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:* users:((\"systemd-resolve\",pid=1,fd=14))";
        assert!(parse_ss_for_foreign_listener(line).is_none());
    }

    #[test]
    fn ss_parser_finds_dnscrypt_proxy() {
        let line = "LISTEN 0 128 127.0.0.1:53 0.0.0.0:* users:((\"dnscrypt-proxy\",pid=42,fd=7))";
        let detail = parse_ss_for_foreign_listener(line).expect("should match");
        assert!(detail.contains("dnscrypt-proxy"));
    }

    /// Tiny tempdir helper. Avoids pulling in the `tempfile` crate to stay
    /// dep-free per project policy. Uses `getrandom` for the suffix so two
    /// concurrent test runs cannot collide.
    pub mod tempdir {
        use std::path::PathBuf;

        pub struct TempRoot {
            pub path: PathBuf,
        }

        impl TempRoot {
            pub fn new() -> Self {
                let mut buf = [0u8; 8];
                getrandom::getrandom(&mut buf).unwrap();
                let suffix: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
                let path = std::env::temp_dir().join(format!("proteus-dns-test-{}", suffix));
                std::fs::create_dir_all(&path).unwrap();
                Self { path }
            }
        }

        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }
}
