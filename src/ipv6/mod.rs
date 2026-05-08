// SPDX-License-Identifier: GPL-3.0-or-later

//! IPv6 fingerprint hardening: stable-privacy IIDs, temporary addresses, and
//! DUID rotation.
//!
//! Two layers cooperate so a network sees a fresh-looking host every visit:
//!
//! * Per-interface sysctls written to a single drop-in at
//!   `/etc/sysctl.d/96-proteus-ipv6.conf`. Kernel knobs covered:
//!   `use_tempaddr`, `addr_gen_mode`, `temp_valid_lft`, `temp_prefered_lft`.
//! * Per-NM-connection settings written via DBus (mirrors the DHCP path):
//!   `ipv6.addr-gen-mode`, `ipv6.dhcp-duid`, `ipv6.dhcp-iaid`.
//!
//! Originals (the pre-Proteus sysctl values) are cached into `state.json`
//! exactly once per interface so `revert` can restore them. The drop-in
//! header carries a SHA-256 of its body so `proteus diff` can spot manual
//! edits without re-running the full apply.
//!
//! See `proteus wiki ipv6` for the threat model + verification recipes.

pub mod nm;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::crypto::sha256;
use crate::version;

/// Drop-in path. `96-` puts us late enough that operator overrides at `99-*`
/// still win, but ahead of distro defaults at `99-*-default`.
pub const DROPIN_PATH: &str = "/etc/sysctl.d/96-proteus-ipv6.conf";

/// Name of the proc-sysctl tree we inspect/restore from. Resolved against an
/// optional `root_prefix` so tests can drive it with a tmpdir.
pub const PROC_BASE: &str = "/proc/sys/net/ipv6/conf";

/// Settings Proteus manages, in the order they appear in the drop-in.
pub const SYSCTLS: &[Sysctl] = &[
    Sysctl {
        key: "use_tempaddr",
        value: "2",
        description: "prefer temporary IPv6 addresses for outbound (RFC 8981)",
    },
    Sysctl {
        key: "addr_gen_mode",
        value: "3",
        description: "stable-privacy IID generation (RFC 7217)",
    },
    Sysctl {
        key: "temp_valid_lft",
        value: "86400",
        description: "1d valid lifetime for temp addresses",
    },
    Sysctl {
        key: "temp_prefered_lft",
        value: "7200",
        description: "2h preferred lifetime for temp addresses",
    },
];

/// One sysctl Proteus owns. Static — values are policy, not user-tunable.
#[derive(Debug, Clone, Copy)]
pub struct Sysctl {
    pub key: &'static str,
    pub value: &'static str,
    pub description: &'static str,
}

/// Snapshot of one interface's IPv6 sysctls at apply-time.
#[derive(Debug, Clone, Default)]
pub struct InterfaceSnapshot {
    pub iface: String,
    pub use_tempaddr: Option<String>,
    pub addr_gen_mode: Option<String>,
    pub temp_valid_lft: Option<String>,
    pub temp_prefered_lft: Option<String>,
}

impl InterfaceSnapshot {
    pub fn lookup(&self, key: &str) -> Option<&str> {
        match key {
            "use_tempaddr" => self.use_tempaddr.as_deref(),
            "addr_gen_mode" => self.addr_gen_mode.as_deref(),
            "temp_valid_lft" => self.temp_valid_lft.as_deref(),
            "temp_prefered_lft" => self.temp_prefered_lft.as_deref(),
            _ => None,
        }
    }
}

/// Render the contents of `/etc/sysctl.d/96-proteus-ipv6.conf` for the supplied
/// interfaces. The header carries a SHA-256 of the body so `proteus diff` can
/// detect manual edits.
pub fn render_dropin(ifaces: &[&str]) -> String {
    let body = render_body(ifaces);
    let digest = sha256::hex_digest(body.as_bytes());
    let header = format!(
        "# managed by proteus v{ver}\n# do not edit; manage via `proteus ipv6 apply`\n# sha256: {digest}\n",
        ver = version::VERSION
    );
    format!("{header}{body}")
}

fn render_body(ifaces: &[&str]) -> String {
    let mut out = String::with_capacity(64 * (ifaces.len().max(1)) * SYSCTLS.len());
    for (idx, iface) in ifaces.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!("# {iface}\n"));
        for s in SYSCTLS {
            out.push_str(&format!(
                "net.ipv6.conf.{iface}.{key} = {val}\n",
                key = s.key,
                val = s.value
            ));
        }
    }
    out
}

