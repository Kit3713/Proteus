// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Result, anyhow};

use crate::config::BluetoothConfig;

// Generic, non-host-derived strings. Anything host-derived (hostname, user
// name, device model) would re-leak the identifier the alias is meant to mask.
pub const GENERIC_ALIASES: &[&str] = &[
    "BT Device",
    "Bluetooth",
    "Bluetooth Device",
    "Linux BT",
    "Linux Bluetooth",
    "Wireless",
    "Wireless Device",
    "Audio Device",
    "Headset",
    "Speaker",
    "Mouse",
    "Keyboard",
    "Trackpad",
    "Controller",
    "Generic Adapter",
    "BLE Device",
    "BT Host",
    "Adapter",
    "Device",
];

pub fn select_alias(cfg: &BluetoothConfig) -> Result<String> {
    let alias = match cfg.alias_source.as_str() {
        "pinned" => cfg
            .pinned_alias
            .clone()
            .ok_or_else(|| anyhow!("alias_source = 'pinned' but pinned_alias is unset"))?,
        "generic" => generic()?,
        other => {
            return Err(anyhow!(
                "unknown alias_source '{other}'; expected 'generic' or 'pinned'"
            ));
        }
    };
    // Issue #236: validate the resolved alias before it ships to BlueZ
    // (and gets broadcast to nearby BT scanners + displayed verbatim by
    // any GUI that lists adapter aliases). Pinned aliases come from
    // user config, generic aliases from a hardcoded ASCII pool —
    // validation catches the pinned path's hostile input and the
    // generic path is a constant-cost no-op pass.
    validate_alias(&alias)?;
    Ok(alias)
}

/// Issue #236: hard-reject Bluetooth aliases that would carry attacker-
/// or typo-controlled bytes onto the air or into the operator's
/// `bluetoothctl info` output. The hostname path has the equivalent
/// validator (`crate::hostname::validate_hostname`); the BT path was
/// missing one. Mirrors that shape:
///
/// - Empty rejected (BlueZ accepts but a blank broadcast leaks "this
///   adapter is unconfigured").
/// - 248-byte cap matches the EIR payload limit BlueZ truncates at;
///   rejecting up-front avoids burning DBus bytes on a string that
///   won't reach scanners anyway.
/// - C0 / C1 / NUL / DEL controls — same terminal-injection vector as
///   #224 (SSID escapes), since the alias surfaces in tools that print
///   to a TTY.
/// - BiDi override codepoints (LRM/RLM/LRE-PDF/LRI-PDI) — homoglyph /
///   right-to-left disguise primitives. A scanner near the host
///   sees what looks like one device name; a tool rendering the same
///   bytes through Unicode awareness sees something different.
///
/// Called by `select_alias` and `select_alias_with_persona` so every
/// path that ships an alias to BlueZ runs the check.
pub fn validate_alias(s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(anyhow!("Bluetooth alias is empty"));
    }
    if s.len() > 248 {
        return Err(anyhow!(
            "Bluetooth alias exceeds 248 bytes ({} bytes); BlueZ would truncate the EIR broadcast",
            s.len()
        ));
    }
    for c in s.chars() {
        let cp = c as u32;
        if cp == 0 {
            return Err(anyhow!("Bluetooth alias contains a NUL byte"));
        }
        if c.is_control() {
            return Err(anyhow!(
                "Bluetooth alias contains control character U+{:04X}",
                cp
            ));
        }
        if matches!(
            cp,
            // LRM / RLM / LRE / RLE / PDF / LRO / RLO / LRI / RLI / FSI / PDI
            0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        ) {
            return Err(anyhow!(
                "Bluetooth alias contains BiDi override character U+{:04X}",
                cp
            ));
        }
    }
    Ok(())
}

/// Roadmap M2 "Integration": pick the BlueZ adapter alias honouring an
/// active persona. When a persona is set and supplies a `bt_name_template`,
/// that template is rendered (against the same wordlist + token pools the
/// hostname renderer uses) and returned. Otherwise the existing
/// pinned/generic flow runs unchanged so v0.2.x users see no behaviour
/// change.
///
/// `cfg.pinned_alias` (when `alias_source = "pinned"`) intentionally beats
/// the persona path — the operator's explicit pin always wins. This
/// mirrors the precedence rule used for DHCP and hostname.
pub fn select_alias_with_persona(
    cfg: &BluetoothConfig,
    persona: Option<&crate::persona::Persona>,
) -> Result<String> {
    if cfg.alias_source.as_str() == "pinned" {
        let pinned = cfg
            .pinned_alias
            .clone()
            .ok_or_else(|| anyhow!("alias_source = 'pinned' but pinned_alias is unset"))?;
        // Issue #236: validate before returning so a hostile pinned
        // alias never reaches BlueZ.
        validate_alias(&pinned)?;
        return Ok(pinned);
    }
    if let Some(p) = persona
        && !p.bt_name_template.trim().is_empty()
    {
        // Wordlist piggybacks on the hostname pool — there's no separate
        // BT-specific dictionary and the existing 534 entries cover the
        // generic-name space well enough for now.
        let words = crate::hostname::wordlist()?;
        let rendered =
            crate::persona::template::render_template(&p.bt_name_template, &words)?;
        // Issue #236: persona-supplied templates are user-authored
        // (built-ins are static `data/personas/*.toml`; user personas
        // come from `/etc/proteus/personas/` which `proteus persona
        // import` writes). Both sources can carry control bytes after
        // template rendering.
        validate_alias(&rendered)?;
        return Ok(rendered);
    }
    select_alias(cfg)
}

