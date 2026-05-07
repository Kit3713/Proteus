// SPDX-License-Identifier: GPL-3.0-or-later

//! Functional configuration profiles.
//!
//! A profile is a coherent baseline of feature toggles tuned for a common
//! deployment scenario. Users select a profile by setting `profile = "..."`
//! at the top of `/etc/proteus/config.toml`; per-knob overrides in the
//! same file take precedence over the profile baseline. The `Off` profile
//! is a special panic-disable: it forces every feature off and ignores any
//! per-knob overrides while it is active. Switching back to a non-`Off`
//! profile restores the previously-set overrides because they remain in
//! the file untouched.
//!
//! The profile baseline is encoded as a complete `Config`. Resolution
//! starts from the profile baseline and overlays the user's per-knob
//! overrides on top; see `RawConfig::resolve` in `config.rs`.

use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// Hard kill: every feature disabled. Per-knob overrides are ignored
    /// while the profile is `Off`. Overrides are preserved on disk and
    /// take effect again the moment the profile is switched back to a
    /// non-`Off` value.
    Off,
    /// Trusted home LAN baseline. Rotation and discovery silencing are
    /// off. Suitable for a system that should appear stable on its own
    /// network and does not need to defend against passive observation.
    Min,
    /// Privacy-curious user on a home network. Scheduled MAC and hostname
    /// rotation, IPv6 stable-privacy, DHCP option suppression, the
    /// non-breaking subset of stack hardening. No discovery silencing,
    /// no TX power reduction.
    Low,
    /// Public Wi-Fi default. Adds discovery silencing of mDNS and LLMNR
    /// on top of `Low`. Recommended baseline for daily use on
    /// coffee-shop, hotel, and airport networks.
    #[default]
    Med,
    /// Hostile-network posture. Adds opt-in TX power reduction so the
    /// passive-capture radius shrinks, plus all of the `Med` baselines.
    /// SSDP/WSD blocks remain off so KDE Connect and Windows printer
    /// discovery still work.
    High,
    /// Conference, border, or actively adversarial environment. Enables
    /// every breaking knob: SSDP/WSD blocks, anonymous outer identity for
    /// 802.1X, gratuitous ARP suppression, and per-visit MAC rotation
    /// for known captive portals. The risk-warning banner from
    /// `proteus apply` lists each breaking knob so the operator knows
    /// what to expect.
    Agr,
}

impl Profile {
    /// Parse the kebab-case CLI / config name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Profile::Off),
            "min" => Some(Profile::Min),
            "low" => Some(Profile::Low),
            "med" => Some(Profile::Med),
            "high" => Some(Profile::High),
            "agr" => Some(Profile::Agr),
            _ => None,
        }
    }

    /// Stable kebab-case name used in TOML and CLI output.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Off => "off",
            Profile::Min => "min",
            Profile::Low => "low",
            Profile::Med => "med",
            Profile::High => "high",
            Profile::Agr => "agr",
        }
    }

    /// Human-readable summary suitable for `proteus config show` and
    /// `proteus wiki profiles`.
    pub fn description(self) -> &'static str {
        match self {
            Profile::Off => "all features disabled; per-knob overrides ignored",
            Profile::Min => "trusted home LAN; no rotation, no discovery work",
            Profile::Low => "privacy-curious home; rotation + DHCP/IPv6/stack, no breakage",
            Profile::Med => "public Wi-Fi default; adds mDNS/LLMNR silencing",
            Profile::High => "hostile network; adds TX power reduction",
            Profile::Agr => "conference/border; every breaking knob enabled",
        }
    }

    /// Every supported profile, listed in increasing aggressiveness order.
    pub fn all() -> &'static [Profile] {
        &[
            Profile::Off,
            Profile::Min,
            Profile::Low,
            Profile::Med,
            Profile::High,
            Profile::Agr,
        ]
    }

    /// Complete baseline `Config` for this profile. Every overrideable
    /// boolean is set to its profile-specific value; non-overrideable
    /// fields (intervals, modes, paths) take the same defaults that the
    /// individual sub-config `Default` impls provide.
    ///
    /// `Profile::Off` returns an all-disabled baseline. Resolution in
    /// `RawConfig::resolve` short-circuits per-knob overrides while the
    /// profile is `Off`, so the baseline returned here is what the
    /// system will actually do.
    pub fn baseline(self) -> Config {
        // Start from the per-section Default impls (which define all the
        // non-bool defaults like `rotation_interval = "2h"`), then
        // override the profile-affected bool toggles and timer cadences.
        let mut cfg = Config::structural_default();
        cfg.profile = self;
        apply_bools(&mut cfg, self);
        apply_timer_intervals(&mut cfg, self);
        cfg
    }
}

