// SPDX-License-Identifier: GPL-3.0-or-later

//! Enterprise Wi-Fi (802.1X) anonymous outer identity. Opt-in, default off.
//!
//! Replaces `802-1x.anonymous-identity` on a NetworkManager connection with
//! `anonymous@<realm>`, where the realm is either extracted from the inner
//! `802-1x.identity` (the `@<realm>` suffix) or supplied verbatim by the
//! operator. The inner identity itself, the EAP method, the certificate
//! configuration, and the password/cert blobs are never touched.
//!
//! See `proteus wiki enterprise-wifi` for the threat model, why this is
//! opt-in, and the failure modes (some Microsoft NPS / Cisco ISE deployments
//! reject mismatched outer identities).
//!
//! The whole module is pure-Rust + DBus; no shelling out, no nmcli.
//!
//! # Behavior
//!
//! * `enable` reads `802-1x.identity`, derives the realm, writes
//!   `anonymous@<realm>` into `802-1x.anonymous-identity`. The pre-Proteus
//!   value is cached in `state.json` so `disable` and `revert` can put the
//!   profile back the way they found it.
//! * `disable` clears `802-1x.anonymous-identity` (sets it to the empty
//!   string, which NM treats as unset on save).
//! * `status` lists every 802.1X-flavored connection NM knows about.

pub mod nm;

use anyhow::{Result, anyhow};

/// Build the anonymous outer identity string for a realm. Wrapping the realm
/// formatter centralises the `anonymous@` literal so a future tweak (e.g.
/// configurable local-part) only touches one site.
pub fn anonymous_identity_for(realm: &str) -> String {
    format!("anonymous@{realm}")
}

/// Extract the realm (the `@`-suffix) from an 802.1X inner identity.
///
/// Returns `Err` if `identity` has no `@` separator, the local-part is empty,
/// or the realm itself is empty. We intentionally accept everything else
/// verbatim — RFC 4282 NAIs allow a wide set of characters and the supplicant
/// already validated it on save, so re-parsing here would just be theatre.
pub fn extract_realm(identity: &str) -> Result<&str> {
    let trimmed = identity.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "802-1x.identity is empty; see proteus wiki enterprise-wifi"
        ));
    }
    let mut parts = trimmed.rsplitn(2, '@');
    let realm = parts.next().unwrap_or("");
    let local = parts.next().ok_or_else(|| {
        anyhow!("identity '{trimmed}' has no '@' realm separator; see proteus wiki enterprise-wifi")
    })?;
    if local.is_empty() {
        return Err(anyhow!(
            "identity '{trimmed}' has no local-part before '@'; see proteus wiki enterprise-wifi"
        ));
    }
    if realm.is_empty() {
        return Err(anyhow!(
            "identity '{trimmed}' has no realm after '@' (cannot derive anonymous identity); see proteus wiki enterprise-wifi"
        ));
    }
    Ok(realm)
}

/// Resolve the realm to use for a given connection. `manual` strategy returns
/// the configured realm verbatim (after a non-empty check); `auto` extracts
/// it from the supplied inner identity. Any other strategy string is rejected
/// up front so a typo in `config.toml` can't cause a silent fallback.
pub fn resolve_realm<'a>(
    strategy: &str,
    configured_realm: &'a str,
    inner_identity: Option<&'a str>,
) -> Result<&'a str> {
    match strategy {
        "manual" => {
            let trimmed = configured_realm.trim();
            if trimmed.is_empty() {
                Err(anyhow!(
                    "enterprise_wifi.realm_strip_strategy = 'manual' but enterprise_wifi.anonymous_realm is empty; see proteus wiki enterprise-wifi"
                ))
            } else {
                Ok(trimmed)
            }
        }
        "auto" => {
            let inner = inner_identity.ok_or_else(|| {
                anyhow!(
                    "connection has no 802-1x.identity to derive a realm from; see proteus wiki enterprise-wifi"
                )
            })?;
            extract_realm(inner)
        }
        other => Err(anyhow!(
            "unknown realm_strip_strategy '{other}'; expected 'auto' or 'manual'; see proteus wiki enterprise-wifi"
        )),
    }
}

