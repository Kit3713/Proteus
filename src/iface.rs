// SPDX-License-Identifier: GPL-3.0-or-later

//! Single-source-of-truth interface-name validator.
//!
//! Issue GH#359 — historically the codebase grew five distinct
//! interface-name validators (`src/ipv6/mod.rs::validate_iface_name`,
//! `src/mac/factory.rs::is_valid_iface_name`, `src/rf/mod.rs::is_safe_iface`,
//! `src/kill_switch/mod.rs::is_safe_iface`, plus assorted private inline
//! shapes). Each enforced a slightly different rule set: some refused
//! leading `-`, some didn't; some capped at 15 bytes, some at 16; some
//! allowed `_`, some didn't. That fragmentation invited drift the next
//! time a new call site appeared.
//!
//! This module is the canonical helper. As of the GH#359 follow-up wave
//! every per-module validator in the tree is a thin wrapper that
//! delegates here — they keep their existing function signature so the
//! local call sites still read naturally, but the rule set lives in one
//! place. New code should prefer [`validate`] (typed reason) or
//! [`is_valid`] (bool) directly.
//!
//! ## Rule set
//!
//! Mirrors the kernel's `dev_valid_name()` plus the audit recommendations
//! from L-3 (no leading `-` so `iw`/`ip`/`ethtool` cannot reparse the
//! iface as a flag) and N-1 (ASCII-only, cap at IFNAMSIZ-1):
//!
//! - non-empty, `<= 15` bytes (`IFNAMSIZ - 1` excluding the trailing NUL).
//! - bytes restricted to `[A-Za-z0-9_.-]` — the punctuation set real
//!   iface names use (`enp48s0`, `wlp3s0f3u2`, `eth0.10`, `enx00e04c360033`).
//! - no leading `-`.
//! - the special names `.` and `..` are forbidden.

use std::fmt;

/// Reasons [`validate`] can refuse an iface name. Surfaced via Display so
/// callers can attach the reason to a wiki-linked error without parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidIface {
    Empty,
    TooLong,
    Reserved,
    LeadingDash,
    IllegalByte,
}

impl fmt::Display for InvalidIface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "interface name is empty"),
            Self::TooLong => write!(f, "interface name exceeds 15 bytes (kernel IFNAMSIZ-1)"),
            Self::Reserved => write!(f, "interface name is reserved (`.` / `..`)"),
            Self::LeadingDash => {
                write!(
                    f,
                    "interface name starts with `-` (would be parsed as a flag)"
                )
            }
            Self::IllegalByte => write!(f, "interface name contains a byte outside [A-Za-z0-9_.-]"),
        }
    }
}

impl std::error::Error for InvalidIface {}

/// Strict iface-name validator. Returns `Ok(())` for any name the kernel
/// would accept under `dev_valid_name()` and that is *also* safe to pass
/// to `iw` / `ip` / `ethtool` as a positional argument.
pub fn validate(iface: &str) -> Result<(), InvalidIface> {
    if iface.is_empty() {
        return Err(InvalidIface::Empty);
    }
    if iface.len() > 15 {
        return Err(InvalidIface::TooLong);
    }
    if iface == "." || iface == ".." {
        return Err(InvalidIface::Reserved);
    }
    if iface.starts_with('-') {
        return Err(InvalidIface::LeadingDash);
    }
    if !iface
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
    {
        return Err(InvalidIface::IllegalByte);
    }
    Ok(())
}

/// Convenience predicate. Equivalent to `validate(iface).is_ok()`. Most
/// call sites prefer [`validate`] because it carries the rejection
/// reason; this is for boolean contexts (e.g. `if !is_valid(iface) { ... }`).
pub fn is_valid(iface: &str) -> bool {
    validate(iface).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_kernel_names() {
        for ok in [
            "eth0",
            "wlan0",
            "lo",
            "enp48s0",
            "wlp3s0f3u2",
            "eth0.10",
            "enx00e04c360033",
            "br0",
            "tun0",
            "tap0",
            "wg0",
            "wlan_dev_0",
            // Max length 15 (IFNAMSIZ - 1).
            "abcdefghijklmno",
        ] {
            assert!(
                validate(ok).is_ok(),
                "{ok:?} should validate, got {:?}",
                validate(ok)
            );
        }
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(validate(""), Err(InvalidIface::Empty));
    }

    #[test]
    fn rejects_too_long() {
        // 16 bytes is one past the IFNAMSIZ-1 ceiling.
        assert_eq!(validate("abcdefghijklmnop"), Err(InvalidIface::TooLong));
    }

    #[test]
    fn rejects_reserved_dot_names() {
        assert_eq!(validate("."), Err(InvalidIface::Reserved));
        assert_eq!(validate(".."), Err(InvalidIface::Reserved));
    }

    #[test]
    fn rejects_leading_dash() {
        assert_eq!(validate("-attacker"), Err(InvalidIface::LeadingDash));
        assert_eq!(validate("--help"), Err(InvalidIface::LeadingDash));
        assert_eq!(validate("-Vroot:1"), Err(InvalidIface::LeadingDash));
    }

    #[test]
    fn rejects_illegal_bytes() {
        for bad in [
            "with/slash",
            "with space",
            "with\nnewline",
            "with\0nul",
            "iface;rm",
            "iface$evil",
            "iface\"quote",
            "wlan:0",
            // Non-ASCII.
            "wlan\u{00ff}",
            "café",
        ] {
            assert_eq!(
                validate(bad),
                Err(InvalidIface::IllegalByte),
                "{bad:?} should fail with IllegalByte"
            );
        }
    }

    #[test]
    fn is_valid_matches_validate() {
        for s in ["wlan0", "", ".", "-x", "with/slash", "abcdefghijklmno"] {
            assert_eq!(is_valid(s), validate(s).is_ok());
        }
    }
}
