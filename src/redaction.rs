// SPDX-License-Identifier: GPL-3.0-or-later

//! Logging-layer redaction for device identifiers (roadmap 1.0.5, L1–L5).
//!
//! Proteus exists to erase fingerprints; writing those same fingerprints
//! (MAC / SSID / hostname / 802.1X identity) into journald or stderr would
//! re-leak exactly what it hides. This module is the single policy-aware
//! choke point every *log* site routes through.
//!
//! Three forms, selected by `[logging] identifiers` in `config.toml`:
//!
//! - `off` — emit `"<hidden>"`; nothing identifier-shaped reaches the log.
//! - `redacted` (default) — emit a stable but non-reversible form: the MAC
//!   keeps its OUI (first three octets, public on the wire anyway) and
//!   masks the NIC-specific tail plus an 8-hex correlation tag; SSID /
//!   hostname collapse to an 8-hex tag; 802.1X identity keeps only its
//!   realm (`***@realm`). Two log lines about the same value share a tag
//!   so an operator can correlate without learning the value.
//! - `full-view` — emit the real value, terminal-sanitized, behind a
//!   loud one-time startup warning. For short-lived local debugging only.
//!
//! Crucially this is a **logging-layer** concern: `--json` output and CLI
//! display always show the real value. Redaction never touches the wire,
//! `state.json`, or `config.toml`; it only shapes the rendered-for-log
//! form. "Encrypt before logging" is explicitly rejected — a log that can
//! be decrypted is a log that still holds the fingerprint.

use std::sync::OnceLock;

/// How device identifiers are rendered at log sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdentifierPolicy {
    /// Replace every identifier with `"<hidden>"`.
    Off,
    /// OUI + hash / `***@realm` forms. The safe default.
    #[default]
    Redacted,
    /// Real values (terminal-sanitized) behind a one-time warning.
    FullView,
}

impl IdentifierPolicy {
    /// Parse the on-disk `[logging] identifiers` string. Accepts exactly
    /// `"off"`, `"redacted"`, and `"full-view"`; anything else is `None`
    /// so the config validator can bail with a helpful message.
    pub fn parse(s: &str) -> Option<IdentifierPolicy> {
        match s {
            "off" => Some(IdentifierPolicy::Off),
            "redacted" => Some(IdentifierPolicy::Redacted),
            "full-view" => Some(IdentifierPolicy::FullView),
            _ => None,
        }
    }
}

/// Module-level alias for [`IdentifierPolicy::parse`], for call sites that
/// read more naturally as `redaction::parse(s)`.
pub fn parse(s: &str) -> Option<IdentifierPolicy> {
    IdentifierPolicy::parse(s)
}

/// Process-global policy. Set once at startup from the resolved config
/// (see `Config::default_or_loaded`). First writer wins; reads before any
/// write fall back to the safe `Redacted` default.
static POLICY: OnceLock<IdentifierPolicy> = OnceLock::new();

/// Install the active policy. First-writer-wins — later calls are ignored
/// so a stray re-init can never *weaken* an already-installed policy.
///
/// Emits the loud one-time `full-view` warning here (and only here, only
/// when `full-view` is the value that actually wins) so the warning fires
/// exactly once per process, at the moment the weakening takes effect.
pub fn set_policy(policy: IdentifierPolicy) {
    let mut newly_set = false;
    let _ = POLICY.get_or_init(|| {
        newly_set = true;
        policy
    });
    if newly_set && policy == IdentifierPolicy::FullView {
        tracing::warn!(
            "logging.identifiers = full-view: device identifiers (MAC/SSID/hostname/802.1X) \
             will appear UNREDACTED in logs. Use only for short-lived local debugging; \
             do not ship these logs."
        );
    }
}

/// The active policy, or the safe `Redacted` default if none was installed.
pub fn policy() -> IdentifierPolicy {
    *POLICY.get().unwrap_or(&IdentifierPolicy::Redacted)
}

/// 8-char correlation tag for `bytes`. A truncated SHA-256 hex digest:
/// stable for a given input, not reversible, zero new dependencies.
fn tag(bytes: &[u8]) -> String {
    crate::crypto::sha256::hex_digest(bytes)[..8].to_string()
}

// ---- per-type forms (policy threaded explicitly for testability) --------

fn mac_with(policy: IdentifierPolicy, value: &crate::mac::Mac) -> String {
    match policy {
        IdentifierPolicy::Off => "<hidden>".to_string(),
        IdentifierPolicy::Redacted => {
            let o = value.octets();
            format!(
                "{:02x}:{:02x}:{:02x}:**:**:** h:{}",
                o[0],
                o[1],
                o[2],
                tag(&o)
            )
        }
        // Display emits the real `aa:bb:cc:dd:ee:ff`; a MAC has no control
        // chars to sanitize, so the real value is safe to print verbatim.
        IdentifierPolicy::FullView => value.to_string(),
    }
}

fn ssid_with(policy: IdentifierPolicy, value: &str) -> String {
    match policy {
        IdentifierPolicy::Off => "<hidden>".to_string(),
        IdentifierPolicy::Redacted => format!("h:{}", tag(value.as_bytes())),
        // SSIDs are attacker-controlled; never echo raw bytes to a
        // terminal-backed log even under full-view.
        IdentifierPolicy::FullView => crate::per_ssid::display_ssid(value),
    }
}

