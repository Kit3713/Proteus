// SPDX-License-Identifier: GPL-3.0-or-later

// Small representative slice of each vendor's OUI assignments. The full IEEE
// OUI registry is enormous; we only need plausible-looking prefixes for
// persona shaping (Milestone 2 "Integration"). The set was chosen to cover
// every persona token referenced by `data/personas/*.toml` plus the
// generic randomizer tokens.

pub type OuiPrefix = [u8; 3];

pub const APPLE: &[OuiPrefix] = &[
    [0x00, 0x03, 0x93],
    [0x00, 0x05, 0x02],
    [0x00, 0x16, 0xCB],
    [0x00, 0x1B, 0x63],
    [0x00, 0x25, 0x00],
    [0x00, 0x50, 0xE4],
    [0x3C, 0x07, 0x54],
    [0xA4, 0x83, 0xE7],
];

pub const INTEL: &[OuiPrefix] = &[
    [0x00, 0x13, 0xE8],
    [0x00, 0x1B, 0x21],
    [0x00, 0x1F, 0x3B],
    [0x00, 0x22, 0xFB],
    [0x00, 0x27, 0x10],
    [0x34, 0x13, 0xE8],
    [0xA0, 0x88, 0xB4],
    [0xDC, 0x53, 0x60],
];

pub const SAMSUNG: &[OuiPrefix] = &[
    [0x00, 0x12, 0xFB],
    [0x00, 0x18, 0xAF],
    [0x00, 0x21, 0x19],
    [0x00, 0x24, 0x54],
    [0x08, 0xFC, 0x88],
    [0x14, 0xBB, 0x6E],
    [0x5C, 0x49, 0x7D],
    [0xCC, 0x07, 0xAB],
];

pub const DELL: &[OuiPrefix] = &[
    [0x00, 0x14, 0x22],
    [0x00, 0x1A, 0xA0],
    [0x00, 0x1D, 0x09],
    [0x00, 0x21, 0x9B],
    [0x00, 0x24, 0xE8],
    [0x18, 0x03, 0x73],
    [0x84, 0x8F, 0x69],
    [0xB8, 0xCA, 0x3A],
];

// === Personas added in Milestone 2 follow-up =============================
//
// Each list below was picked from a small sample of the IEEE registry — not
// the full vendor allocation, just enough plausible prefixes that a passive
// observer running `nmap -O` / fingerbank / p0f will accept the cover.

pub const GOOGLE: &[OuiPrefix] = &[
    [0x3C, 0x5A, 0xB4],
    [0x70, 0x3A, 0xCB],
    [0x94, 0xEB, 0x2C],
    [0xF4, 0xF5, 0xE8],
    [0xF8, 0x8F, 0xCA],
];

pub const MICROSOFT: &[OuiPrefix] = &[
    [0x00, 0x03, 0xFF],
    [0x00, 0x12, 0x5A],
    [0x00, 0x15, 0x5D],
    [0x00, 0x17, 0xFA],
    [0x7C, 0x1E, 0x52],
    [0xC8, 0x3F, 0x26],
];

pub const LG: &[OuiPrefix] = &[
    [0x00, 0x1C, 0x62],
    [0x00, 0x1E, 0x75],
    [0x10, 0xF1, 0xF2],
    [0x3C, 0xCD, 0x93],
    [0x88, 0xC9, 0xD0],
];

pub const TPLINK: &[OuiPrefix] = &[
    [0x14, 0xCC, 0x20],
    [0x50, 0xC7, 0xBF],
    [0x60, 0xE3, 0x27],
    [0xC4, 0xE9, 0x84],
    [0xEC, 0x08, 0x6B],
];

pub const ASUS: &[OuiPrefix] = &[
    [0x00, 0x1A, 0x92],
    [0x00, 0x22, 0x15],
    [0x04, 0x92, 0x26],
    [0x2C, 0x56, 0xDC],
    [0xAC, 0x9E, 0x17],
];

pub const ROKU: &[OuiPrefix] = &[
    [0x08, 0x05, 0x81],
    [0xB0, 0xA7, 0x37],
    [0xB8, 0x3E, 0x59],
    [0xBC, 0xD7, 0xD4],
    [0xCC, 0x6D, 0xA0],
];

pub const AMAZON: &[OuiPrefix] = &[
    [0x00, 0xFC, 0x8B],
    [0x40, 0xB4, 0xCD],
    [0x68, 0x37, 0xE9],
    [0x84, 0xD6, 0xD0],
    [0xF0, 0x27, 0x2D],
];

