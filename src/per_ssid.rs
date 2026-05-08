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

/// Issue #224: render an SSID for terminal/journald output without
/// leaking attacker-controlled escape sequences.
///
/// SSIDs are 0-32 octets of arbitrary bytes per IEEE 802.11; a hostile
/// AP can broadcast e.g. `\x1b[2J\x1b[31mPROTEUS ERROR\x1b[0m` and any
/// unfiltered render of that string against the operator's terminal
/// becomes a message-spoofing primitive (clear screen, repaint with
/// fake error, OSC clipboard injection, etc.). Proteus runs as root in
/// the dispatcher path and surfaces SSIDs in `proteus ssid list/show`,
/// in tracing logs that journald renders, and in the dispatcher's own
/// `logger -t proteus-dispatcher` calls.
///
/// Filter rules:
/// - C0 controls (bytes < 0x20) other than space → `\xNN`.
/// - DEL (0x7f) → `\x7f`.
/// - C1 controls (0x80–0x9F, including the bare-byte CSI 0x9b some
///   modern xterms still parse) → `\u{NNNN}`.
/// - Backslash → `\\` so the escape form round-trips unambiguously.
/// - Everything else passes through, including legitimate non-ASCII.
///
/// Apply at every print, log, and TOML-key-echo site that surfaces an
/// SSID. The wire-side bytes in `state.json` / `config.toml` round-trip
/// through serde unchanged — only the rendered-for-human form is
/// sanitized.
pub fn display_ssid(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        match c {
            ' ' => out.push(' '),
            '\\' => out.push_str("\\\\"),
            _ if cp < 0x20 || cp == 0x7f => {
                out.push_str(&format!("\\x{:02x}", cp));
            }
            _ if (0x80..=0x9f).contains(&cp) => {
                out.push_str(&format!("\\u{{{:04x}}}", cp));
            }
            _ => out.push(c),
        }
    }
    out
}

