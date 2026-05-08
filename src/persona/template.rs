// SPDX-License-Identifier: GPL-3.0-or-later

//! Persona template rendering for hostname / Bluetooth alias / DHCP
//! `host_name` slots (roadmap Milestone 2 "Integration").
//!
//! The grammar is intentionally minimal — three tokens, no escaping:
//!
//! - `{owner}` — picks from a small first-name pool. Real device covers
//!   ("Sarah's iPhone") feel right with a personal name; avoiding email
//!   format keeps the rendered name RFC 1123-shaped.
//! - `{n}` — a 1-4 digit decimal. Used by Galaxy/Pixel-style templates
//!   where a sequence number sells the cover.
//! - `{word}` — a wordlist pick from `data/hostname-wordlist.txt`. Lets
//!   IoT / router personas reuse the existing 534-word entropy pool.
//!
//! Rendering is host-derived-input free: the only entropy comes from
//! `getrandom`, the wordlist, and the persona's own template string.
//! That keeps the rendered name from re-leaking the user's actual name
//! or hostname.

use anyhow::{Result, anyhow};

/// First-name pool for `{owner}`. Twenty entries: small enough to stay
/// in cache, big enough that the per-rotation entropy is meaningful
/// (log2(20) ≈ 4.3 bits). All ASCII-printable, RFC 1123-safe so the
/// rendered name passes `hostname::validate_hostname`.
pub const OWNER_POOL: &[&str] = &[
    "alex", "sam", "chris", "jamie", "morgan", "taylor", "casey", "jordan", "riley", "drew",
    "avery", "robin", "skyler", "blake", "dakota", "harper", "kai", "kim", "lee", "max",
];

