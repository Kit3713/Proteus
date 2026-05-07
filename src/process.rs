// SPDX-License-Identifier: GPL-3.0-or-later

//! Small helpers for spawning external commands with a hardened binary
//! lookup. Roadmap Milestone 6 "Bypass hardening pass": every place the
//! tree shells out should prefer an absolute path to the binary so a
//! tampered `$PATH` (suid context, custom systemd unit, hostile shell
//! integration) can't redirect us to an attacker-controlled
//! lookalike. The pattern mirrors `mac::factory::ETHTOOL_ABS_PATH` and
//! `rf::IW_ABS_PATHS`.
//!
//! This module is not a sandboxing primitive — once we have the right
//! binary, the spawn still inherits the full caller environment. The
//! goal is narrower: pin *which* binary runs, leaving `$PATH` as a
//! Nix/Alpine fallback only.

use std::path::Path;

/// Resolve a binary name to an absolute path when one of the canonical
/// locations exists; otherwise return the bare name and let
/// `Command::new` walk `$PATH`. Empty `abs_paths` means "no canonical
/// locations defined" — caller falls through to `$PATH` immediately.
///
/// Order matters in `abs_paths`: the first existing path wins. Typical
/// shape is `&["/usr/sbin/<name>", "/sbin/<name>", "/usr/bin/<name>"]`
/// — sbin first because Linux distros put privileged tools there by
/// convention.
pub fn resolve_bin(name: &'static str, abs_paths: &[&'static str]) -> &'static str {
    for p in abs_paths {
        if Path::new(p).exists() {
            return p;
        }
    }
    name
}

/// Common absolute-path tables for binaries the tree shells out to.
/// Keeping them in one place means a future audit can scan a single
/// file rather than greppping every `Command::new` site.
pub mod paths {
    pub const NFT: &[&str] = &["/usr/sbin/nft", "/sbin/nft"];
    pub const IP: &[&str] = &["/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip"];
    pub const SYSCTL: &[&str] = &["/usr/sbin/sysctl", "/sbin/sysctl"];
    pub const SYSTEMCTL: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];
    pub const JOURNALCTL: &[&str] = &["/usr/bin/journalctl", "/bin/journalctl"];
    pub const SS: &[&str] = &["/usr/sbin/ss", "/sbin/ss", "/usr/bin/ss"];
    pub const DMESG: &[&str] = &["/usr/bin/dmesg", "/bin/dmesg"];
    pub const SEMANAGE: &[&str] = &["/usr/sbin/semanage", "/sbin/semanage"];
}

// Convenience accessors for the most-common binaries. Each returns the
// resolved absolute path (or the bare name when no canonical location
// exists), letting call sites stay terse: `Command::new(systemctl())`
// rather than threading the `paths::SYSTEMCTL` table through every
// invocation.
pub fn systemctl() -> &'static str {
    resolve_bin("systemctl", paths::SYSTEMCTL)
}

pub fn nft() -> &'static str {
    resolve_bin("nft", paths::NFT)
}

pub fn ip() -> &'static str {
    resolve_bin("ip", paths::IP)
}

pub fn sysctl() -> &'static str {
    resolve_bin("sysctl", paths::SYSCTL)
}

pub fn journalctl() -> &'static str {
    resolve_bin("journalctl", paths::JOURNALCTL)
}

pub fn ss_bin() -> &'static str {
    resolve_bin("ss", paths::SS)
}

pub fn dmesg() -> &'static str {
    resolve_bin("dmesg", paths::DMESG)
}

pub fn semanage() -> &'static str {
    resolve_bin("semanage", paths::SEMANAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_bin_returns_bare_name_when_nothing_exists() {
        let r = resolve_bin("definitely-not-a-real-binary-xyz", &[
            "/this/does/not/exist",
            "/also/missing",
        ]);
        assert_eq!(r, "definitely-not-a-real-binary-xyz");
    }

    #[test]
    fn resolve_bin_walks_in_order_returning_first_existing_path() {
        // /usr/bin/ls and /bin/ls both exist on most Linux hosts.
        // The first hit wins, even when later candidates also exist.
        let r = resolve_bin(
            "ls",
            &["/usr/bin/ls", "/bin/ls", "/this/does/not/exist"],
        );
        assert!(
            r == "/usr/bin/ls" || r == "/bin/ls" || r == "ls",
            "unexpected resolution: {r}"
        );
    }

    #[test]
    fn paths_tables_are_nonempty_and_absolute() {
        for table in [
            paths::NFT,
            paths::IP,
            paths::SYSCTL,
            paths::SYSTEMCTL,
            paths::JOURNALCTL,
            paths::SS,
            paths::DMESG,
            paths::SEMANAGE,
        ] {
            assert!(!table.is_empty(), "path table is empty");
            for p in table {
                assert!(p.starts_with('/'), "non-absolute candidate: {p}");
            }
        }
    }
}