/// Set every profile-affected boolean on `cfg` according to `profile`.
fn apply_bools(cfg: &mut Config, profile: Profile) {
    let f = bool_baseline(profile);
    cfg.mac.enabled = f.mac_enabled;
    cfg.bluetooth.enabled = f.bluetooth_enabled;
    cfg.bluetooth.generic_alias = f.bluetooth_generic_alias;
    cfg.bluetooth.discoverable = f.bluetooth_discoverable;
    cfg.bluetooth.ble_rpa = f.bluetooth_ble_rpa;
    cfg.hostname.enabled = f.hostname_enabled;
    cfg.hostname.rotate_with_mac = f.hostname_rotate_with_mac;
    cfg.dns.strip_edns_client_subnet = f.dns_strip_edns_client_subnet;
    cfg.discovery.mdns_silence = f.discovery_mdns_silence;
    cfg.discovery.llmnr_silence = f.discovery_llmnr_silence;
    cfg.discovery.ssdp_block = f.discovery_ssdp_block;
    cfg.discovery.wsd_block = f.discovery_wsd_block;
    // Resolved/NTP track the discovery silencing baseline: `Med` and above
    // turn off the resolved mDNS/LLMNR responders, and timesyncd
    // normalisation comes online with `Low`. `Off` / `Min` short-circuit
    // both via the all-off baseline below.
    cfg.resolved.mdns_off = f.discovery_mdns_silence;
    cfg.resolved.llmnr_off = f.discovery_llmnr_silence;
    cfg.ntp.enabled = f.ntp_enabled;
    cfg.ipv6.enabled = f.ipv6_enabled;
    cfg.ipv6.use_temp_addresses = f.ipv6_use_temp_addresses;
    cfg.ipv6.ndp_hardening = f.ipv6_ndp_hardening;
    cfg.enterprise_wifi.anonymous_outer_identity = f.enterprise_wifi_anonymous_outer_identity;
    cfg.stack.tcp_timestamps_off = f.stack_tcp_timestamps_off;
    cfg.stack.icmpv6_hardening = f.stack_icmpv6_hardening;
    cfg.stack.suppress_gratuitous_arp = f.stack_suppress_gratuitous_arp;
    cfg.stack.icmp_info_replies_drop = f.stack_icmp_info_replies_drop;
    cfg.dhcp.enabled = f.dhcp_enabled;
    cfg.dhcp.suppress_hostname = f.dhcp_suppress_hostname;
    cfg.dhcp.suppress_vendor_class = f.dhcp_suppress_vendor_class;
    cfg.dhcp.rotate_client_id = f.dhcp_rotate_client_id;
    cfg.captive_portal.enabled = f.captive_portal_enabled;
    cfg.captive_portal.fresh_mac_per_visit = f.captive_portal_fresh_mac_per_visit;
    cfg.rf.tx_power_reduce = f.rf_tx_power_reduce;
}

/// Flat record of every profile-affected boolean. Each profile defines
/// one of these; `apply_bools` projects it onto the structured `Config`.
struct BoolBaseline {
    mac_enabled: bool,
    bluetooth_enabled: bool,
    bluetooth_generic_alias: bool,
    bluetooth_discoverable: bool,
    bluetooth_ble_rpa: bool,
    hostname_enabled: bool,
    hostname_rotate_with_mac: bool,
    dns_strip_edns_client_subnet: bool,
    discovery_mdns_silence: bool,
    discovery_llmnr_silence: bool,
    discovery_ssdp_block: bool,
    discovery_wsd_block: bool,
    ipv6_enabled: bool,
    ipv6_use_temp_addresses: bool,
    ipv6_ndp_hardening: bool,
    enterprise_wifi_anonymous_outer_identity: bool,
    stack_tcp_timestamps_off: bool,
    stack_icmpv6_hardening: bool,
    stack_suppress_gratuitous_arp: bool,
    stack_icmp_info_replies_drop: bool,
    dhcp_enabled: bool,
    dhcp_suppress_hostname: bool,
    dhcp_suppress_vendor_class: bool,
    dhcp_rotate_client_id: bool,
    captive_portal_enabled: bool,
    captive_portal_fresh_mac_per_visit: bool,
    rf_tx_power_reduce: bool,
    ntp_enabled: bool,
}