fn hostname_with(policy: IdentifierPolicy, value: &str) -> String {
    match policy {
        IdentifierPolicy::Off => "<hidden>".to_string(),
        IdentifierPolicy::Redacted => format!("h:{}", tag(value.as_bytes())),
        IdentifierPolicy::FullView => crate::display::display_safe(value).into_owned(),
    }
}

fn identity_with(policy: IdentifierPolicy, value: &str) -> String {
    match policy {
        IdentifierPolicy::Off => "<hidden>".to_string(),
        // `redact_identity` already collapses the local-part to `***`.
        IdentifierPolicy::Redacted => crate::enterprise_wifi::redact_identity(value),
        IdentifierPolicy::FullView => crate::display::display_safe(value).into_owned(),
    }
}

// ---- public wrappers (use the installed global policy) -------------------

/// Render a MAC for a log site under the active policy.
pub fn mac(value: &crate::mac::Mac) -> String {
    mac_with(policy(), value)
}

/// Render an SSID for a log site under the active policy.
pub fn ssid(value: &str) -> String {
    ssid_with(policy(), value)
}

/// Render a hostname for a log site under the active policy.
pub fn hostname(value: &str) -> String {
    hostname_with(policy(), value)
}

/// Render an 802.1X identity for a log site under the active policy.
pub fn identity(value: &str) -> String {
    identity_with(policy(), value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mac::Mac;

    const MAC: Mac = Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);

    #[test]
    fn parse_accepts_the_three_forms() {
        assert_eq!(parse("off"), Some(IdentifierPolicy::Off));
        assert_eq!(parse("redacted"), Some(IdentifierPolicy::Redacted));
        assert_eq!(parse("full-view"), Some(IdentifierPolicy::FullView));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse("redact"), None);
        assert_eq!(parse("full_view"), None);
        assert_eq!(parse("FULL-VIEW"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("on"), None);
    }

    #[test]
    fn default_policy_is_redacted() {
        assert_eq!(IdentifierPolicy::default(), IdentifierPolicy::Redacted);
    }

    #[test]
    fn redacted_mac_keeps_oui_masks_nic_and_hides_tail() {
        let out = mac_with(IdentifierPolicy::Redacted, &MAC);
        assert!(out.contains("aa:bb:cc"), "OUI must survive: {out}");
        assert!(out.contains("**:**:**"), "NIC octets must be masked: {out}");
        // The masked tail must never leak the real dd/ee/ff octets.
        assert!(!out.contains("dd:ee:ff"), "real NIC tail leaked: {out}");
        assert!(out.contains("h:"), "correlation tag must be present: {out}");
    }

    #[test]
    fn redacted_mac_tag_is_stable_and_eight_hex() {
        let a = mac_with(IdentifierPolicy::Redacted, &MAC);
        let b = mac_with(IdentifierPolicy::Redacted, &MAC);
        assert_eq!(a, b, "same MAC must redact to the same string");
        let h = a.rsplit("h:").next().unwrap();
        assert_eq!(h.len(), 8, "tag must be 8 hex chars: {a}");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn redacted_ssid_and_hostname_differ_from_input_with_no_substring() {
        let ssid_in = "MyHomeNetwork";
        let s = ssid_with(IdentifierPolicy::Redacted, ssid_in);
        assert_ne!(s, ssid_in);
        assert!(!s.contains(ssid_in), "redacted ssid leaks input: {s}");
        assert!(s.starts_with("h:"));

        let host_in = "alices-laptop";
        let h = hostname_with(IdentifierPolicy::Redacted, host_in);
        assert_ne!(h, host_in);
        assert!(!h.contains(host_in), "redacted hostname leaks input: {h}");
        assert!(h.starts_with("h:"));
    }

    #[test]
    fn off_hides_every_type() {
        assert_eq!(mac_with(IdentifierPolicy::Off, &MAC), "<hidden>");
        assert_eq!(ssid_with(IdentifierPolicy::Off, "x"), "<hidden>");
        assert_eq!(hostname_with(IdentifierPolicy::Off, "x"), "<hidden>");
        assert_eq!(identity_with(IdentifierPolicy::Off, "a@b"), "<hidden>");
    }

    #[test]
    fn full_view_reveals_real_mac() {
        assert_eq!(
            mac_with(IdentifierPolicy::FullView, &MAC),
            "aa:bb:cc:dd:ee:ff"
        );
    }

    #[test]
    fn full_view_ssid_is_terminal_sanitized() {
        // A hostile SSID carrying a clear-screen escape must be neutralized
        // even under full-view — never hand raw control bytes to journald.
        let out = ssid_with(IdentifierPolicy::FullView, "\x1b[2Jpwned");
        assert!(!out.contains('\x1b'), "raw ESC must be escaped: {out:?}");
        assert!(out.contains("pwned"));
    }

    #[test]
    fn full_view_hostname_is_terminal_sanitized() {
        let out = hostname_with(IdentifierPolicy::FullView, "\x1b[2Jhost");
        assert!(!out.contains('\x1b'), "raw ESC must be escaped: {out:?}");
        assert!(out.contains("host"));
    }

    #[test]
    fn redacted_identity_keeps_realm_only() {
        assert_eq!(
            identity_with(IdentifierPolicy::Redacted, "alice@example.edu"),
            "***@example.edu"
        );
    }
}
