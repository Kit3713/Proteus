// SPDX-License-Identifier: GPL-3.0-or-later

//! Device-persona schema for Milestone 2 of the v0.3 roadmap.
//!
//! A persona is a coherent set of fingerprint-shaped values that the apply
//! and rotate paths consult when shaping every identifier Proteus already
//! controls — MAC OUI choice, hostname template, DHCP option content, IPv6
//! traits, mDNS posture, RF TX-power, sysctl knobs. Two flavours coexist
//! in the same schema and the same on-disk representation:
//!
//! - `kind = "stealth"` — cover-identity goal. Every marker mimics a
//!   specific real device (`iphone-15`, `macbook-air-m3`, `samsung-tv-2024`).
//!   The user *looks like* that device to passive observers.
//! - `kind = "randomizer"` — anonymity goal. Same schema but `oui_pool`
//!   is broad and `rotate_cadence` is set; rotation drives the user into
//!   noise rather than mimicking a single device. The six built-in
//!   `Profile` baselines (`off`/`min`/`low`/`med`/`high`/`agr`) get
//!   identical-content randomizer mirrors so they show up alongside any
//!   user-authored randomizer recipes in `proteus persona list`.
//!
//! ## Scope discipline
//!
//! Persona values shape only what Proteus already controls. Nothing here
//! touches TLS, browser, or app-layer fingerprints — those are explicitly
//! out of scope per the threat model. See `wiki/personas.md` for the
//! field-by-field walkthrough and the verification checklist.
//!
//! ## Skeleton scope (this PR)
//!
//! This module ships the schema, the embedded built-in catalogue, the
//! loader, the validator, and the `proteus persona ...` CLI. The
//! integration with the apply/rotate paths (MAC OUI shaping, hostname
//! rendering from templates, DHCP fingerprint write) is deliberately
//! deferred to a follow-up — see roadmap Milestone 2 "Integration"
//! bullets. The schema is designed so the follow-up touches consumers,
//! not this module.

use serde::{Deserialize, Serialize};

pub mod load;

/// Top-level persona record. Mirrors `data/personas/<id>.toml` 1:1 via
/// serde; user-authored personas under `/etc/proteus/personas/<id>.toml`
/// use the same struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Persona {
    /// Stable kebab-case identifier. Must match the file stem.
    pub id: String,
    /// Human-readable name shown in `proteus persona list`.
    pub display_name: String,
    /// Stealth (cover-identity) or Randomizer (anonymity).
    pub kind: PersonaKind,
    /// Device family. Only meaningful for `Stealth`; randomizers set `Generic`.
    #[serde(default)]
    pub category: PersonaCategory,
    /// Vendor tokens (`apple`, `intel`, ...) or literal `aa:bb:cc` prefixes.
    /// The MAC generator (`src/mac/generator.rs`) consumes this list.
    #[serde(default)]
    pub oui_pool: Vec<String>,
    /// Optional shape for the trailing 3 bytes of the MAC. Free-form for
    /// now; the apply path will define the wildcard syntax in the
    /// integration follow-up (see roadmap Milestone 2 "Integration").
    #[serde(default)]
    pub mac_byte_pattern: Option<String>,
    /// Hostname template with `{n}` (digit), `{owner}` (first-name pool),
    /// and any persona-specific tokens. Rendered against
    /// `data/hostname-wordlist.txt` plus the persona's own pools.
    pub hostname_template: String,
    /// DHCP fingerprint values written instead of suppressed. The existing
    /// suppression path (issue #...; see `src/commands/dhcp.rs`) becomes
    /// "set to persona values" once integration lands.
    #[serde(default)]
    pub dhcp_fingerprint: DhcpFingerprint,
    /// TCP/IP stack knobs that contribute to OS fingerprinting.
    #[serde(default)]
    pub tcp_stack: TcpStackProfile,
    /// IPv6 SLAAC behaviour and ND traits.
    #[serde(default)]
    pub ipv6_traits: Ipv6Traits,
    /// Whether the persona advertises mDNS at all. Stealth personas for
    /// chatty devices (Apple, printers, TVs) leave this on; quiet
    /// personas (laptops in stealth mode) turn it off.
    #[serde(default)]
    pub mdns_advertise: bool,
    /// Bluetooth alias template; same token set as `hostname_template`.
    #[serde(default)]
    pub bt_name_template: String,
    /// RF surface (TX power, scan style).
    #[serde(default)]
    pub rf_traits: RfTraits,
    /// Rotation cadence (e.g. `"30m"`). `None` for stealth personas; the
    /// six randomizer mirrors of the existing `Profile` slider all set it.
    #[serde(default)]
    pub rotate_cadence: Option<String>,
    /// Free-form notes shown by `proteus persona show`. Author guidance,
    /// known limitations, references to source devices for audit trails.
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersonaKind {
    Stealth,
    Randomizer,
}

