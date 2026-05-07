// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-SSID profile policy resolution (roadmap Milestone 3).
//!
//! A user can carve out per-SSID overrides under `[per_ssid."<ssid>"]` in
//! `config.toml`. At NM `connection-up` time the orchestrator asks
//! `resolve_for_ssid` what the *effective* policy is for the joined SSID;
//! the resolver walks four layers in decreasing precedence and stops as
//! soon as it finds an answer for each field:
//!
//!   1. `[per_ssid."<ssid>"]` — the SSID-specific block (highest)
//!   2. `[persona]` — the active persona's defaults (when set)
//!   3. `[profile]` baseline — the slider's documented behaviour
//!   4. `Config` defaults — the structural fallback
//!
//! The `EffectivePolicy::source` vector records which layer contributed
//! each field so `proteus ssid show <ssid>` can print the same trace the
//! orchestrator saw.
//!
//! Integration with the NM connection-up dispatcher and the rotate timer
//! is the follow-up tracked in roadmap Milestone 3 — this module ships
//! the schema, the resolver, and the surfaced CLI; consumers come next.

use std::time::Duration;

use crate::config::{Config, PerSsidPolicy};
use crate::profile::Profile;

/// Resolved policy for one SSID. Every field is concrete (or `None` when
/// the layer-walk found no matching override and there is no profile-level
/// default — the orchestrator treats `None` as "fall through to the
/// existing global behaviour").
#[derive(Debug, Clone, PartialEq)]
pub struct EffectivePolicy {
    /// Persona to shape this connection with. `None` means "no persona;
    /// stay in plain randomizer mode".
    pub persona: Option<String>,
    /// `Profile` slider in effect on this SSID. Always concrete because
    /// either the per-SSID override or the global config supplies one.
    pub profile: Profile,
    /// Pinned MAC for this SSID, if the operator wired one. The
    /// orchestrator pre-empts MAC rotation while a pin is in scope.
    pub pin_mac: Option<String>,
    /// Rotation cadence override parsed into a `Duration`. `None` means
    /// "use the global `[timers.rotate].interval`" — the resolver does
    /// not synthesise a cadence from the profile baseline because the
    /// global timer already covers that case.
    pub rotate_interval: Option<Duration>,
    /// Portal-policy override (e.g. `"fresh-mac-per-visit"`). Pass-through
    /// string; the orchestrator interprets it.
    pub portal_policy: Option<String>,
    /// Source trace, ordered from most-specific to least: each entry is
    /// the layer that contributed at least one field to the resolved
    /// policy. Always non-empty (the `defaults` floor is always present).
    pub source: Vec<&'static str>,
}

/// Layer names used in `EffectivePolicy::source`. Stable strings so
/// callers and tests can match on them without typo risk.
pub const LAYER_PER_SSID: &str = "per_ssid";
pub const LAYER_PERSONA: &str = "persona";
pub const LAYER_PROFILE: &str = "profile";
pub const LAYER_DEFAULTS: &str = "defaults";

/// Walk the four layers and produce the effective policy for `ssid`. The
/// resolver is total: it always returns a value, even when no per-SSID
/// block exists (the result then collapses to the global config). See the
/// module-level docs for the precedence rule.
pub fn resolve_for_ssid(config: &Config, ssid: &str) -> EffectivePolicy {
    let per = config.per_ssid.get(ssid);
    let mut source: Vec<&'static str> = Vec::new();

    // Per-SSID layer: the operator's most specific say. We record the
    // layer in `source` only when at least one field was set, so
    // `source` is a precise trace of what actually contributed.
    let per_persona = per.and_then(|p| p.persona.clone());
    let per_profile = per.and_then(|p| p.aggressiveness_profile.as_deref().and_then(Profile::parse));
    let per_pin = per.and_then(|p| p.pin_mac.clone());
    let per_rotate = per.and_then(|p| p.rotate_interval.as_deref().and_then(parse_duration));
    let per_portal = per.and_then(|p| p.portal_policy.clone());
    let per_contributed = per.is_some_and(per_block_has_any_field);
    if per_contributed {
        source.push(LAYER_PER_SSID);
    }

    // Persona layer: today only the persona id falls out of `[persona]
    // active`. Future fields (e.g. persona-defined rotate cadence) can
    // fold in here without touching call sites. Compute "did persona
    // contribute" before the `or_else` move consumes `per_persona`.
    let persona_contributed = per_persona.is_none() && config.persona.active.is_some();
    let persona = per_persona.or_else(|| config.persona.active.clone());
    if persona_contributed {
        source.push(LAYER_PERSONA);
    }

    // Profile layer: the slider always supplies a concrete `Profile`.
    // Per-SSID can lift / lower it; otherwise the global profile wins.
    let profile = per_profile.unwrap_or(config.profile);
    let profile_contributed = per_profile.is_none();
    if profile_contributed {
        source.push(LAYER_PROFILE);
    }

    // Defaults floor: structural fallback. Always recorded so `source`
    // is never empty (the resolver is total).
    source.push(LAYER_DEFAULTS);

    EffectivePolicy {
        persona,
        profile,
        pin_mac: per_pin,
        rotate_interval: per_rotate,
        portal_policy: per_portal,
        source,
    }
}