/// Read the four sysctls Proteus manages for `iface` from `/proc`. Missing
/// files become `None` rather than errors — interfaces come and go on Linux,
/// and `revert` should still finish if a former managed iface has been
/// removed.
pub fn read_snapshot(root_prefix: Option<&Path>, iface: &str) -> InterfaceSnapshot {
    let mut snap = InterfaceSnapshot {
        iface: iface.to_string(),
        ..Default::default()
    };
    // Read path: a bad iface produces an empty snapshot rather than an
    // error so revert flows over a vanished interface still complete. We
    // keep the validation cheap and bail before touching the filesystem.
    if validate_iface_name(iface).is_err() {
        return snap;
    }
    for s in SYSCTLS {
        let path = sysctl_path(root_prefix, iface, s.key);
        let v = std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string());
        match s.key {
            "use_tempaddr" => snap.use_tempaddr = v,
            "addr_gen_mode" => snap.addr_gen_mode = v,
            "temp_valid_lft" => snap.temp_valid_lft = v,
            "temp_prefered_lft" => snap.temp_prefered_lft = v,
            _ => {}
        }
    }
    snap
}

/// Write a single sysctl knob via the writable mirror under `/proc/sys`.
/// Used by `apply` and `revert` to push settings live without waiting for
/// `sysctl --system` to pick up the drop-in. Validates `iface` first so a
/// caller-supplied bad name can never traverse outside the per-iface tree.
pub fn write_sysctl(root_prefix: Option<&Path>, iface: &str, key: &str, value: &str) -> Result<()> {
    validate_iface_name(iface)?;
    let path = sysctl_path(root_prefix, iface, key);
    std::fs::write(&path, format!("{value}\n"))
        .with_context(|| format!("writing {} = {}", path.display(), value))
}

/// Defense-in-depth: refuse interface names the kernel itself wouldn't
/// accept. Without this check, a hostile or buggy caller could pass
/// `"../../etc/passwd"` and `sysctl_path` would happily produce
/// `/proc/sys/net/ipv6/conf/../../etc/passwd/use_tempaddr` (issue #147).
///
/// Rules mirror `dev_valid_name()` in the Linux kernel
/// (`net/core/dev.c`): 1..=15 bytes, no slash, no NUL, no whitespace, no
/// `:` or `/`, and the special names `.` / `..` are forbidden.
pub(crate) fn validate_iface_name(iface: &str) -> Result<()> {
    if iface.is_empty() {
        return Err(anyhow!("interface name is empty"));
    }
    // The kernel ifname buffer is `IFNAMSIZ - 1 = 15` bytes (the trailing
    // NUL is counted). Any longer name would be rejected by `SIOCSIFNAME`
    // and ENAMETOOLONG-equivalents — refusing it here is a courtesy.
    if iface.len() > 15 {
        return Err(anyhow!(
            "interface name '{iface}' is {} bytes (max 15)",
            iface.len()
        ));
    }
    if iface == "." || iface == ".." {
        return Err(anyhow!("interface name '{iface}' is reserved"));
    }
    for b in iface.bytes() {
        if b == 0 || b == b'/' || b == b':' || b.is_ascii_whitespace() {
            return Err(anyhow!(
                "interface name '{iface}' contains illegal byte 0x{b:02x}"
            ));
        }
        if !b.is_ascii() {
            return Err(anyhow!(
                "interface name '{iface}' contains non-ASCII byte 0x{b:02x}"
            ));
        }
    }
    Ok(())
}

/// Reload kernel sysctls so the drop-in we just wrote takes effect on
/// subsequent interfaces (or after a netns join). Best-effort: if the
/// command isn't on PATH, we've already pushed the live values via
/// `write_sysctl`, so the absence is a warning, not a failure.
pub fn reload_sysctls() -> Result<()> {
    use std::process::Command;
    let status = Command::new("sysctl")
        .arg("--system")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(anyhow!("sysctl --system exited with {s}")),
        Err(e) => Err(anyhow!("running sysctl --system: {e}")),
    }
}