const ALL_OFF: BoolBaseline = BoolBaseline {
    mac_enabled: false,
    bluetooth_enabled: false,
    bluetooth_generic_alias: false,
    bluetooth_discoverable: false,
    bluetooth_ble_rpa: false,
    hostname_enabled: false,
    hostname_rotate_with_mac: false,
    dns_strip_edns_client_subnet: false,
    discovery_mdns_silence: false,
    discovery_llmnr_silence: false,
    discovery_ssdp_block: false,
    discovery_wsd_block: false,
    ipv6_enabled: false,
    ipv6_use_temp_addresses: false,
    ipv6_ndp_hardening: false,
    enterprise_wifi_anonymous_outer_identity: false,
    stack_tcp_timestamps_off: false,
    stack_icmpv6_hardening: false,
    stack_suppress_gratuitous_arp: false,
    stack_icmp_info_replies_drop: false,
    dhcp_enabled: false,
    dhcp_suppress_hostname: false,
    dhcp_suppress_vendor_class: false,
    dhcp_rotate_client_id: false,
    captive_portal_enabled: false,
    captive_portal_fresh_mac_per_visit: false,
    rf_tx_power_reduce: false,
    ntp_enabled: false,
};

const LOW: BoolBaseline = BoolBaseline {
    mac_enabled: true,
    bluetooth_enabled: true,
    bluetooth_generic_alias: true,
    bluetooth_discoverable: false,
    bluetooth_ble_rpa: true,
    hostname_enabled: true,
    hostname_rotate_with_mac: false,
    dns_strip_edns_client_subnet: true,
    discovery_mdns_silence: false,
    discovery_llmnr_silence: false,
    discovery_ssdp_block: false,
    discovery_wsd_block: false,
    ipv6_enabled: true,
    ipv6_use_temp_addresses: true,
    ipv6_ndp_hardening: true,
    enterprise_wifi_anonymous_outer_identity: false,
    stack_tcp_timestamps_off: true,
    stack_icmpv6_hardening: true,
    stack_suppress_gratuitous_arp: false,
    stack_icmp_info_replies_drop: true,
    dhcp_enabled: true,
    dhcp_suppress_hostname: true,
    dhcp_suppress_vendor_class: true,
    dhcp_rotate_client_id: true,
    captive_portal_enabled: true,
    captive_portal_fresh_mac_per_visit: false,
    rf_tx_power_reduce: false,
    ntp_enabled: true,
};

const MED: BoolBaseline = BoolBaseline {
    discovery_mdns_silence: true,
    discovery_llmnr_silence: true,
    ..LOW
};

const HIGH: BoolBaseline = BoolBaseline {
    rf_tx_power_reduce: true,
    ..MED
};

const AGR: BoolBaseline = BoolBaseline {
    discovery_ssdp_block: true,
    discovery_wsd_block: true,
    enterprise_wifi_anonymous_outer_identity: true,
    stack_suppress_gratuitous_arp: true,
    captive_portal_fresh_mac_per_visit: true,
    ..HIGH
};

fn bool_baseline(profile: Profile) -> BoolBaseline {
    match profile {
        Profile::Off => ALL_OFF,
        // Min is the same shape as Off for the bool toggles — every
        // feature off — but it preserves user customs (Off ignores them).
        Profile::Min => ALL_OFF,
        Profile::Low => LOW,
        Profile::Med => MED,
        Profile::High => HIGH,
        Profile::Agr => AGR,
    }
}