impl PersonaKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stealth => "stealth",
            Self::Randomizer => "randomizer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "stealth" => Some(Self::Stealth),
            "randomizer" => Some(Self::Randomizer),
            _ => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PersonaCategory {
    Phone,
    Laptop,
    Tablet,
    Tv,
    Iot,
    Router,
    Console,
    Printer,
    #[default]
    Generic,
}

impl PersonaCategory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Laptop => "laptop",
            Self::Tablet => "tablet",
            Self::Tv => "tv",
            Self::Iot => "iot",
            Self::Router => "router",
            Self::Console => "console",
            Self::Printer => "printer",
            Self::Generic => "generic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "phone" => Some(Self::Phone),
            "laptop" => Some(Self::Laptop),
            "tablet" => Some(Self::Tablet),
            "tv" => Some(Self::Tv),
            "iot" => Some(Self::Iot),
            "router" => Some(Self::Router),
            "console" => Some(Self::Console),
            "printer" => Some(Self::Printer),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }
}

/// DHCP option content the persona wants on the wire. The integration
/// follow-up routes these through `src/commands/dhcp.rs` so the option
/// path *sets* values from a persona instead of only suppressing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DhcpFingerprint {
    /// Option 60. Empty string means "do not send".
    pub vendor_class_identifier: String,
    /// Option 81. Empty string means "do not send".
    pub fqdn: String,
    /// Option 55, ordered. Empty means "use the kernel/dhclient default".
    pub parameter_request_list: Vec<u8>,
    /// Option 12. Empty means "use the rotated hostname from the template".
    pub host_name: String,
}

