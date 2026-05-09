// SPDX-License-Identifier: GPL-3.0-or-later

//! Output-sanitization helper for any attacker-controlled string Proteus
//! renders to a human terminal or to journald.
//!
//! Issue #241 (captive-portal `Location:` header echoed verbatim into the
//! operator's terminal) is the immediate motivator, but the same primitive
//! covers any other place where a network peer's bytes reach a print/log
//! site without trusted shaping. The companion [`per_ssid::display_ssid`]
//! handles SSIDs specifically; this module is the catch-all for the rest.
//!
//! Stream 9 (security surface hardening) widened the call-site set: the
//! cluster of issues #357, #365, #367, #373, #374, #380, #389 all share
//! one root cause — display layer doesn't sanitize AP-controlled strings
//! (SSIDs, iface names from NM, persona display_name / notes). Rather
//! than a point-fix per issue, we ship the central [`display_safe`]
//! helper here and call it at every print site that surfaces such a
//! value.
//!
//! The render rules — repeated here so the call sites don't need to read
//! the implementation:
//!
//! - C0 controls (bytes < 0x20) other than space → `\xNN`.
//! - DEL (0x7f) → `\x7f`.
//! - C1 controls (0x80–0x9F, including the bare-byte CSI 0x9b some
//!   modern xterms still parse) → `\u{NNNN}`.
//! - BiDi overrides (LRO/RLO/PDF/LRE/RLE/LRI/RLI/FSI/PDI) → `\u{NNNN}`.
//!   These do not paint pixels themselves but reorder anything around
//!   them in a logically-rtl-but-visually-spoofed way.
//! - Backslash → `\\` so the escape form round-trips unambiguously.
//! - Everything else passes through, including legitimate non-ASCII.
//! - Output is clamped to [`MAX_DISPLAY_LEN`] characters; anything longer
//!   is truncated and a `…` marker appended so the operator can spot the
//!   clamp.

use std::borrow::Cow;

/// Maximum number of characters [`display_string`] will emit before
/// truncating with a trailing `…`. Picked so a typical terminal line does
/// not get wrapped into a many-line paragraph by a hostile peer; the cap is
/// generous enough to fit any real `Location:` header (RFC 7230 imposes no
/// limit, but 8 KB is the de-facto server cap and 1024 chars is plenty for
/// human review).
pub const MAX_DISPLAY_LEN: usize = 1024;

/// Sanitize `s` for human-facing rendering. See module docs for rules.
///
/// The output clamp counts **output** characters, not input. A hostile peer
/// can multiply length by sending bytes that escape to `\u{NNNN}` (six
/// chars per input char) or `\xNN` (four chars per input char); naïvely
/// counting input chars lets the rendered output blow past
/// `MAX_DISPLAY_LEN` by ~6× and re-introduces the line-wrap problem the
/// clamp exists to prevent (issue #388). We therefore check the
/// post-escape length of every emission and stop the moment the *next*
/// token would not fit.
pub fn display_string(s: &str) -> String {
    // We count chars (Unicode scalar values), not graneme clusters: the
    // escape forms above all emit ASCII (every output char is a single
    // codepoint), so `chars().count()` is exact for the escape side and
    // a safe lower bound for the pass-through side. Adding
    // `unicode-segmentation` would be more accurate for combining marks
    // in pass-through text, but it's a new dep — see N12.6 in
    // docs/ROADMAP.md if you need cluster-perfect clamping.
    let mut out = String::with_capacity(s.len().min(MAX_DISPLAY_LEN));
    let mut emitted = 0usize;
    // Helper: would emitting `n` more output chars exceed the budget?
    let exceeds = |emitted: usize, n: usize| emitted.saturating_add(n) > MAX_DISPLAY_LEN;
    for c in s.chars() {
        let cp = c as u32;
        // Compute the token to append and its char-length. We stage the
        // string lazily for the rare escape paths; the common pass-through
        // path stays a single push with no allocation.
        let (token, token_len): (String, usize) = match c {
            ' ' => (" ".to_string(), 1),
            '\\' => ("\\\\".to_string(), 2),
            _ if cp < 0x20 || cp == 0x7f => {
                let s = format!("\\x{cp:02x}");
                let len = s.chars().count();
                (s, len)
            }
            _ if (0x80..=0x9f).contains(&cp) || is_bidi_override(cp) => {
                // BiDi formatting controls and C1 controls — neutralise
                // so a hostile peer can't visually swap order of
                // subsequent bytes ("login.evil.com" ->
                // "moc.live.nigol" rendered as "login.evil.com").
                let s = format!("\\u{{{cp:04x}}}");
                let len = s.chars().count();
                (s, len)
            }
            _ => (c.to_string(), 1),
        };
        // If this token plus the trailing `…` marker won't fit, stop now
        // and emit the marker. We reserve one slot for `…` so the marker
        // itself never tips the output over the cap.
        if exceeds(emitted, token_len) {
            out.push('…');
            return out;
        }
        out.push_str(&token);
        emitted += token_len;
    }
    out
}

