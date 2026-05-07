// SPDX-License-Identifier: GPL-3.0-or-later

// Small representative slice of each vendor's OUI assignments. The full IEEE
// OUI registry is enormous; we only need plausible-looking prefixes.

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

#[derive(Debug, Clone, Copy)]
pub enum Vendor {
    Apple,
    Intel,
    Samsung,
    Dell,
    LocallyAdministered,
}

impl Vendor {
    pub fn from_pool_token(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "apple" => Some(Self::Apple),
            "intel" => Some(Self::Intel),
            "samsung" => Some(Self::Samsung),
            "dell" => Some(Self::Dell),
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
            Self::LocallyAdministered => None,
        }
    }
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
        for v in [Vendor::Apple, Vendor::Intel, Vendor::Samsung, Vendor::Dell] {
            let prefs = v.prefixes().unwrap();
            assert!(!prefs.is_empty(), "vendor {v:?} has no prefixes");
        }
        assert!(Vendor::LocallyAdministered.prefixes().is_none());
    }
}