fn sysctl_path(root_prefix: Option<&Path>, iface: &str, key: &str) -> PathBuf {
    let base = root_prefix
        .map(|p| p.join("proc/sys/net/ipv6/conf"))
        .unwrap_or_else(|| PathBuf::from(PROC_BASE));
    base.join(iface).join(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropin_includes_header_and_per_iface_sections() {
        let body = render_dropin(&["wlan0", "eth0"]);
        assert!(body.starts_with("# managed by proteus v"));
        assert!(body.contains("# sha256: "));
        assert!(body.contains("# wlan0\n"));
        assert!(body.contains("# eth0\n"));
        assert!(body.contains("net.ipv6.conf.wlan0.use_tempaddr = 2"));
        assert!(body.contains("net.ipv6.conf.eth0.addr_gen_mode = 3"));
        assert!(body.contains("net.ipv6.conf.wlan0.temp_valid_lft = 86400"));
        assert!(body.contains("net.ipv6.conf.eth0.temp_prefered_lft = 7200"));
    }

    #[test]
    fn dropin_for_no_ifaces_renders_header_only() {
        let body = render_dropin(&[]);
        assert!(body.starts_with("# managed by proteus v"));
        // No iface body → no `net.ipv6.conf` lines.
        assert!(!body.contains("net.ipv6.conf"));
    }

    #[test]
    fn snapshot_reads_from_root_prefix() {
        // Build a fake /proc tree under tmpdir and confirm read_snapshot picks
        // it up — drives the same code path the real /proc read uses.
        let tmp = std::env::temp_dir().join(format!("proteus-ipv6-test-{}", std::process::id()));
        let dir = tmp.join("proc/sys/net/ipv6/conf/wlan0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("use_tempaddr"), "0\n").unwrap();
        std::fs::write(dir.join("addr_gen_mode"), "1\n").unwrap();
        let snap = read_snapshot(Some(&tmp), "wlan0");
        assert_eq!(snap.iface, "wlan0");
        assert_eq!(snap.use_tempaddr.as_deref(), Some("0"));
        assert_eq!(snap.addr_gen_mode.as_deref(), Some("1"));
        assert_eq!(snap.temp_valid_lft, None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn validate_iface_name_accepts_typical_kernel_names() {
        for name in ["eth0", "wlan0", "wlo1", "enp0s3", "enp48s0", "lo"] {
            validate_iface_name(name).unwrap_or_else(|e| panic!("rejected {name}: {e}"));
        }
        // Max length 15 is OK.
        validate_iface_name(&"x".repeat(15)).unwrap();
    }

    #[test]
    fn validate_iface_name_rejects_path_traversal_and_garbage() {
        // Issue #147 — defense in depth against caller-supplied paths.
        for bad in [
            "",
            "../../etc/passwd",
            "../etc",
            "wlan0/../etc",
            ".",
            "..",
            "wlan 0",
            "wlan\t0",
            "wlan\n0",
            "wlan:0",
            "café",
            // Length 16 is over the kernel ifname budget.
            "x".repeat(16).as_str(),
        ] {
            assert!(validate_iface_name(bad).is_err(), "should reject '{bad}'");
        }
    }

    #[test]
    fn write_sysctl_refuses_traversal_iface() {
        // The path must not be created for a traversal iface — even if
        // someone removed `validate_iface_name` from `sysctl_path` itself.
        let tmp =
            std::env::temp_dir().join(format!("proteus-ipv6-validate-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let res = write_sysctl(Some(&tmp), "../../etc/shadow", "use_tempaddr", "2");
        assert!(res.is_err(), "expected validation error");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn snapshot_lookup_dispatches_by_key() {
        let snap = InterfaceSnapshot {
            iface: "x".into(),
            use_tempaddr: Some("2".into()),
            addr_gen_mode: Some("3".into()),
            temp_valid_lft: Some("86400".into()),
            temp_prefered_lft: None,
        };
        assert_eq!(snap.lookup("use_tempaddr"), Some("2"));
        assert_eq!(snap.lookup("addr_gen_mode"), Some("3"));
        assert_eq!(snap.lookup("temp_valid_lft"), Some("86400"));
        assert_eq!(snap.lookup("temp_prefered_lft"), None);
        assert_eq!(snap.lookup("nope"), None);
    }
}