/// Unicode codepoints that reorder surrounding text without a visible
/// glyph: U+202A..U+202E (LRE/RLE/PDF/LRO/RLO), U+2066..U+2069
/// (LRI/RLI/FSI/PDI). Plus U+200E/U+200F (LRM/RLM) which paint nothing
/// but still flip directionality of weakly-typed neighbours.
fn is_bidi_override(cp: u32) -> bool {
    matches!(
        cp,
        0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
    )
}

/// Stream 9 central helper — sanitize attacker-controlled strings *only when
/// they need it*. Returns `Cow::Borrowed(s)` when the input is already safe
/// (the common case for plain-ASCII iface names, persona display names with
/// printable content, etc.) so the hot path allocates nothing; falls back
/// to the full [`display_string`] escaping only when an unsafe character is
/// present.
///
/// This is the helper every print site should use when surfacing values
/// that originated from an AP / NM dict / persona file: SSIDs, iface
/// names, persona ids, display_names, and notes. It centralises the rules
/// so a future widening of the unsafe-codepoint set lands once, not at
/// every call site (the cluster #357/#365/#367/#373/#374/#380/#389).
///
/// The truncation contract is identical to [`display_string`]: anything
/// longer than [`MAX_DISPLAY_LEN`] characters is escaped + clamped, even
/// when the input was otherwise safe — uncapped echo of an attacker-sized
/// SSID is itself a soft DoS on the operator's terminal.
pub fn display_safe(s: &str) -> Cow<'_, str> {
    let needs_escape = s.chars().any(needs_escaping);
    let too_long = s.chars().count() > MAX_DISPLAY_LEN;
    if !needs_escape && !too_long {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(display_string(s))
    }
}