/// Sysctl + supplicant knobs that move the OS-fingerprint signature. The
/// concrete sysctl names are deliberately not encoded here; the apply
/// path translates these abstract traits to the actual `/proc/sys/net/...`
/// writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TcpStackProfile {
    /// `net.ipv4.tcp_window_scaling` shape. Real iOS/Android/Linux values.
    pub window_scale: u8,
    /// MSS (`net.ipv4.tcp_base_mss`). 1460 is the wired-Ethernet default.
    pub mss: u16,
    /// `net.ipv4.tcp_timestamps` (0 disables). iPhone leaves them on.
    pub tcp_timestamps: bool,
    /// `net.ipv4.tcp_sack` — modern stacks have it on.
    pub tcp_sack: bool,
    /// Initial TTL — Linux defaults to 64, Apple to 64, Windows to 128.
    pub default_ttl: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Ipv6Traits {
    /// `net.ipv6.conf.<iface>.use_tempaddr`.
    pub use_temp_addresses: bool,
    /// `addr_gen_mode` — `eui64`, `stable-privacy`, `random`.
    pub addr_gen_mode: String,
    /// Whether to advertise router-solicitation behaviour. iPhones do.
    pub send_rs: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RfTraits {
    /// dBm absolute. 0 means "leave at regulatory max".
    pub tx_power_dbm: u8,
    /// `passive` or `active` — affects supplicant scan behaviour.
    pub scan_style: String,
    /// Wi-Fi `power_save` setting (`on`/`off`/`auto`).
    pub power_save: String,
}

/// Lightweight summary used by `proteus persona list`. Avoids paying the
/// cost of fully deserialising every embedded persona when the user only
/// asked for a list.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
    pub kind: PersonaKind,
    pub category: PersonaCategory,
    pub source: PersonaSource,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersonaSource {
    /// Embedded under `data/personas/` and shipped with the binary.
    Builtin,
    /// User-authored under `/etc/proteus/personas/`. Shadows builtin on
    /// id collision.
    User,
}

impl PersonaSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_kind_round_trips_through_kebab() {
        // The `toml` crate (0.8, with default features off) refuses to
        // serialize a top-level struct whose only field is a unit-variant
        // enum — `UnsupportedType(None)`. That's a serializer quirk, not
        // a schema problem: the deserializer happily parses the kebab
        // form. Verify *that* round-trip directly, since it's what the
        // built-in catalogue's `kind = "..."` line actually exercises.
        #[derive(Deserialize, Debug, PartialEq)]
        struct W {
            kind: PersonaKind,
        }
        let parsed: W = toml::from_str("kind = \"randomizer\"").unwrap();
        assert_eq!(parsed.kind, PersonaKind::Randomizer);
        let parsed: W = toml::from_str("kind = \"stealth\"").unwrap();
        assert_eq!(parsed.kind, PersonaKind::Stealth);
        // Unknown kinds are rejected at parse time so a typo'd persona
        // file lands a wiki-linked error rather than silently degrading.
        assert!(toml::from_str::<W>("kind = \"chaos\"").is_err());
    }

    #[test]
    fn persona_category_parses_and_renders() {
        for c in [
            PersonaCategory::Phone,
            PersonaCategory::Laptop,
            PersonaCategory::Tablet,
            PersonaCategory::Tv,
            PersonaCategory::Iot,
            PersonaCategory::Router,
            PersonaCategory::Console,
            PersonaCategory::Printer,
            PersonaCategory::Generic,
        ] {
            assert_eq!(PersonaCategory::parse(c.name()), Some(c));
        }
        assert!(PersonaCategory::parse("nonsense").is_none());
    }

    #[test]
    fn persona_kind_parse_round_trips() {
        for k in [PersonaKind::Stealth, PersonaKind::Randomizer] {
            assert_eq!(PersonaKind::parse(k.name()), Some(k));
        }
        assert!(PersonaKind::parse("foo").is_none());
    }

    #[test]
    fn minimal_persona_round_trips_through_toml() {
        let p = Persona {
            id: "test-phone".into(),
            display_name: "Test Phone".into(),
            kind: PersonaKind::Stealth,
            category: PersonaCategory::Phone,
            oui_pool: vec!["apple".into()],
            mac_byte_pattern: None,
            hostname_template: "{owner}s-iPhone".into(),
            dhcp_fingerprint: DhcpFingerprint {
                vendor_class_identifier: "iPhone".into(),
                fqdn: String::new(),
                parameter_request_list: vec![1, 3, 6, 15, 119, 252],
                host_name: String::new(),
            },
            tcp_stack: TcpStackProfile {
                window_scale: 6,
                mss: 1460,
                tcp_timestamps: true,
                tcp_sack: true,
                default_ttl: 64,
            },
            ipv6_traits: Ipv6Traits {
                use_temp_addresses: true,
                addr_gen_mode: "stable-privacy".into(),
                send_rs: true,
            },
            mdns_advertise: true,
            bt_name_template: "{owner}s iPhone".into(),
            rf_traits: RfTraits {
                tx_power_dbm: 0,
                scan_style: "passive".into(),
                power_save: "auto".into(),
            },
            rotate_cadence: None,
            notes: "demo persona".into(),
        };
        let s = toml::to_string(&p).expect("serialize");
        let back: Persona = toml::from_str(&s).expect("deserialize");
        assert_eq!(back, p);
    }
}
