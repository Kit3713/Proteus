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

/// Maximum number of characters [`display_string`] will emit before
/// truncating with a trailing `…`. Picked so a typical terminal line does
/// not get wrapped into a many-line paragraph by a hostile peer; the cap is
/// generous enough to fit any real `Location:` header (RFC 7230 imposes no
/// limit, but 8 KB is the de-facto server cap and 1024 chars is plenty for
/// human review).
pub const MAX_DISPLAY_LEN: usize = 1024;

/// Sanitize `s` for human-facing rendering. See module docs for rules.
pub fn display_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_DISPLAY_LEN));
    let mut emitted = 0usize;
    for c in s.chars() {
        if emitted >= MAX_DISPLAY_LEN {
            out.push('…');
            return out;
        }
        let cp = c as u32;
        match c {
            ' ' => {
                out.push(' ');
                emitted += 1;
            }
            '\\' => {
                out.push_str("\\\\");
                emitted += 1;
            }
            _ if cp < 0x20 || cp == 0x7f => {
                out.push_str(&format!("\\x{cp:02x}"));
                emitted += 1;
            }
            _ if (0x80..=0x9f).contains(&cp) => {
                out.push_str(&format!("\\u{{{cp:04x}}}"));
                emitted += 1;
            }
            // BiDi formatting controls — neutralise so a hostile peer can't
            // visually swap order of subsequent bytes ("login.evil.com" ->
            // "moc.live.nigol" rendered as "login.evil.com").
            _ if is_bidi_override(cp) => {
                out.push_str(&format!("\\u{{{cp:04x}}}"));
                emitted += 1;
            }
            _ => {
                out.push(c);
                emitted += 1;
            }
        }
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
}