fn per_block_has_any_field(p: &PerSsidPolicy) -> bool {
    p.persona.is_some()
        || p.aggressiveness_profile.is_some()
        || p.pin_mac.is_some()
        || p.rotate_interval.is_some()
        || p.portal_policy.is_some()
}

/// Parse a compact duration string (`30s`, `5m`, `2h`, `1d`) into
/// `Duration`. Mirrors the subset of `proteus timer set --interval`
/// grammar that makes sense for an SSID-scoped knob: bare seconds /
/// minutes / hours / days. Returns `None` on anything off-format so the
/// resolver can transparently fall through to the global timer.
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: u64 = num.parse().ok()?;
    match unit {
        "s" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_secs(n * 60)),
        "h" => Some(Duration::from_secs(n * 3600)),
        "d" => Some(Duration::from_secs(n * 86_400)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn cfg_with_profile(p: Profile) -> Config {
        let mut cfg = p.baseline();
        cfg.per_ssid = BTreeMap::new();
        cfg
    }

    /// No per-SSID block at all: every field falls through to the global
    /// profile / defaults. `source` reflects that no SSID-level layer
    /// contributed.
    #[test]
    fn resolve_with_no_per_ssid_entry_reads_through() {
        let cfg = cfg_with_profile(Profile::Med);
        let eff = resolve_for_ssid(&cfg, "anything");
        assert_eq!(eff.profile, Profile::Med);
        assert!(eff.persona.is_none());
        assert!(eff.pin_mac.is_none());
        assert!(eff.rotate_interval.is_none());
        assert!(eff.portal_policy.is_none());
        assert!(!eff.source.contains(&LAYER_PER_SSID));
        assert!(eff.source.contains(&LAYER_PROFILE));
        assert!(eff.source.contains(&LAYER_DEFAULTS));
    }

    /// Per-SSID `aggressiveness_profile` lifts the slider to `Agr` even
    /// though the global profile is `Med`. Source trace shows `per_ssid`
    /// before `defaults`.
    #[test]
    fn per_ssid_profile_override_beats_global_profile() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.per_ssid.insert(
            "conference".into(),
            PerSsidPolicy {
                aggressiveness_profile: Some("agr".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "conference");
        assert_eq!(eff.profile, Profile::Agr);
        assert_eq!(eff.source.first(), Some(&LAYER_PER_SSID));
    }

    /// Persona on the per-SSID block beats the globally-active persona.
    #[test]
    fn per_ssid_persona_beats_global_persona() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.persona.active = Some("randomizer-med".into());
        cfg.per_ssid.insert(
            "coffee".into(),
            PerSsidPolicy {
                persona: Some("iphone-15".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "coffee");
        assert_eq!(eff.persona.as_deref(), Some("iphone-15"));
        assert_eq!(eff.source.first(), Some(&LAYER_PER_SSID));
    }

    /// Without per-SSID, the global persona feeds the resolved policy and
    /// `source` records the persona layer (in addition to profile +
    /// defaults).
    #[test]
    fn global_persona_beats_profile_when_per_ssid_absent() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.persona.active = Some("randomizer-med".into());
        let eff = resolve_for_ssid(&cfg, "any-ssid");
        assert_eq!(eff.persona.as_deref(), Some("randomizer-med"));
        assert!(eff.source.contains(&LAYER_PERSONA));
        assert!(eff.source.contains(&LAYER_PROFILE));
        assert!(!eff.source.contains(&LAYER_PER_SSID));
    }

    /// Profile baseline beats the structural defaults floor: when the
    /// SSID has no entry and no persona is active, `profile` is the
    /// configured slider value, not `Profile::default()`.
    #[test]
    fn profile_baseline_beats_default_when_persona_absent() {
        let cfg = cfg_with_profile(Profile::High);
        let eff = resolve_for_ssid(&cfg, "anywhere");
        assert_eq!(eff.profile, Profile::High);
        assert!(eff.persona.is_none());
        assert_eq!(*eff.source.last().unwrap(), LAYER_DEFAULTS);
    }

    /// Source-trace order is "most specific first": when every layer
    /// contributes, the vector reads `per_ssid, persona, profile,
    /// defaults`. Persona drops out when per-SSID supplies a persona;
    /// `LAYER_PROFILE` drops out when per-SSID supplies a profile.
    #[test]
    fn source_trace_order_with_full_per_ssid_block() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.persona.active = Some("randomizer-med".into());
        cfg.per_ssid.insert(
            "x".into(),
            PerSsidPolicy {
                persona: Some("iphone-15".into()),
                aggressiveness_profile: Some("agr".into()),
                pin_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                rotate_interval: Some("30m".into()),
                portal_policy: Some("fresh-mac-per-visit".into()),
            },
        );
        let eff = resolve_for_ssid(&cfg, "x");
        // per_ssid supplies persona + profile + everything else, so
        // the persona-layer slot is unused. Defaults always last.
        assert_eq!(eff.source.first(), Some(&LAYER_PER_SSID));
        assert_eq!(eff.source.last(), Some(&LAYER_DEFAULTS));
        assert!(!eff.source.contains(&LAYER_PERSONA));
        assert!(!eff.source.contains(&LAYER_PROFILE));
    }

    /// Source-trace ordering when only some per-SSID fields are set.
    /// The persona layer kicks in (per-SSID didn't provide one), and
    /// the profile layer also kicks in (per-SSID didn't override the
    /// slider).
    #[test]
    fn source_trace_with_partial_per_ssid_block() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.persona.active = Some("randomizer-med".into());
        cfg.per_ssid.insert(
            "x".into(),
            PerSsidPolicy {
                pin_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "x");
        assert_eq!(eff.source[0], LAYER_PER_SSID);
        assert_eq!(eff.source[1], LAYER_PERSONA);
        assert_eq!(eff.source[2], LAYER_PROFILE);
        assert_eq!(eff.source[3], LAYER_DEFAULTS);
    }

    /// `rotate_interval` parses the same `30m` / `2h` / `1d` syntax the
    /// rest of Proteus uses; off-format strings transparently fall
    /// through (no `Err` to bubble up).
    #[test]
    fn rotate_interval_parses_compact_duration() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.per_ssid.insert(
            "x".into(),
            PerSsidPolicy {
                rotate_interval: Some("45m".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "x");
        assert_eq!(eff.rotate_interval, Some(Duration::from_secs(45 * 60)));
    }

    #[test]
    fn rotate_interval_garbage_yields_none() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.per_ssid.insert(
            "x".into(),
            PerSsidPolicy {
                rotate_interval: Some("not-a-duration".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "x");
        assert!(eff.rotate_interval.is_none());
    }

    #[test]
    fn parse_duration_recognises_each_unit() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86_400)));
        assert!(parse_duration("").is_none());
        assert!(parse_duration("xx").is_none());
        assert!(parse_duration("3w").is_none());
    }

    /// Empty per-SSID block (`[per_ssid."x"]` with no fields) is treated
    /// as "no contribution" — the layer is not stamped into source.
    #[test]
    fn empty_per_ssid_block_does_not_appear_in_source() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.per_ssid.insert("x".into(), PerSsidPolicy::default());
        let eff = resolve_for_ssid(&cfg, "x");
        assert!(!eff.source.contains(&LAYER_PER_SSID));
    }

    /// SSIDs are case-sensitive; mismatched casing falls through to
    /// the global path.
    #[test]
    fn ssid_lookup_is_case_sensitive() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.per_ssid.insert(
            "Coffee-Shop".into(),
            PerSsidPolicy {
                pin_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "coffee-shop");
        assert!(eff.pin_mac.is_none());
        let eff_exact = resolve_for_ssid(&cfg, "Coffee-Shop");
        assert_eq!(eff_exact.pin_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }
}