/// Generic IoT pool for personas that don't bind to a single vendor (e.g.
/// no-name camera, generic HVAC controller). A blend across consumer
/// vendors keeps the cover plausible without claiming a specific brand.
pub const IOT_GENERIC: &[OuiPrefix] = &[
    [0x18, 0xB4, 0x30], // Nest Labs
    [0x44, 0x65, 0x0D], // Amazon
    [0x68, 0xC6, 0x3A], // Espressif
    [0xB0, 0xC5, 0x54], // D-Link
    [0xCC, 0x50, 0xE3], // Espressif
    [0xEC, 0xFA, 0xBC], // Espressif
];

/// Sony — covers `playstation-5` and other Sony-branded personas.
pub const SONY: &[OuiPrefix] = &[
    [0x00, 0x13, 0xA9],
    [0x00, 0x19, 0xC5],
    [0x00, 0x24, 0xBE],
    [0x70, 0x9E, 0x29],
    [0xFC, 0x0F, 0xE6],
];

/// Nintendo — covers `nintendo-switch`.
pub const NINTENDO: &[OuiPrefix] = &[
    [0x00, 0x16, 0x56],
    [0x00, 0x17, 0xAB],
    [0x00, 0x19, 0x1D],
    [0x00, 0x1A, 0xE9],
    [0x18, 0x2A, 0x7B],
];

/// HP — covers `printer-generic-hp`.
pub const HP: &[OuiPrefix] = &[
    [0x00, 0x1F, 0x29],
    [0x3C, 0xD9, 0x2B],
    [0x68, 0xB5, 0x99],
    [0x80, 0xCE, 0x62],
    [0xB4, 0x99, 0xBA],
];

/// Espressif — covers `iot-generic` and any ESP32/ESP8266-flavoured persona.
/// IEEE OUI allocations to Espressif Inc. (canonical, widely-published).
pub const ESPRESSIF: &[OuiPrefix] = &[
    [0x24, 0x0A, 0xC4],
    [0x24, 0x62, 0xAB],
    [0x30, 0xAE, 0xA4],
    [0x68, 0xC6, 0x3A],
    [0x7C, 0x9E, 0xBD],
    [0x94, 0xB9, 0x7E],
    [0xA0, 0x20, 0xA6],
    [0xCC, 0x50, 0xE3],
    [0xEC, 0xFA, 0xBC],
];

/// Realtek — covers `iot-generic` and any Realtek-chipset-flavoured persona.
/// IEEE OUI allocations to Realtek Semiconductor Corp. `52:54:00` is the
/// QEMU/KVM virtual-NIC range Realtek originally registered, kept because
/// virtual IoT gear ships with it routinely.
pub const REALTEK: &[OuiPrefix] = &[
    [0x00, 0x13, 0x33],
    [0x00, 0xE0, 0x4C],
    [0x00, 0xE0, 0x7D],
    [0x52, 0x54, 0x00],
    [0x00, 0x21, 0xCC],
    [0x10, 0xEC, 0x4F],
    [0xB0, 0xC0, 0x90],
];

#[derive(Debug, Clone, Copy)]
pub enum Vendor {
    Apple,
    Intel,
    Samsung,
    Dell,
    Google,
    Microsoft,
    Lg,
    TpLink,
    Asus,
    Roku,
    Amazon,
    Sony,
    Nintendo,
    Hp,
    Espressif,
    Realtek,
    IotGeneric,
    LocallyAdministered,
}

impl Vendor {
    pub fn from_pool_token(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "apple" => Some(Self::Apple),
            "intel" => Some(Self::Intel),
            "samsung" => Some(Self::Samsung),
            "dell" => Some(Self::Dell),
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            "lg" => Some(Self::Lg),
            "tplink" | "tp-link" => Some(Self::TpLink),
            "asus" => Some(Self::Asus),
            "roku" => Some(Self::Roku),
            "amazon" => Some(Self::Amazon),
            "sony" => Some(Self::Sony),
            "nintendo" => Some(Self::Nintendo),
            "hp" | "hewlett-packard" => Some(Self::Hp),
            "espressif" => Some(Self::Espressif),
            "realtek" => Some(Self::Realtek),
            "iot-generic" | "generic-iot" => Some(Self::IotGeneric),
            "random-locally-administered" | "laa" | "locally-administered" => {
                Some(Self::LocallyAdministered)
            }
            _ => None,
        }
    }

    pub fn prefixes(self) -> Option<&'static [OuiPrefix]> {
        match self {
            Self::Apple => Some(APPLE),
            Self::Intel => Some(INTEL),
            Self::Samsung => Some(SAMSUNG),
            Self::Dell => Some(DELL),
            Self::Google => Some(GOOGLE),
            Self::Microsoft => Some(MICROSOFT),
            Self::Lg => Some(LG),
            Self::TpLink => Some(TPLINK),
            Self::Asus => Some(ASUS),
            Self::Roku => Some(ROKU),
            Self::Amazon => Some(AMAZON),
            Self::Sony => Some(SONY),
            Self::Nintendo => Some(NINTENDO),
            Self::Hp => Some(HP),
            Self::Espressif => Some(ESPRESSIF),
            Self::Realtek => Some(REALTEK),
            Self::IotGeneric => Some(IOT_GENERIC),
            Self::LocallyAdministered => None,
        }
    }
}

