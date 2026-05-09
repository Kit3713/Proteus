// SPDX-License-Identifier: GPL-3.0-or-later

pub mod arp;
pub mod factory;
pub mod generator;
pub mod oui;
pub mod plan;
pub mod probe;

use std::fmt;
use std::str::FromStr;

pub use plan::{plan_pin, plan_rotate};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MacError {
    #[error("MAC address must have 6 octets (got {0})")]
    WrongOctetCount(usize),
    #[error("MAC address octet must be two hex chars (got '{0}')")]
    BadOctet(String),
    #[error("hex parse error: {0}")]
    Hex(#[from] std::num::ParseIntError),
    #[error("unsuitable MAC: must be unicast (multicast bit clear)")]
    Multicast,
    #[error("unsuitable MAC: all-zero")]
    AllZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    pub fn octets(&self) -> [u8; 6] {
        self.0
    }

    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0x01 != 0
    }

    pub fn is_locally_administered(&self) -> bool {
        self.0[0] & 0x02 != 0
    }

    pub fn is_all_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    pub fn validate_assignable(&self) -> Result<(), MacError> {
        if self.is_all_zero() {
            return Err(MacError::AllZero);
        }
        if self.is_multicast() {
            return Err(MacError::Multicast);
        }
        Ok(())
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let o = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }
}

impl FromStr for Mac {
    type Err = MacError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept colon, dash, or no separator (12 hex chars).
        // S10: strict canonical form — reject mixed separators
        // (e.g. `aa:bb-cc:dd-ee:ff`) so a typo doesn't get silently
        // normalised into a valid MAC. The check runs first so the
        // error names the offending shape, rather than landing as a
        // generic "WrongOctetCount" downstream.
        // Issue #284: drop any non-ASCII char during cleaning so byte-indexed
        // slicing below is safe. A multi-byte UTF-8 codepoint that satisfies
        // the old `*c != ':' && *c != '-'` filter would survive into `cleaned`,
        // make `cleaned.len()` (bytes) hit 12 with N<6 logical hex pairs, and
        // panic on `&cleaned[i*2..i*2+2]` at a non-char-boundary index.
        if s.contains(':') && s.contains('-') {
            return Err(MacError::BadOctet(s.to_string()));
        }
        let cleaned: String = s
            .chars()
            .filter(|c| *c != ':' && *c != '-' && c.is_ascii())
            .collect();
        if cleaned.len() != 12 {
            // Re-split using the original separators to give a useful error count.
            let parts: Vec<&str> = if s.contains(':') {
                s.split(':').collect()
            } else if s.contains('-') {
                s.split('-').collect()
            } else {
                vec![s]
            };
            if parts.len() != 6 {
                return Err(MacError::WrongOctetCount(parts.len()));
            }
            return Err(MacError::BadOctet(s.to_string()));
        }
        let mut out = [0u8; 6];
        for (i, byte) in out.iter_mut().enumerate() {
            let chunk = &cleaned[i * 2..i * 2 + 2];
            *byte = u8::from_str_radix(chunk, 16)?;
        }
        Ok(Mac(out))
    }
}

impl Serialize for Mac {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Mac {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: String = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colon_form() {
        let m: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        assert_eq!(m.octets(), [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    }

    #[test]
    fn parses_dash_form() {
        let m: Mac = "AA-BB-CC-DD-EE-FF".parse().unwrap();
        assert_eq!(m.octets(), [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    }

    #[test]
    fn formats_lowercase_colon() {
        let m = Mac::new([0xaa, 0xbb, 0xcc, 0x12, 0x34, 0x56]);
        assert_eq!(m.to_string(), "aa:bb:cc:12:34:56");
    }

    #[test]
    fn rejects_wrong_octet_count() {
        let r: Result<Mac, _> = "aa:bb:cc".parse();
        assert!(matches!(r, Err(MacError::WrongOctetCount(3))));
    }

    #[test]
    fn rejects_bad_hex() {
        let r: Result<Mac, _> = "zz:bb:cc:dd:ee:ff".parse();
        assert!(matches!(r, Err(MacError::Hex(_))));
    }

    /// Issue #284: a string whose cleaned byte length lands on 12 because of
    /// multi-byte UTF-8 chars must NOT panic — old code byte-indexed the
    /// `cleaned` String at i*2..i*2+2 and tripped on a non-char-boundary.
    #[test]
    fn rejects_multibyte_utf8_input_without_panic() {
        // "µ" is 2 bytes (0xC2 0xB5). 6 of them = 12 bytes but only 6 chars,
        // not 12 hex digits — must reject cleanly, not panic.
        let r: Result<Mac, _> = "µµµµµµ".parse();
        assert!(r.is_err(), "multi-byte UTF-8 input must error, not panic");

        // A mixed input that survives the old filter and reaches 12 bytes.
        let r: Result<Mac, _> = "µµaabbcc".parse();
        assert!(r.is_err());
    }

    #[test]
    fn detects_multicast_bit() {
        let m = Mac::new([0x01, 0, 0, 0, 0, 0]);
        assert!(m.is_multicast());
        assert!(matches!(m.validate_assignable(), Err(MacError::Multicast)));
    }

    #[test]
    fn detects_locally_administered_bit() {
        let m = Mac::new([0x02, 0, 0, 0, 0, 1]);
        assert!(m.is_locally_administered());
        assert!(!m.is_multicast());
    }

    #[test]
    fn detects_all_zero() {
        let m = Mac::new([0; 6]);
        assert!(m.is_all_zero());
        assert!(matches!(m.validate_assignable(), Err(MacError::AllZero)));
    }

    #[test]
    fn assignable_unicast_passes() {
        let m = Mac::new([0x00, 0x1B, 0x21, 0x12, 0x34, 0x56]);
        assert!(m.validate_assignable().is_ok());
    }

    /// S10: strict separator parser. Mixed `:`/`-` in the same input
    /// is indistinguishable from a typo and silently accepting it
    /// removes a useful operator-error signal.
    #[test]
    fn rejects_mixed_separators_strict_canonical_form() {
        for s in [
            "aa:bb-cc:dd-ee:ff",
            "aa-bb:cc-dd:ee-ff",
            "aa:bb:cc-dd:ee:ff",
            "aa-bb-cc:dd-ee-ff",
        ] {
            let r: Result<Mac, _> = s.parse();
            assert!(
                r.is_err(),
                "mixed-separator input '{s}' must be rejected, got {:?}",
                r
            );
        }
    }

    /// S10: but a single canonical separator must still parse — pin
    /// the contract so the strict check doesn't become "reject every
    /// MAC with a separator".
    #[test]
    fn accepts_single_separator_canonical_forms() {
        assert!("aa:bb:cc:dd:ee:ff".parse::<Mac>().is_ok());
        assert!("aa-bb-cc-dd-ee-ff".parse::<Mac>().is_ok());
        assert!("aabbccddeeff".parse::<Mac>().is_ok());
    }
}