/// Issue #224: hard-reject SSIDs containing a NUL byte at the
/// validation boundary. NUL is unconditionally either an SSID-encoding
/// bug or hostile input — IEEE 802.11 doesn't reserve a meaning for it,
/// most NM/wpa_supplicant code paths use NUL as a string terminator
/// internally, and it has no display form that round-trips through
/// shell editors. Used by `proteus ssid set` and `resolve_for_ssid`.
pub fn validate_ssid(ssid: &str) -> Result<(), &'static str> {
    if ssid.is_empty() {
        return Err("ssid must not be empty");
    }
    if ssid.contains('\0') {
        return Err("ssid contains a NUL byte (0x00); rejected");
    }
    Ok(())
}

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
    let per_profile =
        per.and_then(|p| p.aggressiveness_profile.as_deref().and_then(Profile::parse));
    let per_pin = per.and_then(|p| p.pin_mac.clone());
    let per_rotate = per.and_then(|p| p.rotate_interval.as_deref().and_then(parse_duration));
    let per_portal = per.and_then(|p| p.portal_policy.clone());
    let per_contributed = per.is_some_and(per_block_has_any_field);
    if per_contributed {
        source.push(LAYER_PER_SSID);
    }

    // Persona layer: today only the persona id falls out of `[persona]
    // active`. Future fields (e.g. persona-defined rotate cadence) can
    // fold in here without touching call sites. Compute "did the global
    // persona contribute" (V9) before the `or_else` move consumes
    // `per_persona` — the variable is named `global_persona_contributed`
    // because `per_ssid` may legitimately layer its own `persona` *and*
    // the global `[persona] active` may also be set; the latter only
    // appears in the source trace when the per-SSID layer left the slot
    // empty for the global persona to fill. Without the name, the
    // 4-layer source-trace reads as "persona never affected this SSID"
    // even when the persona-shaped global *did* layer through.
    let global_persona_contributed = per_persona.is_none() && config.persona.active.is_some();
    let persona = per_persona.or_else(|| config.persona.active.clone());
    if global_persona_contributed {
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
///
/// Issue #272: previously this used `s.split_at(s.len() - 1)` which slices
/// on a byte boundary. A multi-byte UTF-8 trailing character (e.g. `5µ`,
/// where `µ` is a 2-byte sequence) would land mid-codepoint and panic.
/// `panic = abort` is set crate-wide, so a hostile or buggy config value
/// would abort the events daemon. We now split on the last *character*
/// boundary via `char_indices` and reject any non-ASCII suffix as
/// off-format (returning `None`), which causes the resolver to transparently
/// fall through to the global timer rather than abort.
///
/// Issues N12.4 / V8: the previous body used `n * 60`, `n * 3600`,
/// `n * 86_400` unconditionally. Combined with a u64-shaped numeric
/// prefix (e.g. `9999999999999999d`) that overflows multiplication and
/// triggers an arithmetic-overflow panic in debug builds, or silently
/// wraps in release. Use `checked_mul` and emit a `tracing::warn!` on
/// overflow so the operator sees *why* their per-SSID timer fell through
/// to the global cadence rather than the silent-fallback that V8 calls
/// out. The function still returns `None` on overflow — the resolver
/// treats `None` as "use the global timer," which is the safe default —
/// but the warn distinguishes "no per-SSID value set" (no log line) from
/// "value set but unusable" (logged so the user can fix it).
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Find the last character's byte offset. `char_indices` yields
    // `(byte_offset, char)` for each code point; the last entry's offset
    // is where the final char starts, which is the only safe split point.
    let last_idx = s.char_indices().next_back()?.0;
    let (num, unit) = s.split_at(last_idx);
    // Reject anything where the unit isn't a single ASCII byte — `µ`,
    // emoji, etc. can't be one of our four units, so the parse fails
    // cleanly rather than panicking.
    if unit.len() != 1 || !unit.is_ascii() {
        return None;
    }
    let n: u64 = num.parse().ok()?;
    let secs_opt = match unit {
        "s" => Some(n),
        "m" => n.checked_mul(60),
        "h" => n.checked_mul(3600),
        "d" => n.checked_mul(86_400),
        _ => return None,
    };
    match secs_opt {
        Some(secs) => Some(Duration::from_secs(secs)),
        None => {
            tracing::warn!(
                input = s,
                "per-SSID rotate_interval overflowed u64 seconds; falling back to global timer"
            );
            None
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    /// Issue #224: ANSI escape (`\x1b`) and clear-screen / colour codes
    /// land as `\xNN`. The repaint-attack payload from the issue body
    /// renders inert.
    #[test]
    fn display_ssid_neuters_ansi_escape_payload() {
        let raw = "\x1b[2J\x1b[H\x1b[31mPROTEUS ERROR: pay 1 BTC\x1b[0m";
        let out = display_ssid(raw);
        assert!(!out.contains('\x1b'), "ESC byte must be escaped: {out:?}");
        assert!(out.starts_with("\\x1b"), "ESC renders as \\x1b: {out:?}");
        assert!(
            out.contains("PROTEUS ERROR"),
            "literal text passes through: {out:?}"
        );
    }

    /// OSC-style clipboard injection (`\x1b]52;c;<base64>\x07`) — every
    /// control byte should escape; only the printable middle survives.
    #[test]
    fn display_ssid_neuters_osc_clipboard_injection() {
        let raw = "\x1b]52;c;dGVzdA==\x07";
        let out = display_ssid(raw);
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\x07'));
    }

    /// C1 controls (0x80-0x9F) get the `\u{NNNN}` form. CSI=0x9b is the
    /// classic concern: some xterms still parse the bare 0x9b byte as
    /// ESC[.
    #[test]
    fn display_ssid_escapes_c1_controls() {
        let raw = "\u{009b}[31m";
        let out = display_ssid(raw);
        assert!(!out.contains('\u{009b}'));
        assert!(out.contains("\\u{009b}"), "C1 CSI escapes: {out:?}");
    }

    /// Backslash itself round-trips so the operator can tell a real
    /// `\x1b` in the SSID from one Proteus inserted while escaping.
    #[test]
    fn display_ssid_escapes_backslash() {
        assert_eq!(display_ssid("a\\b"), "a\\\\b");
    }

    /// Common case: an ASCII-printable SSID survives untouched. Pin so
    /// a future filter doesn't over-escape benign names.
    #[test]
    fn display_ssid_passes_through_printable_ascii() {
        assert_eq!(display_ssid("Coffee Shop Wi-Fi"), "Coffee Shop Wi-Fi");
    }

    /// Legitimate non-ASCII (e.g. an emoji or accented character)
    /// survives untouched. The filter targets controls, not text.
    #[test]
    fn display_ssid_passes_through_unicode() {
        assert_eq!(display_ssid("café"), "café");
        assert_eq!(display_ssid("café 📶"), "café 📶");
    }

    /// `validate_ssid` rejects empty and NUL-bearing SSIDs but accepts
    /// arbitrary other UTF-8 text — sanitization is `display_ssid`'s
    /// job.
    #[test]
    fn validate_ssid_boundary_rules() {
        assert!(validate_ssid("").is_err());
        assert!(validate_ssid("a\0b").is_err());
        assert!(validate_ssid("Coffee Shop").is_ok());
        // ESC byte alone is not a validation failure — the operator
        // can store it; only the render is sanitized.
        assert!(validate_ssid("\x1b[31m").is_ok());
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

    /// Issue #272 regression: the previous implementation used
    /// `split_at(s.len() - 1)`, which slices on a byte boundary. The
    /// trailing `µ` (U+00B5) is two UTF-8 bytes, so the byte-boundary split
    /// landed mid-codepoint and panicked. With `panic = abort` set
    /// crate-wide, this aborted the events daemon every time a per-SSID
    /// config carried an unusual suffix. The fix splits on the last *char*
    /// boundary via `char_indices` and rejects non-ASCII suffixes as
    /// off-format. This test must exit cleanly — never panic — and return
    /// `None` for the bad input.
    #[test]
    fn parse_duration_handles_multibyte_utf8_without_panic() {
        // Two-byte UTF-8 (`µ` = U+00B5, encoded as 0xC2 0xB5)
        assert!(parse_duration("5µ").is_none());
        // Four-byte UTF-8 (emoji, encoded as 4 bytes)
        assert!(parse_duration("5🦀").is_none());
        // Three-byte UTF-8 (CJK ideograph)
        assert!(parse_duration("5日").is_none());
        // Bare multi-byte sequence with no leading number
        assert!(parse_duration("µ").is_none());
        // Multi-byte interior char with ASCII suffix is also rejected — the
        // numeric part of `1µs` ("1µ") fails to parse as u64.
        assert!(parse_duration("1µs").is_none());
    }

    /// Companion to the multibyte test: the standard ASCII suffixes still
    /// work after the char_indices fix. Pin the happy path so the regression
    /// fix doesn't regress the regression fix.
    #[test]
    fn parse_duration_ascii_suffixes_still_parse_after_utf8_fix() {
        assert_eq!(parse_duration("1s"), Some(Duration::from_secs(1)));
        assert_eq!(parse_duration("60s"), Some(Duration::from_secs(60)));
        assert_eq!(parse_duration("0s"), Some(Duration::from_secs(0)));
    }

    /// N12.4 / V8 regression: a numerically-valid but multiplicatively-
    /// overflowing duration (e.g. `u64::MAX / 60 + 1` minutes) used to
    /// either abort in debug builds or silently wrap in release. The fix
    /// uses `checked_mul` and returns `None` on overflow so the resolver
    /// transparently falls through to the global timer — and emits a
    /// `tracing::warn!` so the operator sees *why* (V8: distinguish "no
    /// value" from "value but unusable").
    #[test]
    fn parse_duration_overflow_returns_none_instead_of_panicking() {
        // `u64::MAX` minutes overflows when multiplied by 60.
        let huge = format!("{}m", u64::MAX);
        assert!(
            parse_duration(&huge).is_none(),
            "overflow case must return None, not panic or wrap"
        );
        // Same for hours and days.
        let huge_h = format!("{}h", u64::MAX);
        assert!(parse_duration(&huge_h).is_none());
        let huge_d = format!("{}d", u64::MAX);
        assert!(parse_duration(&huge_d).is_none());
        // Sanity: the seconds suffix has no multiplier so it can carry
        // any u64 the caller throws at it.
        assert_eq!(
            parse_duration(&format!("{}s", u64::MAX)),
            Some(Duration::from_secs(u64::MAX))
        );
    }

    /// V9: the renamed `global_persona_contributed` flag is exercised
    /// indirectly through the source-trace ordering. With per-SSID
    /// supplying a persona, the global persona must NOT contribute even
    /// though `config.persona.active` is set — proves the flag is gating
    /// correctly under the new name.
    #[test]
    fn global_persona_does_not_contribute_when_per_ssid_supplies_persona() {
        let mut cfg = cfg_with_profile(Profile::Med);
        cfg.persona.active = Some("randomizer-med".into());
        cfg.per_ssid.insert(
            "x".into(),
            PerSsidPolicy {
                persona: Some("iphone-15".into()),
                ..PerSsidPolicy::default()
            },
        );
        let eff = resolve_for_ssid(&cfg, "x");
        // The per-SSID layer wins — global persona must NOT show up in
        // the source trace because per-SSID supplied the persona slot.
        assert_eq!(eff.persona.as_deref(), Some("iphone-15"));
        assert!(!eff.source.contains(&LAYER_PERSONA));
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
