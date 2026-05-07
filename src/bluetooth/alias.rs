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
    match cfg.alias_source.as_str() {
        "pinned" => cfg
            .pinned_alias
            .clone()
            .ok_or_else(|| anyhow!("alias_source = 'pinned' but pinned_alias is unset")),
        "generic" => generic(),
        other => Err(anyhow!(
            "unknown alias_source '{other}'; expected 'generic' or 'pinned'"
        )),
    }
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
        return cfg
            .pinned_alias
            .clone()
            .ok_or_else(|| anyhow!("alias_source = 'pinned' but pinned_alias is unset"));
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
}