/// Render a persona template against the supplied wordlist. Returns the
/// final string with every `{owner}` / `{n}` / `{word}` token replaced.
/// Unknown tokens are left in place — the caller's hostname / alias
/// validator will surface them as an error so a typo'd template lands
/// at the user, not silently in `state.json`.
pub fn render_template(template: &str, wordlist: &[&str]) -> Result<String> {
    if template.is_empty() {
        return Err(anyhow!("persona template is empty"));
    }
    if wordlist.is_empty() {
        return Err(anyhow!("hostname wordlist is empty"));
    }
    let mut out = String::with_capacity(template.len() + 8);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        // Read `{token}` ahead. Bail if the closing brace is missing —
        // a half-token is a hand-edit bug and the user wants to see it.
        let mut tok = String::new();
        let mut closed = false;
        for cc in chars.by_ref() {
            if cc == '}' {
                closed = true;
                break;
            }
            tok.push(cc);
        }
        if !closed {
            return Err(anyhow!("template '{template}' has unclosed '{{'"));
        }
        match tok.as_str() {
            "owner" => out.push_str(pick_owner()?),
            "n" => out.push_str(&format!("{}", pick_digits()?)),
            "word" => out.push_str(pick_word(wordlist)?),
            other => {
                // Unknown token: leave it in place, surfaced as `{other}`
                // so the validator catches the typo with a legible error.
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    Ok(out)
}

fn pick_owner() -> Result<&'static str> {
    // NMOD.4: defensive guard against an empty OWNER_POOL. The constant
    // is hand-maintained above with 20 entries; we statically assert
    // against accidental empties at compile time, and at runtime fall
    // through to a clear error rather than letting `unbiased_index(0, _)`
    // or a later `OWNER_POOL[idx]` index out of bounds. `panic = abort`
    // is set crate-wide — an out-of-bounds index would terminate the
    // events daemon — so the cost of one extra branch is well worth it.
    const _: () = assert!(
        !OWNER_POOL.is_empty(),
        "OWNER_POOL must contain at least one entry; an empty pool would index-OOB at rotate time"
    );
    if OWNER_POOL.is_empty() {
        return Err(anyhow!("OWNER_POOL is empty; persona owner-token slot cannot be filled"));
    }
    // Issue #226: rejection-sampled byte stream. OWNER_POOL is 20 entries
    // — well below the byte-stream picker's 256-entry ceiling. Naive
    // `byte % 20` would skew 16 of the 20 indices ~6% high, clustering
    // persona-rendered names by which name landed more often than chance.
    let idx = crate::rand::unbiased_index(OWNER_POOL.len(), crate::rand::getrandom_byte)?;
    Ok(OWNER_POOL[idx])
}

fn pick_word<'a>(words: &'a [&'a str]) -> Result<&'a str> {
    // Issue #226: the embedded hostname wordlist is 534 entries, which
    // exceeds the byte-stream picker's ceiling. Drop into the u64-stream
    // variant so wordlist picks are uniform too.
    let idx = crate::rand::unbiased_index_u64(words.len(), crate::rand::getrandom_u64)?;
    Ok(words[idx])
}

/// 1-4 digit decimal. Uses 4 bits of entropy to choose the width
/// (1..=4) and then `getrandom` for the value. Realistic on phones
/// where templates like `Galaxy-S24-{n}` pick up small sequence
/// numbers the user wouldn't notice.
///
/// Issue #226: width is picked via the rejection-sampled u8 helper
/// (`256 % 4 == 0`, so `byte % 4` is actually unbiased here, but going
/// through the helper keeps every selection in this file on the same
/// audited path). The decimal value `% max` is *not* a pool index —
/// it's a cap on a counter range — so it stays as plain modulo.
fn pick_digits() -> Result<u32> {
    let width = crate::rand::unbiased_index(4, crate::rand::getrandom_byte)? + 1;
    let max = 10u32.pow(width as u32);
    let mut buf = [0u8; 4];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(u32::from_le_bytes(buf) % max)
}

/// NTEST.1: deterministic renderer used by `persona::load::schema_check`
/// to feed a candidate template through `hostname::validate_hostname`
/// without touching `getrandom`. Substitutes `{owner}`, `{n}`, `{word}`
/// with their canonical lowest-entropy representatives — `alex` (the
/// first OWNER_POOL entry, ASCII-lowercase by construction), `0` (a
/// single ASCII digit), and a caller-supplied word from the persona's
/// wordlist (the loader passes a tiny static list when the production
/// wordlist isn't relevant). Unknown tokens survive as `{token}` so the
/// downstream validator catches the typo.
///
/// Used at load time, not apply time; the apply path keeps using the
/// real rejection-sampled `render_template`.
pub fn render_for_validation(template: &str, sample_word: &str) -> String {
    render_with_static(template, OWNER_POOL.first().copied().unwrap_or("a"), 0, sample_word)
}

fn render_with_static(template: &str, owner: &str, digit: u32, word: &str) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut tok = String::new();
        let mut closed = false;
        for cc in chars.by_ref() {
            if cc == '}' {
                closed = true;
                break;
            }
            tok.push(cc);
        }
        if !closed {
            // Mirror the live renderer's "unclosed brace" — surface as a
            // literal `{` so the validator sees the bug.
            out.push('{');
            out.push_str(&tok);
            return out;
        }
        match tok.as_str() {
            "owner" => out.push_str(owner),
            "n" => out.push_str(&format!("{digit}")),
            "word" => out.push_str(word),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}

/// Pure helper used by tests: render a template against an explicit
/// owner / digit / word picker so behaviour stays deterministic
/// without recompiling.
#[cfg(test)]
pub(crate) fn render_with(template: &str, owner: &str, digit: u32, word: &str) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut tok = String::new();
        for cc in chars.by_ref() {
            if cc == '}' {
                break;
            }
            tok.push(cc);
        }
        match tok.as_str() {
            "owner" => out.push_str(owner),
            "n" => out.push_str(&format!("{digit}")),
            "word" => out.push_str(word),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_pool_is_kebab_safe() {
        // Each owner must survive RFC 1123 hostname validation when
        // dropped into a template like `{owner}s-iPhone`. The hostname
        // validator rejects uppercase, so check the pool's casing.
        for o in OWNER_POOL {
            assert!(
                o.chars().all(|c| c.is_ascii_lowercase()),
                "owner '{o}' must be ascii-lowercase to pass RFC 1123"
            );
            assert!(!o.is_empty(), "empty owner");
        }
        assert!(OWNER_POOL.len() >= 15, "owner pool should be ~20 entries");
    }

    #[test]
    fn render_empty_template_errors() {
        let words = ["fedora", "linksys"];
        assert!(render_template("", &words).is_err());
    }

    #[test]
    fn render_empty_wordlist_errors() {
        let words: [&str; 0] = [];
        assert!(render_template("{word}", &words).is_err());
    }

    #[test]
    fn render_unclosed_brace_errors() {
        let words = ["fedora"];
        let err = render_template("hi-{owner", &words).unwrap_err();
        assert!(err.to_string().contains("unclosed"), "got: {err}");
    }

    #[test]
    fn render_with_known_picks() {
        // The pure helper should substitute every known token.
        let s = render_with("{owner}s-iPhone", "alex", 0, "fedora");
        assert_eq!(s, "alexs-iPhone");
        let s = render_with("Galaxy-S24-{n}", "alex", 4242, "fedora");
        assert_eq!(s, "Galaxy-S24-4242");
        let s = render_with("{word}-router", "alex", 0, "linksys");
        assert_eq!(s, "linksys-router");
    }

    #[test]
    fn render_unknown_token_is_left_alone() {
        // `{model}` isn't in the grammar; leave it intact so the
        // hostname validator catches the typo.
        let s = render_with("{model}-xx", "alex", 0, "fedora");
        assert_eq!(s, "{model}-xx");
    }

    #[test]
    fn render_template_against_real_random_picks_a_plausible_name() {
        // Smoke test the live renderer: the result must contain
        // recognised character classes only and never the raw token.
        let words = ["fedora", "linksys", "tplink"];
        for _ in 0..16 {
            let r = render_template("{owner}s-iPhone", &words).unwrap();
            assert!(!r.contains("{owner}"), "owner token must be substituted");
            assert!(r.ends_with("s-iPhone"));
        }
        for _ in 0..16 {
            let r = render_template("Galaxy-S24-{n}", &words).unwrap();
            assert!(!r.contains("{n}"));
            assert!(r.starts_with("Galaxy-S24-"));
        }
    }

    #[test]
    fn render_iphone_template_substitutes_owner() {
        // Pin the canonical iPhone template behaviour: result begins
        // with one of the OWNER_POOL entries and ends with `s-iPhone`.
        for _ in 0..50 {
            let r = render_template("{owner}s-iPhone", &["x"]).unwrap();
            let prefix = r.trim_end_matches("s-iPhone");
            assert!(
                OWNER_POOL.contains(&prefix),
                "rendered prefix '{prefix}' must come from OWNER_POOL"
            );
        }
    }

    #[test]
    fn render_n_template_produces_only_digits() {
        for _ in 0..50 {
            let r = render_template("{n}", &["x"]).unwrap();
            assert!(
                r.chars().all(|c| c.is_ascii_digit()),
                "{{n}}-only template must yield digits, got '{r}'"
            );
            assert!(!r.is_empty());
            assert!(r.len() <= 4);
        }
    }
}