/// Sentinel value for `[timers.<name>].interval` meaning "do not run this
/// timer"; the apply orchestrator removes any drop-in and treats the timer
/// as disabled. See `wiki/profiles.md` for the per-profile cadence table.
pub const TIMER_NEVER: &str = "never";

/// Project a profile's timer-cadence baseline onto `cfg`. Mirrors
/// `apply_bools` but for the `[timers]` section. Cadences are strings to
/// keep the parsing path identical to `proteus timer set --interval`.
fn apply_timer_intervals(cfg: &mut Config, profile: Profile) {
    let t = timer_baseline(profile);
    cfg.timers.rotate.interval = t.rotate.into();
    cfg.timers.check.interval = t.check.into();
}

/// Per-profile timer cadence baseline. Strings flow straight into the
/// drop-in renderer via `timer::parse_interval`. `TIMER_NEVER` disables
/// the timer.
struct TimerBaseline {
    rotate: &'static str,
    check: &'static str,
}

const TIMER_OFF: TimerBaseline = TimerBaseline {
    rotate: TIMER_NEVER,
    check: TIMER_NEVER,
};

const TIMER_LOW: TimerBaseline = TimerBaseline {
    rotate: "4h",
    check: "5m",
};

const TIMER_MED: TimerBaseline = TimerBaseline {
    rotate: "2h",
    check: "5m",
};

const TIMER_HIGH: TimerBaseline = TimerBaseline {
    rotate: "30m",
    check: "2m",
};

const TIMER_AGR: TimerBaseline = TimerBaseline {
    rotate: "15m",
    check: "1m",
};