/// Predicate matching the same set [`display_string`] escapes. Kept private
/// so the rule list lives in exactly one place.
fn needs_escaping(c: char) -> bool {
    let cp = c as u32;
    if c == ' ' {
        return false;
    }
    if c == '\\' {
        return true;
    }
    if cp < 0x20 || cp == 0x7f {
        return true;
    }
    if (0x80..=0x9f).contains(&cp) {
        return true;
    }
    if is_bidi_override(cp) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_plain_ascii() {
        assert_eq!(display_string("Hello, world."), "Hello, world.");
    }

    #[test]
    fn passes_through_unicode_letters() {
        assert_eq!(display_string("café"), "café");
        assert_eq!(display_string("北京"), "北京");
    }

    #[test]
    fn neuters_ansi_escape_csi() {
        // ESC [ 2 J — clear screen
        let raw = "\x1b[2Jhello";
        let out = display_string(raw);
        assert!(!out.contains('\x1b'), "raw ESC must be escaped: {out:?}");
        assert!(out.contains("\\x1b"));
        assert!(out.ends_with("hello"));
    }

    #[test]
    fn neuters_ansi_escape_osc() {
        // OSC 52 (clipboard injection) — ESC ] 52 ; c ; ... BEL
        let raw = "\x1b]52;c;ZXZpbA==\x07after";
        let out = display_string(raw);
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\x07'));
        assert!(out.contains("\\x1b"));
        assert!(out.contains("\\x07"));
    }

    #[test]
    fn escapes_c1_csi_byte() {
        // 0x9b is CSI as a single byte — some terminals still parse it.
        let raw = "\u{9b}2Khello";
        let out = display_string(raw);
        assert!(!out.contains('\u{9b}'));
        assert!(out.contains("\\u{009b}"));
    }

    #[test]
    fn escapes_bidi_override() {
        // U+202E (RIGHT-TO-LEFT OVERRIDE) — classic spoofing primitive.
        let raw = "https://login.evil.com\u{202e}/safe.bank.com";
        let out = display_string(raw);
        assert!(!out.contains('\u{202e}'));
        assert!(out.contains("\\u{202e}"));
    }

    #[test]
    fn escapes_bidi_marks() {
        for cp in [0x200E, 0x200F, 0x2066, 0x2067, 0x2068, 0x2069] {
            let s = char::from_u32(cp).unwrap().to_string();
            let out = display_string(&s);
            assert!(
                !out.chars().any(|c| c as u32 == cp),
                "codepoint U+{cp:04X} should be escaped, got {out:?}",
            );
        }
    }

    #[test]
    fn escapes_backslash() {
        assert_eq!(display_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn clamps_oversize_input() {
        let raw = "A".repeat(MAX_DISPLAY_LEN + 100);
        let out = display_string(&raw);
        // The clamp marker is one character, so the output is at most
        // `MAX_DISPLAY_LEN + 1` chars.
        assert!(out.chars().count() <= MAX_DISPLAY_LEN + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn does_not_clamp_at_threshold() {
        let raw = "A".repeat(MAX_DISPLAY_LEN);
        let out = display_string(&raw);
        assert_eq!(out.chars().count(), MAX_DISPLAY_LEN);
        assert!(!out.contains('…'));
    }

    /// Regression for GH#388 / N12.6: a hostile peer can amplify length by
    /// sending control bytes that each escape to four chars (`\xNN`). The
    /// pre-fix code counted input chars, so 1024 control bytes rendered
    /// as ~4096 output chars — six× the clamp on `\u{NNNN}` paths, four×
    /// on `\xNN`. The clamp must count **output** chars.
    #[test]
    fn clamp_counts_output_not_input_for_c0_controls() {
        // 1024 bell chars (each escapes to `\x07`, four output chars).
        let raw = "\x07".repeat(MAX_DISPLAY_LEN);
        let out = display_string(&raw);
        assert!(
            out.chars().count() <= MAX_DISPLAY_LEN + 1,
            "output {} chars exceeds clamp {} (input was {} chars of \\x07)",
            out.chars().count(),
            MAX_DISPLAY_LEN + 1,
            raw.chars().count(),
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn clamp_counts_output_not_input_for_c1_controls() {
        // C1 chars escape to `\u{NNNN}` — six output chars per input char.
        let raw = "\u{009b}".repeat(MAX_DISPLAY_LEN);
        let out = display_string(&raw);
        assert!(
            out.chars().count() <= MAX_DISPLAY_LEN + 1,
            "output {} chars exceeds clamp {} (input was C1 controls)",
            out.chars().count(),
            MAX_DISPLAY_LEN + 1,
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn clamp_counts_output_not_input_for_backslashes() {
        // Backslash escapes to `\\` — two output chars per input char.
        let raw = "\\".repeat(MAX_DISPLAY_LEN);
        let out = display_string(&raw);
        assert!(
            out.chars().count() <= MAX_DISPLAY_LEN + 1,
            "output {} chars exceeds clamp {} (input was backslashes)",
            out.chars().count(),
            MAX_DISPLAY_LEN + 1,
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!(display_string(""), "");
    }

    #[test]
    fn space_passes_through_but_other_c0_does_not() {
        let out = display_string("a b\tc\nd");
        assert!(out.contains(' '));
        assert!(out.contains("\\x09"));
        assert!(out.contains("\\x0a"));
    }

    // ---- display_safe (Cow wrapper, Stream 9 cluster fix) --------------

    #[test]
    fn display_safe_borrows_when_input_is_already_safe() {
        // The hot path: plain-ASCII iface names and SSIDs allocate nothing.
        let s = "wlan0";
        let out = display_safe(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "wlan0");

        let s = "Coffee Shop Wi-Fi";
        let out = display_safe(s);
        assert!(matches!(out, Cow::Borrowed(_)));

        // Legitimate non-ASCII passes through borrowed too.
        let s = "café";
        let out = display_safe(s);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn display_safe_escapes_ansi_csi_in_ssid() {
        // Issue #357 / #365 — hostile SSID with raw CSI must not paint.
        let raw = "EvilAP\x1b[31m";
        let out = display_safe(raw);
        assert!(matches!(out, Cow::Owned(_)));
        assert!(!out.contains('\x1b'));
        assert!(out.contains("\\x1b"));
    }

    #[test]
    fn display_safe_escapes_bidi_override_in_persona_display_name() {
        // Issue #374 / #380 — persona display_name with U+202E reorders
        // anything printed after it. The cluster fix is to escape, not
        // strip, so the operator sees the marker and the line layout is
        // preserved.
        let raw = "MyPhone\u{202e}gnp.gpj";
        let out = display_safe(raw);
        assert!(matches!(out, Cow::Owned(_)));
        assert!(!out.contains('\u{202e}'));
        assert!(out.contains("\\u{202e}"));
    }

    #[test]
    fn display_safe_escapes_cr_lf_nul() {
        // Issue #367 — iface name surfaced from NM with embedded LF /
        // NUL. NUL kills journald lines; LF/CR can let the attacker
        // emit fake log entries below the legitimate one.
        for raw in ["wlan\n0", "wlan\r0", "wlan\0attack"] {
            let out = display_safe(raw);
            assert!(matches!(out, Cow::Owned(_)));
            assert!(!out.contains('\n') || !raw.contains('\n'));
            assert!(!out.contains('\r') || !raw.contains('\r'));
            assert!(!out.contains('\0'));
        }
    }

    #[test]
    fn display_safe_clamps_oversize_input() {
        let raw = "A".repeat(MAX_DISPLAY_LEN + 100);
        let out = display_safe(&raw);
        assert!(matches!(out, Cow::Owned(_)));
        assert!(out.ends_with('…'));
    }

    #[test]
    fn display_safe_does_not_clamp_at_threshold() {
        let raw = "A".repeat(MAX_DISPLAY_LEN);
        let out = display_safe(&raw);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn display_safe_round_trips_with_display_string_when_unsafe() {
        // When escaping kicks in, display_safe must produce the identical
        // bytes display_string would. This is what lets the helper drop
        // in at every call site without behaviour drift.
        for raw in ["x\x1b[2J", "ssid\x07", "name\u{202e}rev", "with\\backslash"] {
            assert_eq!(display_safe(raw), display_string(raw));
        }
    }

    #[test]
    fn display_safe_handles_empty_input() {
        let out = display_safe("");
        assert_eq!(out, "");
    }
}