/// Redact the local-part of an inner identity for status output. Keeps the
/// realm intact (the wiki is explicit that the realm stays public on the
/// wire anyway) and replaces the local-part with `***`.
///
/// Identities without an `@` are surfaced as `***` outright — nothing about
/// them is safe to print.
pub fn redact_identity(identity: &str) -> String {
    match identity.rsplit_once('@') {
        Some((local, realm)) if !local.is_empty() && !realm.is_empty() => {
            format!("***@{realm}")
        }
        _ => "***".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_realm_pulls_suffix_after_at() {
        assert_eq!(extract_realm("alice@example.edu").unwrap(), "example.edu");
        assert_eq!(extract_realm("j.smith@uni.edu").unwrap(), "uni.edu");
        assert_eq!(
            extract_realm("user@sub.realm.example").unwrap(),
            "sub.realm.example"
        );
    }

    #[test]
    fn extract_realm_uses_last_at_for_addresses_with_multiple() {
        // RFC 4282 NAIs may legally contain multiple `@`; the realm is by
        // definition the right-most label.
        assert_eq!(
            extract_realm("weird@user@example.com").unwrap(),
            "example.com"
        );
    }

    #[test]
    fn extract_realm_rejects_missing_or_empty_parts() {
        assert!(extract_realm("").is_err());
        assert!(extract_realm("just-a-name").is_err());
        assert!(extract_realm("@example.com").is_err());
        assert!(extract_realm("alice@").is_err());
        assert!(extract_realm("   ").is_err());
    }

    #[test]
    fn extract_realm_trims_surrounding_whitespace() {
        assert_eq!(extract_realm("  bob@x.y  ").unwrap(), "x.y");
    }

    #[test]
    fn anonymous_identity_format_is_anonymous_at_realm() {
        assert_eq!(
            anonymous_identity_for("university.edu"),
            "anonymous@university.edu"
        );
        assert_eq!(
            anonymous_identity_for("eduroam.org"),
            "anonymous@eduroam.org"
        );
    }

    #[test]
    fn resolve_realm_auto_uses_inner_identity() {
        let r = resolve_realm("auto", "ignored.example", Some("user@actual.example")).unwrap();
        assert_eq!(r, "actual.example");
    }

    #[test]
    fn resolve_realm_auto_fails_without_inner_identity() {
        assert!(resolve_realm("auto", "", None).is_err());
    }

    #[test]
    fn resolve_realm_manual_uses_configured_realm() {
        let r = resolve_realm("manual", "example.com", Some("ignored@whatever")).unwrap();
        assert_eq!(r, "example.com");
    }

    #[test]
    fn resolve_realm_manual_rejects_empty_configured_realm() {
        assert!(resolve_realm("manual", "", Some("alice@x.y")).is_err());
        assert!(resolve_realm("manual", "   ", Some("alice@x.y")).is_err());
    }

    #[test]
    fn resolve_realm_unknown_strategy_is_rejected() {
        assert!(resolve_realm("automatic", "", Some("a@b")).is_err());
        assert!(resolve_realm("", "x", Some("a@b")).is_err());
    }

    #[test]
    fn redact_identity_keeps_realm_only() {
        assert_eq!(redact_identity("alice@example.edu"), "***@example.edu");
        assert_eq!(redact_identity("j.smith@uni.edu"), "***@uni.edu");
    }

    #[test]
    fn redact_identity_handles_malformed_inputs() {
        assert_eq!(redact_identity("no-at"), "***");
        assert_eq!(redact_identity("@only-realm"), "***");
        assert_eq!(redact_identity("only-local@"), "***");
        assert_eq!(redact_identity(""), "***");
    }
}