fn timer_baseline(profile: Profile) -> TimerBaseline {
    match profile {
        // Off and Min both disable scheduled rotation. Off short-circuits
        // resolution so user overrides are dropped; Min still respects
        // user overrides via `RawConfig::resolve`.
        Profile::Off | Profile::Min => TIMER_OFF,
        Profile::Low => TIMER_LOW,
        Profile::Med => TIMER_MED,
        Profile::High => TIMER_HIGH,
        Profile::Agr => TIMER_AGR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_for_every_variant() {
        for p in Profile::all() {
            let parsed = Profile::parse(p.name()).expect("name should parse back");
            assert_eq!(*p, parsed);
        }
    }

    #[test]
    fn parse_rejects_unknown_names() {
        assert!(Profile::parse("paranoid").is_none());
        assert!(Profile::parse("").is_none());
        assert!(Profile::parse("OFF").is_none());
    }

    #[test]
    fn off_disables_every_overrideable_bool() {
        let cfg = Profile::Off.baseline();
        assert!(!cfg.mac.enabled);
        assert!(!cfg.bluetooth.enabled);
        assert!(!cfg.hostname.enabled);
        assert!(!cfg.dns.strip_edns_client_subnet);
        assert!(!cfg.discovery.mdns_silence);
        assert!(!cfg.discovery.llmnr_silence);
        assert!(!cfg.discovery.ssdp_block);
        assert!(!cfg.discovery.wsd_block);
        assert!(!cfg.ipv6.enabled);
        assert!(!cfg.enterprise_wifi.anonymous_outer_identity);
        assert!(!cfg.stack.tcp_timestamps_off);
        assert!(!cfg.dhcp.enabled);
        assert!(!cfg.captive_portal.enabled);
    }

    #[test]
    fn aggressiveness_is_monotonic_for_core_features() {
        let order = [
            Profile::Min,
            Profile::Low,
            Profile::Med,
            Profile::High,
            Profile::Agr,
        ];
        for pair in order.windows(2) {
            let weaker = pair[0].baseline();
            let stronger = pair[1].baseline();
            for (name, w, s) in [
                ("mac.enabled", weaker.mac.enabled, stronger.mac.enabled),
                (
                    "hostname.enabled",
                    weaker.hostname.enabled,
                    stronger.hostname.enabled,
                ),
                ("ipv6.enabled", weaker.ipv6.enabled, stronger.ipv6.enabled),
                ("dhcp.enabled", weaker.dhcp.enabled, stronger.dhcp.enabled),
            ] {
                assert!(
                    !w || s,
                    "{name}: {:?} enabled it but {:?} did not",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn agr_enables_every_breaking_knob() {
        let cfg = Profile::Agr.baseline();
        assert!(cfg.discovery.ssdp_block);
        assert!(cfg.discovery.wsd_block);
        assert!(cfg.enterprise_wifi.anonymous_outer_identity);
        assert!(cfg.stack.suppress_gratuitous_arp);
        assert!(cfg.captive_portal.fresh_mac_per_visit);
    }

    #[test]
    fn med_includes_mdns_and_llmnr_silencing_but_not_breaking_blocks() {
        let cfg = Profile::Med.baseline();
        assert!(cfg.discovery.mdns_silence);
        assert!(cfg.discovery.llmnr_silence);
        assert!(!cfg.discovery.ssdp_block);
        assert!(!cfg.discovery.wsd_block);
    }

    #[test]
    fn low_does_not_silence_discovery() {
        let cfg = Profile::Low.baseline();
        assert!(!cfg.discovery.mdns_silence);
        assert!(!cfg.discovery.llmnr_silence);
    }

    #[test]
    fn min_disables_every_feature_like_off() {
        // Min == Off for the bool toggles. The semantic difference is
        // override-respect (Off ignores them, Min applies them). Both
        // baselines themselves are identical.
        let off = Profile::Off.baseline();
        let min = Profile::Min.baseline();
        assert_eq!(off.mac.enabled, min.mac.enabled);
        assert_eq!(off.hostname.enabled, min.hostname.enabled);
        assert_eq!(off.ipv6.enabled, min.ipv6.enabled);
    }

    #[test]
    fn high_and_agr_enable_rf_tx_power_reduction() {
        assert!(Profile::High.baseline().rf.tx_power_reduce);
        assert!(Profile::Agr.baseline().rf.tx_power_reduce);
    }

    #[test]
    fn quiet_profiles_do_not_enable_rf_tx_power_reduction() {
        assert!(!Profile::Off.baseline().rf.tx_power_reduce);
        assert!(!Profile::Min.baseline().rf.tx_power_reduce);
        assert!(!Profile::Low.baseline().rf.tx_power_reduce);
        assert!(!Profile::Med.baseline().rf.tx_power_reduce);
    }

    #[test]
    fn user_override_for_rf_tx_power_reduce_survives_med_to_high() {
        let toml_str = r#"
profile = "med"

[rf]
tx_power_reduce = false
"#;
        let raw: crate::config::RawConfig = toml::from_str(toml_str).unwrap();
        let mut switched = raw.clone();
        switched.profile = Some(Profile::High);
        let cfg = switched.resolve();
        assert!(
            !cfg.rf.tx_power_reduce,
            "explicit override must beat the High baseline"
        );
    }

    #[test]
    fn med_baseline_uses_two_hour_rotate_cadence() {
        let cfg = Profile::Med.baseline();
        assert_eq!(cfg.timers.rotate.interval, "2h");
        assert_eq!(cfg.timers.check.interval, "5m");
    }

    #[test]
    fn agr_baseline_uses_aggressive_timer_cadences() {
        let cfg = Profile::Agr.baseline();
        assert_eq!(cfg.timers.rotate.interval, "15m");
        assert_eq!(cfg.timers.check.interval, "1m");
    }

    #[test]
    fn high_baseline_bumps_rotate_to_thirty_minutes() {
        let cfg = Profile::High.baseline();
        assert_eq!(cfg.timers.rotate.interval, "30m");
        assert_eq!(cfg.timers.check.interval, "2m");
    }

    #[test]
    fn low_baseline_uses_relaxed_home_cadence() {
        let cfg = Profile::Low.baseline();
        assert_eq!(cfg.timers.rotate.interval, "4h");
        assert_eq!(cfg.timers.check.interval, "5m");
    }

    #[test]
    fn off_and_min_baselines_force_timers_to_never() {
        for p in [Profile::Off, Profile::Min] {
            let cfg = p.baseline();
            assert_eq!(cfg.timers.rotate.interval, TIMER_NEVER);
            assert_eq!(cfg.timers.check.interval, TIMER_NEVER);
        }
    }
}