fn generic() -> Result<String> {
    let idx = unbiased_index(GENERIC_ALIASES.len(), getrandom_byte)?;
    Ok(GENERIC_ALIASES[idx].to_string())
}

fn getrandom_byte() -> Result<u8> {
    let mut buf = [0u8; 1];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(buf[0])
}

/// Rejection-sampled `[0, len)` from a stream of bytes. Avoids the modulo
/// bias of `byte % len` when `256 % len != 0` — for the 19-entry pool the
/// naive `% 19` skews 4 of the 19 indices ~5% high (issues #143/#152/#154).
/// Worst case for any `len <= 256` is two extra random byte reads, so the
/// cost is negligible.
fn unbiased_index<F: FnMut() -> Result<u8>>(len: usize, mut next: F) -> Result<usize> {
    if len == 0 {
        return Err(anyhow!("cannot pick from empty pool"));
    }
    if len > 256 {
        // The byte-stream picker only covers up to 256 distinct values; this
        // never happens in practice (our pool is fixed) but guard explicitly
        // so a future caller can't silently fall back to biased modulo.
        return Err(anyhow!("unbiased_index supports len <= 256 (got {len})"));
    }
    let span = 256 - (256 % len);
    loop {
        let byte = next()? as usize;
        if byte < span {
            return Ok(byte % len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(source: &str, pinned: Option<&str>) -> BluetoothConfig {
        BluetoothConfig {
            enabled: true,
            generic_alias: true,
            alias_source: source.into(),
            pinned_alias: pinned.map(str::to_string),
            discoverable: false,
            ble_rpa: true,
        }
    }

    #[test]
    fn generic_aliases_have_at_least_fifteen_entries() {
        assert!(
            GENERIC_ALIASES.len() >= 15,
            "need a decent pool to avoid trivial guess-the-alias",
        );
    }

    #[test]
    fn generic_aliases_have_no_host_strings() {
        // None of the entries should look like hostname/user-derived data.
        for a in GENERIC_ALIASES {
            assert!(
                !a.contains("'"),
                "alias '{a}' contains an apostrophe, suggests possessive"
            );
            assert!(!a.contains('@'), "alias '{a}' contains '@', suggests email");
            assert!(
                a.is_ascii(),
                "alias '{a}' has non-ascii chars (could leak locale)"
            );
        }
    }

    #[test]
    fn generic_returns_one_of_the_pool() {
        for _ in 0..50 {
            let pick = select_alias(&cfg("generic", None)).unwrap();
            assert!(
                GENERIC_ALIASES.contains(&pick.as_str()),
                "pick '{pick}' not in pool"
            );
        }
    }

    #[test]
    fn pinned_returns_pinned_value() {
        let pick = select_alias(&cfg("pinned", Some("MyBT"))).unwrap();
        assert_eq!(pick, "MyBT");
    }

    #[test]
    fn pinned_without_value_errors() {
        assert!(select_alias(&cfg("pinned", None)).is_err());
    }

    #[test]
    fn unknown_source_errors() {
        assert!(select_alias(&cfg("nonsense", None)).is_err());
    }

    // === Roadmap M2 "Integration" — persona-aware alias ===

    fn persona_with_bt_template(template: &str) -> crate::persona::Persona {
        crate::persona::Persona {
            id: "iphone".into(),
            display_name: "iPhone".into(),
            kind: crate::persona::PersonaKind::Stealth,
            category: crate::persona::PersonaCategory::Phone,
            oui_pool: vec!["apple".into()],
            mac_byte_pattern: None,
            hostname_template: "host".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: true,
            bt_name_template: template.into(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        }
    }

    #[test]
    fn persona_template_drives_bt_alias_when_active() {
        let cfg = cfg("generic", None);
        let p = persona_with_bt_template("{owner}s iphone");
        for _ in 0..16 {
            let alias = select_alias_with_persona(&cfg, Some(&p)).expect("ok");
            // Result must end with " iphone" and not be one of the
            // generic pool entries (those are the without-persona path).
            assert!(alias.ends_with(" iphone"), "got '{alias}'");
            assert!(!GENERIC_ALIASES.contains(&alias.as_str()));
        }
    }

    #[test]
    fn persona_unset_uses_generic_pool_path() {
        let cfg = cfg("generic", None);
        // No persona → behaviour is exactly what `select_alias` does.
        for _ in 0..16 {
            let alias = select_alias_with_persona(&cfg, None).expect("ok");
            assert!(GENERIC_ALIASES.contains(&alias.as_str()));
        }
    }

    #[test]
    fn pinned_alias_source_beats_persona_template() {
        // Operator's explicit pin always wins, even with a persona set.
        let cfg = cfg("pinned", Some("MyExplicitBT"));
        let p = persona_with_bt_template("{owner}s iphone");
        let alias = select_alias_with_persona(&cfg, Some(&p)).expect("ok");
        assert_eq!(alias, "MyExplicitBT");
    }

    #[test]
    fn persona_with_empty_template_falls_through_to_generic() {
        let cfg = cfg("generic", None);
        let p = persona_with_bt_template("   ");
        let alias = select_alias_with_persona(&cfg, Some(&p)).expect("ok");
        assert!(GENERIC_ALIASES.contains(&alias.as_str()));
    }

    #[test]
    fn unbiased_index_rejects_high_bytes_for_non_divisor_lens() {
        // 256 % 19 = 9, so the biased "high" range is bytes 247..=255 (the
        // last 9 of the 256 codepoints). Drive that range explicitly and
        // confirm we re-roll instead of producing biased output.
        let mut bytes: Vec<u8> = (247u8..=255).collect();
        bytes.push(7); // first non-rejected byte
        bytes.reverse();
        let idx = unbiased_index(19, || {
            bytes.pop().ok_or_else(|| anyhow!("ran out of bytes"))
        })
        .unwrap();
        assert_eq!(idx, 7);
    }

    #[test]
    fn unbiased_index_distribution_is_uniform_in_practice() {
        // Cycle through every byte 0..=255 once (a uniform stream) and
        // confirm each index in [0, 19) gets the same number of hits. With
        // pure `% 19` we would see 4 indices over-represented by ~5%.
        let len = 19;
        let span = 256 - (256 % len);
        let mut counts = vec![0usize; len];
        let mut feed: Vec<u8> = (0u8..=255).collect();
        feed.reverse();
        while !feed.is_empty() {
            let res = unbiased_index(len, || feed.pop().ok_or_else(|| anyhow!("end of stream")));
            match res {
                Ok(i) => counts[i] += 1,
                Err(_) => break, // ran out of bytes mid-rejection
            }
        }
        let expected_each = span / len; // 247 / 19 = 13
        for c in &counts {
            assert_eq!(
                *c, expected_each,
                "uniform stream should give uniform indices, got {counts:?}"
            );
        }
    }

    #[test]
    fn unbiased_index_rejects_empty_pool() {
        assert!(unbiased_index(0, || Ok(0)).is_err());
    }

    // === Issue #236: alias validation boundary tests ===

    #[test]
    fn validate_alias_rejects_empty() {
        assert!(validate_alias("").is_err());
    }

    #[test]
    fn validate_alias_rejects_oversized() {
        let long = "a".repeat(249);
        assert!(validate_alias(&long).is_err());
        let at_limit = "a".repeat(248);
        assert!(validate_alias(&at_limit).is_ok());
    }

    #[test]
    fn validate_alias_rejects_nul_and_control_chars() {
        assert!(validate_alias("foo\0bar").is_err());
        // ESC: classic terminal-injection primitive.
        assert!(validate_alias("\x1b[31mhi").is_err());
        // BEL: visible to anyone who cats journald output.
        assert!(validate_alias("\x07alert").is_err());
        // C1 CSI (0x9b) — encoded as U+009B in UTF-8.
        assert!(validate_alias("\u{009b}red").is_err());
    }

    #[test]
    fn validate_alias_rejects_bidi_override_chars() {
        // RLO — flips display order; classic homoglyph primitive.
        assert!(validate_alias("file\u{202e}txt.exe").is_err());
        // LRM, RLM, LRE, RLE, PDF, LRO, LRI, RLI, FSI, PDI — all rejected.
        for cp in [0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x2066, 0x2067, 0x2068, 0x2069] {
            let s = format!("a{}b", char::from_u32(cp).unwrap());
            assert!(
                validate_alias(&s).is_err(),
                "U+{cp:04X} should be rejected"
            );
        }
    }

    #[test]
    fn validate_alias_accepts_printable_ascii_and_unicode() {
        assert!(validate_alias("My iPhone").is_ok());
        assert!(validate_alias("Café BT").is_ok()); // legitimate non-ASCII
        assert!(validate_alias("Adapter 12").is_ok());
    }

    /// Issue #236: a hostile pinned alias is rejected at `select_alias`
    /// time; the bad bytes never reach BlueZ.
    #[test]
    fn select_alias_rejects_hostile_pinned() {
        let bad = cfg("pinned", Some("\x1b[2J\x1b[31mPWNED\x1b[0m"));
        assert!(select_alias(&bad).is_err());
    }

    /// Issue #236: a persona whose `bt_name_template` renders to a
    /// hostile string is rejected at `select_alias_with_persona` time.
    /// (No template variable currently injects controls — this guards
    /// against a future variable that might.)
    #[test]
    fn select_alias_with_persona_rejects_hostile_template_render() {
        let cfg = cfg("generic", None);
        // Use a literal-only template so the renderer doesn't need a
        // variable lookup; the rendered string contains a control byte.
        let p = persona_with_bt_template("BT\x1b[31m");
        assert!(select_alias_with_persona(&cfg, Some(&p)).is_err());
    }
}