/// Resolve a list of persona pool tokens (as written in `oui_pool` in
/// `data/personas/*.toml`) into the OUI prefixes those tokens cover.
/// Unknown tokens are skipped with a `tracing::warn!` so a typo in a
/// hand-edited persona doesn't fail the apply — the pool may still
/// contain other valid tokens. Roadmap Milestone 2 "Integration".
///
/// Literal `aa:bb:cc` prefixes parse via [`parse_literal_prefix`]. The
/// caller should fall back to `[mac] oui_pool` when this returns an
/// empty vec (every token was unknown / unparseable).
pub fn resolve_vendor_tokens(tokens: &[String]) -> Vec<OuiPrefix> {
    let mut out: Vec<OuiPrefix> = Vec::new();
    for tok in tokens {
        if let Some(v) = Vendor::from_pool_token(tok)
            && let Some(prefs) = v.prefixes()
        {
            out.extend_from_slice(prefs);
            continue;
        }
        if let Some(p) = parse_literal_prefix(tok) {
            out.push(p);
            continue;
        }
        tracing::warn!(
            token = %tok,
            "ignoring unknown OUI pool token in persona; not a vendor name and not a 'aa:bb:cc' prefix"
        );
    }
    out
}

/// Parse a literal `aa:bb:cc` (or `aa-bb-cc`, `aabbcc`) prefix into an
/// `OuiPrefix`. Lowercase-tolerant. `None` when the input doesn't fit
/// the grammar — caller decides whether that's a warning or a hard error.
pub fn parse_literal_prefix(s: &str) -> Option<OuiPrefix> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if cleaned.len() != 6 {
        return None;
    }
    let mut out = [0u8; 3];
    for (i, byte) in cleaned.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(byte).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_tokens_parse() {
        assert!(matches!(
            Vendor::from_pool_token("apple"),
            Some(Vendor::Apple)
        ));
        assert!(matches!(
            Vendor::from_pool_token("INTEL"),
            Some(Vendor::Intel)
        ));
        assert!(matches!(
            Vendor::from_pool_token("random-locally-administered"),
            Some(Vendor::LocallyAdministered)
        ));
        assert!(Vendor::from_pool_token("nonsense").is_none());
    }

    #[test]
    fn vendor_prefix_lists_are_nonempty() {
        for v in [
            Vendor::Apple,
            Vendor::Intel,
            Vendor::Samsung,
            Vendor::Dell,
            Vendor::Google,
            Vendor::Microsoft,
            Vendor::Lg,
            Vendor::TpLink,
            Vendor::Asus,
            Vendor::Roku,
            Vendor::Amazon,
            Vendor::Sony,
            Vendor::Nintendo,
            Vendor::Hp,
            Vendor::Espressif,
            Vendor::Realtek,
            Vendor::IotGeneric,
        ] {
            let prefs = v.prefixes().unwrap();
            assert!(!prefs.is_empty(), "vendor {v:?} has no prefixes");
        }
        assert!(Vendor::LocallyAdministered.prefixes().is_none());
    }

    #[test]
    fn new_vendors_resolve_to_their_tables() {
        // Smoke-test every vendor added in Milestone 2 follow-up.
        for (tok, expected_first_byte) in [
            ("google", GOOGLE[0][0]),
            ("microsoft", MICROSOFT[0][0]),
            ("lg", LG[0][0]),
            ("tplink", TPLINK[0][0]),
            ("tp-link", TPLINK[0][0]),
            ("asus", ASUS[0][0]),
            ("roku", ROKU[0][0]),
            ("amazon", AMAZON[0][0]),
            ("sony", SONY[0][0]),
            ("nintendo", NINTENDO[0][0]),
            ("hp", HP[0][0]),
            ("espressif", ESPRESSIF[0][0]),
            ("realtek", REALTEK[0][0]),
            ("iot-generic", IOT_GENERIC[0][0]),
        ] {
            let v = Vendor::from_pool_token(tok)
                .unwrap_or_else(|| panic!("token '{tok}' must resolve"));
            let prefs = v.prefixes().expect("prefixes");
            assert_eq!(prefs[0][0], expected_first_byte);
        }
    }

    #[test]
    fn resolve_vendor_tokens_unions_pools_for_apple_and_google() {
        // Persona-style mixed pool: Apple + Google. Result must contain
        // every Apple prefix AND every Google prefix.
        let tokens = vec!["apple".to_string(), "google".to_string()];
        let out = resolve_vendor_tokens(&tokens);
        for p in APPLE {
            assert!(out.contains(p), "apple prefix {p:?} missing from union");
        }
        for p in GOOGLE {
            assert!(out.contains(p), "google prefix {p:?} missing from union");
        }
        assert_eq!(out.len(), APPLE.len() + GOOGLE.len());
    }

    #[test]
    fn resolve_vendor_tokens_skips_unknown_with_warn() {
        // Unknown token doesn't poison the resolution — known tokens
        // still produce their prefix set.
        let tokens = vec!["apple".to_string(), "nonsense".to_string()];
        let out = resolve_vendor_tokens(&tokens);
        for p in APPLE {
            assert!(out.contains(p));
        }
        assert_eq!(out.len(), APPLE.len());
    }

    #[test]
    fn resolve_vendor_tokens_accepts_literal_prefix() {
        // Persona may want a specific prefix not covered by the vendor
        // table. Literal `aa:bb:cc` form is honoured.
        let tokens = vec!["aa:bb:cc".to_string()];
        let out = resolve_vendor_tokens(&tokens);
        assert_eq!(out, vec![[0xAA, 0xBB, 0xCC]]);
    }

    #[test]
    fn parse_literal_prefix_accepts_three_formats() {
        assert_eq!(parse_literal_prefix("aa:bb:cc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_literal_prefix("aa-bb-cc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_literal_prefix("aabbcc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_literal_prefix("AA:BB:CC"), Some([0xAA, 0xBB, 0xCC]));
        // Wrong length / non-hex.
        assert_eq!(parse_literal_prefix("aa:bb"), None);
        assert_eq!(parse_literal_prefix("aabbccdd"), None);
        assert_eq!(parse_literal_prefix("zz:bb:cc"), None);
    }

    #[test]
    fn resolve_empty_token_list_yields_empty_vec() {
        let out = resolve_vendor_tokens(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn resolve_all_unknown_tokens_yields_empty_vec() {
        // The caller's contract is "fall back to global mac.oui_pool when
        // empty" — pin the empty-vec invariant.
        let tokens = vec!["unknown1".to_string(), "unknown2".to_string()];
        let out = resolve_vendor_tokens(&tokens);
        assert!(out.is_empty());
    }

    /// Stream 2 V11 follow-up: `iot-generic` carries `espressif` and
    /// `realtek` tokens that must now resolve to real OUI prefixes rather
    /// than degrading to LAA.
    #[test]
    fn espressif_and_realtek_tokens_resolve() {
        let esp = Vendor::from_pool_token("espressif").expect("espressif must resolve");
        let rt = Vendor::from_pool_token("realtek").expect("realtek must resolve");
        assert!(matches!(esp, Vendor::Espressif));
        assert!(matches!(rt, Vendor::Realtek));
        assert!(!esp.prefixes().unwrap().is_empty());
        assert!(!rt.prefixes().unwrap().is_empty());
    }

    /// Stream 2 V11 follow-up: the canonical `iot-generic` persona pool
    /// (`espressif`, `realtek`, `random-locally-administered`) must
    /// resolve to at least one prefix per non-LAA token with no unknown
    /// tokens dropped.
    #[test]
    fn iot_generic_persona_pool_resolves_cleanly() {
        let tokens = vec![
            "espressif".to_string(),
            "realtek".to_string(),
            "random-locally-administered".to_string(),
        ];
        let out = resolve_vendor_tokens(&tokens);
        // LAA contributes nothing — the other two each contribute their slice.
        assert_eq!(out.len(), ESPRESSIF.len() + REALTEK.len());
        for p in ESPRESSIF {
            assert!(out.contains(p), "espressif prefix {p:?} missing");
        }
        for p in REALTEK {
            assert!(out.contains(p), "realtek prefix {p:?} missing");
        }
    }
}
