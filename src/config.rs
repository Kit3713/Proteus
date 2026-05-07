// SPDX-License-Identifier: GPL-3.0-or-later

//! On-disk configuration: profile-aware loading with per-knob overrides.
//!
//! The public `Config` struct is what every consumer in the codebase sees.
//! Bool fields are concrete `bool` so call sites stay simple. The TOML
//! file, by contrast, is parsed into the private `RawConfig` shape where
//! every field is `Option<T>`. Loading is a two-step process:
//!
//! 1. Read the file as `RawConfig`. `Option<T>` makes the difference
//!    between "user explicitly set this" and "user left it at the
//!    profile default" recoverable.
//! 2. Resolve the raw form by overlaying the user's explicit fields on
//!    top of the profile baseline (`Profile::baseline`).
//!
//! `Profile::Off` short-circuits resolution: it returns the all-disabled
//! baseline regardless of any per-knob overrides. The overrides remain
//! on disk, so switching back to a non-`Off` profile restores them.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::Profile;

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    /// Active profile. `Profile::Off` panic-disables every feature.
    pub profile: Profile,
    pub mac: MacConfig,
    pub bluetooth: BluetoothConfig,
    pub hostname: HostnameConfig,
    pub dns: DnsConfig,
    pub resolved: ResolvedConfig,
    pub ntp: NtpConfig,
    pub nft: NftConfig,
    pub discovery: DiscoveryConfig,
    pub probes: ProbesConfig,
    pub ipv6: Ipv6Config,
    pub enterprise_wifi: EnterpriseWifiConfig,
    pub stack: StackConfig,
    pub dhcp: DhcpConfig,
    pub captive_portal: CaptivePortalConfig,
    pub rf: RfConfig,
    pub timers: TimersConfig,
    /// Active persona (Milestone 2). Loader respects user-set values but
    /// the apply / rotate paths do **not** yet consume persona fields —
    /// integration with MAC OUI shaping, hostname rendering, and DHCP
    /// fingerprint write is the follow-up tracked in roadmap Milestone 2
    /// "Integration".
    pub persona: PersonaConfig,
    /// Backend selector (Milestone 1). `driver = "auto"` walks
    /// NM → networkd → raw at runtime; only the NM impl is fully
    /// wired in this PR.
    pub backend: BackendConfig,
    /// Per-SSID policy overrides (roadmap Milestone 3). Each entry under
    /// `[per_ssid."<ssid>"]` may override one or more knobs that the
    /// orchestrator looks up at NM `connection-up` time. Precedence is
    /// `per_ssid["X"]` > `[persona]` > `[profile]` baseline > config
    /// defaults — see `crate::per_ssid::resolve_for_ssid`. The keys are
    /// the literal SSID strings (case-sensitive); the values follow
    /// `PerSsidPolicy`. Integration with the NM connection-up dispatcher
    /// is the follow-up tracked in roadmap Milestone 3.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_ssid: BTreeMap<String, PerSsidPolicy>,
}

impl Default for Config {
    fn default() -> Self {
        Profile::default().baseline()
    }
}

impl Config {
    /// Load `path` as TOML, resolving profile + per-knob overrides. If the
    /// file is absent the default profile baseline is returned. Parse
    /// errors propagate as `Err`.
    pub fn default_or_loaded(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let raw: RawConfig =
                    toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
                Ok(raw.resolve())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Structural baseline with the per-section `Default` impl values for
    /// every non-profile-affected field. The bool toggles are placeholder
    /// values that `Profile::baseline` always overwrites — never call
    /// this directly.
    pub(crate) fn structural_default() -> Self {
        Config {
            profile: Profile::default(),
            mac: MacConfig::default(),
            bluetooth: BluetoothConfig::default(),
            hostname: HostnameConfig::default(),
            dns: DnsConfig::default(),
            resolved: ResolvedConfig::default(),
            ntp: NtpConfig::default(),
            nft: NftConfig::default(),
            discovery: DiscoveryConfig::default(),
            probes: ProbesConfig::default(),
            ipv6: Ipv6Config::default(),
            enterprise_wifi: EnterpriseWifiConfig::default(),
            stack: StackConfig::default(),
            dhcp: DhcpConfig::default(),
            captive_portal: CaptivePortalConfig::default(),
            rf: RfConfig::default(),
            timers: TimersConfig::default(),
            persona: PersonaConfig::default(),
            backend: BackendConfig::default(),
            per_ssid: BTreeMap::new(),
        }
    }

    /// Render the resolved config back into the on-disk `RawConfig` shape
    /// where every field is the user's actual value. Used by the test
    /// suite to assert round-trip behavior.
    #[cfg(test)]
    pub fn to_raw_explicit(&self) -> RawConfig {
        RawConfig {
            profile: Some(self.profile),
            mac: Some(RawMacConfig {
                enabled: Some(self.mac.enabled),
                rotation_interval: Some(self.mac.rotation_interval.clone()),
                oui_pool: Some(self.mac.oui_pool.clone()),
            }),
            bluetooth: Some(RawBluetoothConfig {
                enabled: Some(self.bluetooth.enabled),
                generic_alias: Some(self.bluetooth.generic_alias),
                alias_source: Some(self.bluetooth.alias_source.clone()),
                pinned_alias: self.bluetooth.pinned_alias.clone(),
                discoverable: Some(self.bluetooth.discoverable),
                ble_rpa: Some(self.bluetooth.ble_rpa),
            }),
            hostname: Some(RawHostnameConfig {
                enabled: Some(self.hostname.enabled),
                mode: Some(self.hostname.mode.clone()),
                pinned_value: self.hostname.pinned_value.clone(),
                rotate_with_mac: Some(self.hostname.rotate_with_mac),
            }),
            dns: Some(RawDnsConfig {
                strip_edns_client_subnet: Some(self.dns.strip_edns_client_subnet),
            }),
            resolved: Some(RawResolvedConfig {
                mdns_off: Some(self.resolved.mdns_off),
                llmnr_off: Some(self.resolved.llmnr_off),
            }),
            ntp: Some(RawNtpConfig {
                enabled: Some(self.ntp.enabled),
                ntp_servers: Some(self.ntp.ntp_servers.clone()),
                fallback_servers: Some(self.ntp.fallback_servers.clone()),
            }),
            nft: Some(RawNftConfig {
                icmpv4_timestamp_drop: Some(self.nft.icmpv4_timestamp_drop),
                broadcast_ping_drop: Some(self.nft.broadcast_ping_drop),
                igmp_query_drop: Some(self.nft.igmp_query_drop),
            }),
            discovery: Some(RawDiscoveryConfig {
                mdns_silence: Some(self.discovery.mdns_silence),
                llmnr_silence: Some(self.discovery.llmnr_silence),
                ssdp_block: Some(self.discovery.ssdp_block),
                wsd_block: Some(self.discovery.wsd_block),
            }),
            probes: Some(RawProbesConfig {
                quorum_n: Some(self.probes.quorum_n),
                quorum_total: Some(self.probes.quorum_total),
                interval: Some(self.probes.interval.clone()),
                cooldown: Some(self.probes.cooldown.clone()),
                endpoints: Some(self.probes.endpoints.clone()),
            }),
            ipv6: Some(RawIpv6Config {
                enabled: Some(self.ipv6.enabled),
                use_temp_addresses: Some(self.ipv6.use_temp_addresses),
                addr_gen_mode: Some(self.ipv6.addr_gen_mode.clone()),
                ndp_hardening: Some(self.ipv6.ndp_hardening),
            }),
            enterprise_wifi: Some(RawEnterpriseWifiConfig {
                anonymous_outer_identity: Some(self.enterprise_wifi.anonymous_outer_identity),
                realm_strip_strategy: Some(self.enterprise_wifi.realm_strip_strategy.clone()),
                anonymous_realm: Some(self.enterprise_wifi.anonymous_realm.clone()),
            }),
            stack: Some(RawStackConfig {
                tcp_timestamps_off: Some(self.stack.tcp_timestamps_off),
                icmpv6_hardening: Some(self.stack.icmpv6_hardening),
                suppress_gratuitous_arp: Some(self.stack.suppress_gratuitous_arp),
                icmp_info_replies_drop: Some(self.stack.icmp_info_replies_drop),
            }),
            dhcp: Some(RawDhcpConfig {
                enabled: Some(self.dhcp.enabled),
                suppress_hostname: Some(self.dhcp.suppress_hostname),
                suppress_vendor_class: Some(self.dhcp.suppress_vendor_class),
                rotate_client_id: Some(self.dhcp.rotate_client_id),
                renew_on_apply: Some(self.dhcp.renew_on_apply),
            }),
            captive_portal: Some(RawCaptivePortalConfig {
                enabled: Some(self.captive_portal.enabled),
                detect_url: Some(self.captive_portal.detect_url.clone()),
                expected_response: Some(self.captive_portal.expected_response.clone()),
                policy: Some(self.captive_portal.policy.clone()),
                fresh_mac_per_visit: Some(self.captive_portal.fresh_mac_per_visit),
                timeout_secs: Some(self.captive_portal.timeout_secs),
            }),
            rf: Some(RawRfConfig {
                tx_power_reduce: Some(self.rf.tx_power_reduce),
                tx_power_reduction_db: Some(self.rf.tx_power_reduction_db),
                scan_random_mac: Some(self.rf.scan_random_mac),
            }),
            timers: Some(RawTimersConfig {
                rotate: Some(RawTimerConfig {
                    interval: Some(self.timers.rotate.interval.clone()),
                }),
                check: Some(RawTimerConfig {
                    interval: Some(self.timers.check.interval.clone()),
                }),
            }),
            persona: Some(RawPersonaConfig {
                active: self.persona.active.clone(),
            }),
            backend: Some(RawBackendConfig {
                driver: Some(self.backend.driver.clone()),
            }),
            per_ssid: self.per_ssid.clone(),
        }
    }
}

/// On-disk parsing target. Every field is `Option<T>` so resolution can
/// distinguish "user did not set this" from "user explicitly set this to
/// the same value the profile would produce." The distinction matters
/// for `proteus config show` (which annotates each value with its origin)
/// and `proteus config reset` (which clears overrides while preserving
/// the chosen profile).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawConfig {
    pub profile: Option<Profile>,
    pub mac: Option<RawMacConfig>,
    pub bluetooth: Option<RawBluetoothConfig>,
    pub hostname: Option<RawHostnameConfig>,
    pub dns: Option<RawDnsConfig>,
    pub resolved: Option<RawResolvedConfig>,
    pub ntp: Option<RawNtpConfig>,
    pub nft: Option<RawNftConfig>,
    pub discovery: Option<RawDiscoveryConfig>,
    pub probes: Option<RawProbesConfig>,
    pub ipv6: Option<RawIpv6Config>,
    pub enterprise_wifi: Option<RawEnterpriseWifiConfig>,
    pub stack: Option<RawStackConfig>,
    pub dhcp: Option<RawDhcpConfig>,
    pub captive_portal: Option<RawCaptivePortalConfig>,
    pub rf: Option<RawRfConfig>,
    pub timers: Option<RawTimersConfig>,
    pub persona: Option<RawPersonaConfig>,
    pub backend: Option<RawBackendConfig>,
    /// Per-SSID policies (roadmap Milestone 3). Stored as a flat map so
    /// `[per_ssid."<ssid>"]` round-trips through TOML without losing
    /// fields the resolver would otherwise discard. The map is always
    /// carried through `resolve()` verbatim — the precedence merge with
    /// persona / profile / defaults happens at NM connection-up time
    /// via `crate::per_ssid::resolve_for_ssid`, not at config load.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_ssid: BTreeMap<String, PerSsidPolicy>,
}

impl RawConfig {
    /// Overlay the user's explicit fields on top of the active profile's
    /// baseline. `Profile::Off` short-circuits and returns the
    /// all-disabled baseline regardless of overrides.
    pub fn resolve(self) -> Config {
        let profile = self.profile.unwrap_or_default();
        let mut cfg = profile.baseline();
        if profile == Profile::Off {
            return cfg;
        }
        if let Some(m) = self.mac {
            if let Some(v) = m.enabled {
                cfg.mac.enabled = v;
            }
            if let Some(v) = m.rotation_interval {
                cfg.mac.rotation_interval = v;
            }
            if let Some(v) = m.oui_pool {
                cfg.mac.oui_pool = v;
            }
        }
        if let Some(b) = self.bluetooth {
            if let Some(v) = b.enabled {
                cfg.bluetooth.enabled = v;
            }
            if let Some(v) = b.generic_alias {
                cfg.bluetooth.generic_alias = v;
            }
            if let Some(v) = b.alias_source {
                cfg.bluetooth.alias_source = v;
            }
            if b.pinned_alias.is_some() {
                cfg.bluetooth.pinned_alias = b.pinned_alias;
            }
            if let Some(v) = b.discoverable {
                cfg.bluetooth.discoverable = v;
            }
            if let Some(v) = b.ble_rpa {
                cfg.bluetooth.ble_rpa = v;
            }
        }
        if let Some(h) = self.hostname {
            if let Some(v) = h.enabled {
                cfg.hostname.enabled = v;
            }
            if let Some(v) = h.mode {
                cfg.hostname.mode = v;
            }
            if h.pinned_value.is_some() {
                cfg.hostname.pinned_value = h.pinned_value;
            }
            if let Some(v) = h.rotate_with_mac {
                cfg.hostname.rotate_with_mac = v;
            }
        }
        if let Some(d) = self.dns
            && let Some(v) = d.strip_edns_client_subnet
        {
            cfg.dns.strip_edns_client_subnet = v;
        }
        if let Some(r) = self.resolved {
            if let Some(v) = r.mdns_off {
                cfg.resolved.mdns_off = v;
            }
            if let Some(v) = r.llmnr_off {
                cfg.resolved.llmnr_off = v;
            }
        }
        if let Some(n) = self.ntp {
            if let Some(v) = n.enabled {
                cfg.ntp.enabled = v;
            }
            if let Some(v) = n.ntp_servers {
                cfg.ntp.ntp_servers = v;
            }
            if let Some(v) = n.fallback_servers {
                cfg.ntp.fallback_servers = v;
            }
        }
        if let Some(n) = self.nft {
            if let Some(v) = n.icmpv4_timestamp_drop {
                cfg.nft.icmpv4_timestamp_drop = v;
            }
            if let Some(v) = n.broadcast_ping_drop {
                cfg.nft.broadcast_ping_drop = v;
            }
            if let Some(v) = n.igmp_query_drop {
                cfg.nft.igmp_query_drop = v;
            }
        }
        if let Some(d) = self.discovery {
            if let Some(v) = d.mdns_silence {
                cfg.discovery.mdns_silence = v;
            }
            if let Some(v) = d.llmnr_silence {
                cfg.discovery.llmnr_silence = v;
            }
            if let Some(v) = d.ssdp_block {
                cfg.discovery.ssdp_block = v;
            }
            if let Some(v) = d.wsd_block {
                cfg.discovery.wsd_block = v;
            }
        }
        if let Some(p) = self.probes {
            if let Some(v) = p.quorum_n {
                cfg.probes.quorum_n = v;
            }
            if let Some(v) = p.quorum_total {
                cfg.probes.quorum_total = v;
            }
            if let Some(v) = p.interval {
                cfg.probes.interval = v;
            }
            if let Some(v) = p.cooldown {
                cfg.probes.cooldown = v;
            }
            if let Some(v) = p.endpoints {
                cfg.probes.endpoints = v;
            }
        }
        if let Some(i) = self.ipv6 {
            if let Some(v) = i.enabled {
                cfg.ipv6.enabled = v;
            }
            if let Some(v) = i.use_temp_addresses {
                cfg.ipv6.use_temp_addresses = v;
            }
            if let Some(v) = i.addr_gen_mode {
                cfg.ipv6.addr_gen_mode = v;
            }
            if let Some(v) = i.ndp_hardening {
                cfg.ipv6.ndp_hardening = v;
            }
        }
        if let Some(e) = self.enterprise_wifi {
            if let Some(v) = e.anonymous_outer_identity {
                cfg.enterprise_wifi.anonymous_outer_identity = v;
            }
            if let Some(v) = e.realm_strip_strategy {
                cfg.enterprise_wifi.realm_strip_strategy = v;
            }
            if let Some(v) = e.anonymous_realm {
                cfg.enterprise_wifi.anonymous_realm = v;
            }
        }
        if let Some(s) = self.stack {
            if let Some(v) = s.tcp_timestamps_off {
                cfg.stack.tcp_timestamps_off = v;
            }
            if let Some(v) = s.icmpv6_hardening {
                cfg.stack.icmpv6_hardening = v;
            }
            if let Some(v) = s.suppress_gratuitous_arp {
                cfg.stack.suppress_gratuitous_arp = v;
            }
            if let Some(v) = s.icmp_info_replies_drop {
                cfg.stack.icmp_info_replies_drop = v;
            }
        }
        if let Some(d) = self.dhcp {
            if let Some(v) = d.enabled {
                cfg.dhcp.enabled = v;
            }
            if let Some(v) = d.suppress_hostname {
                cfg.dhcp.suppress_hostname = v;
            }
            if let Some(v) = d.suppress_vendor_class {
                cfg.dhcp.suppress_vendor_class = v;
            }
            if let Some(v) = d.rotate_client_id {
                cfg.dhcp.rotate_client_id = v;
            }
            if let Some(v) = d.renew_on_apply {
                cfg.dhcp.renew_on_apply = v;
            }
        }
        if let Some(c) = self.captive_portal {
            if let Some(v) = c.enabled {
                cfg.captive_portal.enabled = v;
            }
            if let Some(v) = c.detect_url {
                cfg.captive_portal.detect_url = v;
            }
            if let Some(v) = c.expected_response {
                cfg.captive_portal.expected_response = v;
            }
            if let Some(v) = c.policy {
                cfg.captive_portal.policy = v;
            }
            if let Some(v) = c.fresh_mac_per_visit {
                cfg.captive_portal.fresh_mac_per_visit = v;
            }
            if let Some(v) = c.timeout_secs {
                cfg.captive_portal.timeout_secs = v;
            }
        }
        if let Some(r) = self.rf {
            if let Some(v) = r.tx_power_reduce {
                cfg.rf.tx_power_reduce = v;
            }
            if let Some(v) = r.tx_power_reduction_db {
                cfg.rf.tx_power_reduction_db = v;
            }
            if let Some(v) = r.scan_random_mac {
                cfg.rf.scan_random_mac = v;
            }
        }
        if let Some(t) = self.timers {
            if let Some(r) = t.rotate
                && let Some(v) = r.interval
            {
                cfg.timers.rotate.interval = v;
            }
            if let Some(c) = t.check
                && let Some(v) = c.interval
            {
                cfg.timers.check.interval = v;
            }
        }
        if let Some(p) = self.persona {
            cfg.persona.active = p.active;
        }
        if let Some(b) = self.backend
            && let Some(v) = b.driver
        {
            // Reject anything outside the backend selector's grammar
            // up-front so a typo'd `[backend] driver` lands in
            // `proteus doctor` rather than triggering an obscure
            // failure deep inside `select::select`. Match the
            // existing config behaviour: fall back to the default
            // and log so the user sees the rejection in `-v` mode.
            if crate::backend::select::is_valid_driver(&v) {
                cfg.backend.driver = v;
            } else {
                tracing::warn!(
                    driver = v.as_str(),
                    "ignoring invalid [backend] driver; expected one of auto|nm|networkd|raw"
                );
            }
        }
        cfg.per_ssid = self.per_ssid;
        cfg
    }

    /// True iff the user has set at least one per-knob override on top
    /// of the profile baseline. Used by `proteus config reset` to report
    /// how many overrides were cleared.
    pub fn has_overrides(&self) -> bool {
        macro_rules! any_some {
            ($section:expr, [$($field:ident),+ $(,)?]) => {
                if let Some(s) = $section {
                    $( if s.$field.is_some() { return true; } )+
                }
            };
        }
        any_some!(&self.mac, [enabled, rotation_interval, oui_pool]);
        any_some!(
            &self.bluetooth,
            [
                enabled,
                generic_alias,
                alias_source,
                pinned_alias,
                discoverable,
                ble_rpa
            ]
        );
        any_some!(
            &self.hostname,
            [enabled, mode, pinned_value, rotate_with_mac]
        );
        any_some!(&self.dns, [strip_edns_client_subnet]);
        any_some!(&self.resolved, [mdns_off, llmnr_off]);
        any_some!(&self.ntp, [enabled, ntp_servers, fallback_servers]);
        any_some!(
            &self.nft,
            [icmpv4_timestamp_drop, broadcast_ping_drop, igmp_query_drop]
        );
        any_some!(
            &self.discovery,
            [mdns_silence, llmnr_silence, ssdp_block, wsd_block]
        );
        any_some!(
            &self.probes,
            [quorum_n, quorum_total, interval, cooldown, endpoints]
        );
        any_some!(
            &self.ipv6,
            [enabled, use_temp_addresses, addr_gen_mode, ndp_hardening]
        );
        any_some!(
            &self.enterprise_wifi,
            [
                anonymous_outer_identity,
                realm_strip_strategy,
                anonymous_realm
            ]
        );
        any_some!(
            &self.stack,
            [
                tcp_timestamps_off,
                icmpv6_hardening,
                suppress_gratuitous_arp,
                icmp_info_replies_drop
            ]
        );
        any_some!(
            &self.dhcp,
            [
                enabled,
                suppress_hostname,
                suppress_vendor_class,
                rotate_client_id,
                renew_on_apply
            ]
        );
        any_some!(
            &self.captive_portal,
            [
                enabled,
                detect_url,
                expected_response,
                policy,
                fresh_mac_per_visit,
                timeout_secs
            ]
        );
        any_some!(
            &self.rf,
            [tx_power_reduce, tx_power_reduction_db, scan_random_mac]
        );
        if let Some(t) = &self.timers {
            if let Some(r) = &t.rotate
                && r.interval.is_some()
            {
                return true;
            }
            if let Some(c) = &t.check
                && c.interval.is_some()
            {
                return true;
            }
        }
        if let Some(p) = &self.persona
            && p.active.is_some()
        {
            return true;
        }
        any_some!(&self.backend, [driver]);
        if !self.per_ssid.is_empty() {
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawMacConfig {
    pub enabled: Option<bool>,
    pub rotation_interval: Option<String>,
    pub oui_pool: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawBluetoothConfig {
    pub enabled: Option<bool>,
    pub generic_alias: Option<bool>,
    pub alias_source: Option<String>,
    pub pinned_alias: Option<String>,
    pub discoverable: Option<bool>,
    pub ble_rpa: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawHostnameConfig {
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub pinned_value: Option<String>,
    pub rotate_with_mac: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawDnsConfig {
    pub strip_edns_client_subnet: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawResolvedConfig {
    pub mdns_off: Option<bool>,
    pub llmnr_off: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawNtpConfig {
    pub enabled: Option<bool>,
    pub ntp_servers: Option<Vec<String>>,
    pub fallback_servers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawNftConfig {
    pub icmpv4_timestamp_drop: Option<bool>,
    pub broadcast_ping_drop: Option<bool>,
    pub igmp_query_drop: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawDiscoveryConfig {
    pub mdns_silence: Option<bool>,
    pub llmnr_silence: Option<bool>,
    pub ssdp_block: Option<bool>,
    pub wsd_block: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawProbesConfig {
    pub quorum_n: Option<u8>,
    pub quorum_total: Option<u8>,
    pub interval: Option<String>,
    pub cooldown: Option<String>,
    pub endpoints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawIpv6Config {
    pub enabled: Option<bool>,
    pub use_temp_addresses: Option<bool>,
    pub addr_gen_mode: Option<String>,
    pub ndp_hardening: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawEnterpriseWifiConfig {
    pub anonymous_outer_identity: Option<bool>,
    pub realm_strip_strategy: Option<String>,
    pub anonymous_realm: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawStackConfig {
    pub tcp_timestamps_off: Option<bool>,
    pub icmpv6_hardening: Option<bool>,
    pub suppress_gratuitous_arp: Option<bool>,
    pub icmp_info_replies_drop: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawDhcpConfig {
    pub enabled: Option<bool>,
    pub suppress_hostname: Option<bool>,
    pub suppress_vendor_class: Option<bool>,
    pub rotate_client_id: Option<bool>,
    /// Roadmap Milestone 4c: when true, the orchestrator runs `dhcp
    /// renew` after `apply` so the upstream DHCP server hands out a
    /// fresh lease against the new client identity. Default false —
    /// integration-wired in the follow-up; the knob ships now so the
    /// schema is stable.
    pub renew_on_apply: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawCaptivePortalConfig {
    pub enabled: Option<bool>,
    pub detect_url: Option<String>,
    pub expected_response: Option<String>,
    pub policy: Option<String>,
    pub fresh_mac_per_visit: Option<bool>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawRfConfig {
    pub tx_power_reduce: Option<bool>,
    pub tx_power_reduction_db: Option<u8>,
    /// Roadmap Milestone 4b — see `RfConfig::scan_random_mac`.
    pub scan_random_mac: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTimersConfig {
    pub rotate: Option<RawTimerConfig>,
    pub check: Option<RawTimerConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTimerConfig {
    pub interval: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawPersonaConfig {
    pub active: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawBackendConfig {
    pub driver: Option<String>,
}

// ---- Resolved (public) sub-configs --------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacConfig {
    pub enabled: bool,
    pub rotation_interval: String,
    pub oui_pool: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BluetoothConfig {
    pub enabled: bool,
    pub generic_alias: bool,
    pub alias_source: String,
    pub pinned_alias: Option<String>,
    pub discoverable: bool,
    pub ble_rpa: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostnameConfig {
    pub enabled: bool,
    pub mode: String,
    pub pinned_value: Option<String>,
    /// Rotate hostname every time MAC rotates. Default off — see wiki.
    pub rotate_with_mac: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsConfig {
    pub strip_edns_client_subnet: bool,
}

/// Drop-in for `systemd-resolved` (`/etc/systemd/resolved.conf.d/10-proteus-mdns-llmnr.conf`).
/// Hard-guards on a foreign drop-in or non-resolved `/etc/resolv.conf`; in
/// either case Proteus defers to whatever is already in charge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedConfig {
    /// Disable the resolved mDNS responder + resolver. The Avahi-driven path
    /// the user may already use is unaffected — this knob only shapes
    /// resolved's own behaviour.
    pub mdns_off: bool,
    /// Disable LLMNR responder + resolver in resolved.
    pub llmnr_off: bool,
}

/// Drop-in for `systemd-timesyncd` (`/etc/systemd/timesyncd.conf.d/10-proteus.conf`).
/// Skipped when `chronyd` or `ntpd` is on the system — both manage their own
/// configs and Proteus does not fight them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NtpConfig {
    /// Master switch for writing the timesyncd drop-in. When off, apply
    /// removes any prior Proteus-managed file.
    pub enabled: bool,
    /// Privacy-preserving NTP pool. Persona-aware customization is a
    /// follow-up in roadmap Milestone 4a.
    pub ntp_servers: Vec<String>,
    /// Fallback servers if the primary list is unreachable.
    pub fallback_servers: Vec<String>,
}

/// nftables-side opt-in rules. Mirrors the `[discovery]` `ssdp_block` style
/// — every flag defaults to `false` so the table stays minimal until the
/// operator enables a specific drop. Persona-aware variants (e.g. iOS
/// blocks 5353 inbound; Android allows it) are tracked separately.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NftConfig {
    /// `icmp type timestamp-request drop`. ICMP timestamps are an
    /// underused fingerprint vector; the existing `icmp_drops` chain
    /// already covers timestamp-request via the same mechanism, but this
    /// flag lets the operator narrow the picture in `nft status`.
    pub icmpv4_timestamp_drop: bool,
    /// Drop ICMPv4 echo-request to broadcast addresses (smurf-style probes).
    pub broadcast_ping_drop: bool,
    /// Suppress IGMP membership-query replies on input.
    pub igmp_query_drop: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    pub mdns_silence: bool,
    pub llmnr_silence: bool,
    pub ssdp_block: bool,
    pub wsd_block: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbesConfig {
    pub quorum_n: u8,
    pub quorum_total: u8,
    pub interval: String,
    pub cooldown: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ipv6Config {
    pub enabled: bool,
    pub use_temp_addresses: bool,
    pub addr_gen_mode: String,
    pub ndp_hardening: bool,
}

/// 802.1X anonymous outer identity for enterprise Wi-Fi (eduroam,
/// corporate). Opt-in, default off — some auth servers reject mismatched
/// outer/inner identities. See `proteus wiki enterprise-wifi`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnterpriseWifiConfig {
    pub anonymous_outer_identity: bool,
    /// `auto` extracts the realm from `802-1x.identity` (the part after `@`).
    /// `manual` uses `anonymous_realm` verbatim.
    pub realm_strip_strategy: String,
    /// Used when `realm_strip_strategy = "manual"`. Empty otherwise.
    pub anonymous_realm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StackConfig {
    pub tcp_timestamps_off: bool,
    pub icmpv6_hardening: bool,
    pub suppress_gratuitous_arp: bool,
    pub icmp_info_replies_drop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DhcpConfig {
    pub enabled: bool,
    pub suppress_hostname: bool,
    pub suppress_vendor_class: bool,
    pub rotate_client_id: bool,
    /// Roadmap Milestone 4c: when true, the orchestrator runs `dhcp
    /// renew` after `apply` so the upstream DHCP server hands out a
    /// fresh lease against the new client identity. Default false.
    pub renew_on_apply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptivePortalConfig {
    pub enabled: bool,
    pub detect_url: String,
    pub expected_response: String,
    pub policy: String,
    pub fresh_mac_per_visit: bool,
    pub timeout_secs: u64,
}

/// Wi-Fi RF surface controls. The TX-power knob is opt-in: enabling it
/// shrinks the passive-capture radius at the cost of range from the AP.
/// Default reduction is 6 dB (~quarter the radiated power).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RfConfig {
    /// Master switch for TX-power reduction. Off in Min/Low/Med profiles,
    /// on in High and Agr.
    pub tx_power_reduce: bool,
    /// dB below the regulatory maximum. Hardware-clamped on actual write.
    pub tx_power_reduction_db: u8,
    /// Roadmap Milestone 4b: scan-time MAC randomization at the NM layer.
    /// When true, `proteus apply` writes
    /// `wifi.scan-rand-mac-address = "random"` and
    /// `wifi.mac-address-randomization = 2` on every managed Wi-Fi
    /// connection — supplicant scans use a per-scan random source MAC,
    /// and saved-SSID probe lists stop being broadcast in the clear.
    /// Default `true` (high-value, low-risk; see `wiki/wpa-supplicant-hardening.md`).
    pub scan_random_mac: bool,
}

/// Per-timer cadence baselines. Each entry maps to a `proteus-<name>.timer`
/// systemd unit; `interval` accepts the same syntax as `proteus timer set
/// <name> --interval <duration>` (compact durations like `2h`, named
/// systemd cadences, raw calendar expressions). The sentinel value
/// `"never"` means "do not run this timer"; the apply orchestrator
/// removes any existing drop-in for a `"never"` interval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimersConfig {
    pub rotate: TimerConfig,
    pub check: TimerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerConfig {
    pub interval: String,
}

/// Active persona pointer. `active = None` means "no persona; the
/// `Profile` slider drives entropy-only randomization" — the v0.2.x
/// behaviour. `active = Some("iphone-15")` means "shape every controlled
/// fingerprint to look like an iPhone 15."
///
/// Loaded by `proteus persona use <id>` and surfaced via
/// `proteus persona current`. The integration with the apply / rotate
/// paths (MAC OUI shaping, hostname rendering, DHCP fingerprint write)
/// is the follow-up tracked in roadmap Milestone 2 "Integration"; this
/// PR ships the schema, catalogue, loader, and CLI surface only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonaConfig {
    pub active: Option<String>,
}

/// One `[per_ssid."<ssid>"]` block. Every field is `Option<...>` so the
/// operator can override exactly the knobs they care about and let the
/// rest fall through the precedence chain (persona → profile → defaults).
///
/// Roadmap Milestone 3. The struct is shared by `RawConfig` and the
/// resolved `Config` because it is pass-through: the per-SSID resolve
/// happens at NM `connection-up` time via
/// `crate::per_ssid::resolve_for_ssid`, not at config-load time. Storing
/// it on both sides keeps the round-trip clean and lets read commands
/// (`proteus ssid list / show`) inspect it without re-parsing the file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PerSsidPolicy {
    /// Persona id to use on this SSID (e.g. `"iphone-15"`). When set this
    /// beats the global `[persona] active`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// `Profile` slider override: one of `off|min|low|med|high|agr`.
    /// Lets a single hostile SSID lift to `agr` without flipping the
    /// global profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggressiveness_profile: Option<String>,
    /// Pin a literal MAC for this SSID (e.g. `"aa:bb:cc:dd:ee:ff"`).
    /// Useful for home networks where the operator wants a stable lease.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_mac: Option<String>,
    /// Rotation interval override (e.g. `"30m"`, `"4h"`). Same syntax as
    /// `[timers.rotate].interval` and `proteus timer set --interval`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate_interval: Option<String>,
    /// Captive-portal-style policy override. Currently the only known
    /// value is `"fresh-mac-per-visit"`; the resolver passes the string
    /// through verbatim so future policies can land without a schema bump.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_policy: Option<String>,
}

/// `NetworkBackend` driver selector. Roadmap Milestone 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendConfig {
    /// One of `auto` | `nm` | `networkd` | `raw`. `auto` walks the
    /// available backends in order at runtime.
    pub driver: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            driver: "auto".into(),
        }
    }
}

// ---- Defaults -----------------------------------------------------------
//
// These provide the non-profile-affected fields (intervals, modes, paths,
// numeric tunables). The bool toggles populated here are placeholders that
// `Profile::baseline` always overwrites; treat them as "structural" only.

// Per-section `Default` impls return the standalone "as documented"
// values: what each feature would do when enabled with no profile or
// override context. The profile system always overwrites the bool
// toggles via `apply_bools`, so the bool values here are inert when
// going through `Config::default_or_loaded`. They are still meaningful
// when downstream code constructs a sub-config directly (e.g. for unit
// tests of the rendering helpers).

impl Default for MacConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rotation_interval: "2h".into(),
            oui_pool: vec![
                "apple".into(),
                "intel".into(),
                "samsung".into(),
                "dell".into(),
                "random-locally-administered".into(),
            ],
        }
    }
}

impl Default for BluetoothConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            generic_alias: true,
            alias_source: "generic".into(),
            pinned_alias: None,
            discoverable: false,
            ble_rpa: true,
        }
    }
}

impl Default for HostnameConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "wordlist".into(),
            pinned_value: None,
            rotate_with_mac: false,
        }
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            strip_edns_client_subnet: true,
        }
    }
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            mdns_off: true,
            llmnr_off: true,
        }
    }
}

impl Default for NtpConfig {
    fn default() -> Self {
        // Privacy-preserving defaults: the Fedora pool covers most users by
        // default (already trusted on Fedora hosts) and Cloudflare time is
        // a well-known privacy-respecting fallback that doesn't require an
        // RPM-supplied root anchor.
        Self {
            enabled: true,
            ntp_servers: vec!["2.fedora.pool.ntp.org".into()],
            fallback_servers: vec!["time.cloudflare.com".into()],
        }
    }
}

impl Default for ProbesConfig {
    fn default() -> Self {
        Self {
            quorum_n: 3,
            quorum_total: 4,
            interval: "5m".into(),
            cooldown: "60s".into(),
            endpoints: vec![
                "1.1.1.1:443".into(),
                "8.8.8.8:443".into(),
                "9.9.9.9:443".into(),
                "142.250.190.78:443".into(),
            ],
        }
    }
}

impl Default for Ipv6Config {
    fn default() -> Self {
        Self {
            enabled: true,
            use_temp_addresses: true,
            addr_gen_mode: "stable-privacy".into(),
            ndp_hardening: true,
        }
    }
}

impl Default for EnterpriseWifiConfig {
    fn default() -> Self {
        Self {
            anonymous_outer_identity: false,
            realm_strip_strategy: "auto".into(),
            anonymous_realm: String::new(),
        }
    }
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            tcp_timestamps_off: true,
            icmpv6_hardening: true,
            suppress_gratuitous_arp: false,
            icmp_info_replies_drop: true,
        }
    }
}

impl Default for DhcpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            suppress_hostname: true,
            suppress_vendor_class: true,
            rotate_client_id: true,
            renew_on_apply: false,
        }
    }
}

impl Default for CaptivePortalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            detect_url: "http://nmcheck.gnome.org/check_network_status.txt".into(),
            expected_response: "NetworkManager is online".into(),
            policy: "rotate-before-auth".into(),
            fresh_mac_per_visit: true,
            timeout_secs: 5,
        }
    }
}

impl Default for RfConfig {
    fn default() -> Self {
        Self {
            tx_power_reduce: false,
            tx_power_reduction_db: 6,
            // Milestone 4b: opt-out, not opt-in. The NM keys this writes
            // are inert on hardware that doesn't support them, so leaving
            // it on by default costs nothing and the privacy win is real.
            scan_random_mac: true,
        }
    }
}

// `Default` for `TimersConfig` returns the structural placeholder shape:
// the per-timer `Default` values are inert sentinels that
// `Profile::baseline` always overwrites with the profile-specific cadence.
// Direct callers (tests of the renderer) see the documented "as-is" defaults.
impl Default for TimersConfig {
    fn default() -> Self {
        Self {
            rotate: TimerConfig {
                interval: "2h".into(),
            },
            check: TimerConfig {
                interval: "5m".into(),
            },
        }
    }
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            interval: "2h".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_med_profile() {
        let cfg = Config::default();
        assert_eq!(cfg.profile, Profile::Med);
        assert!(cfg.mac.enabled);
        assert!(cfg.discovery.mdns_silence);
        assert!(cfg.discovery.llmnr_silence);
        assert!(!cfg.discovery.ssdp_block);
    }

    #[test]
    fn empty_toml_resolves_to_default_profile() {
        let raw: RawConfig = toml::from_str("").unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.profile, Profile::Med);
    }

    #[test]
    fn user_override_takes_precedence_over_profile_baseline() {
        let toml_str = r#"
profile = "med"

[mac]
enabled = false

[discovery]
ssdp_block = true
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.profile, Profile::Med);
        assert!(!cfg.mac.enabled, "user override should beat profile");
        // med has mdns_silence on
        assert!(cfg.discovery.mdns_silence);
        // user enabled ssdp_block (med has it off)
        assert!(cfg.discovery.ssdp_block);
    }

    #[test]
    fn off_ignores_user_overrides() {
        let toml_str = r#"
profile = "off"

[mac]
enabled = true

[dhcp]
enabled = true
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.profile, Profile::Off);
        assert!(!cfg.mac.enabled, "Off overrides user-enabled mac");
        assert!(!cfg.dhcp.enabled, "Off overrides user-enabled dhcp");
    }

    #[test]
    fn off_preserves_overrides_in_raw_form() {
        // The on-disk form keeps the overrides; only resolution ignores
        // them. Switching back to a non-Off profile should restore them.
        let toml_str = r#"
profile = "off"

[mac]
enabled = true
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
        // Now imagine the user switches profile back to med.
        let mut switched = raw.clone();
        switched.profile = Some(Profile::Med);
        let cfg = switched.resolve();
        assert!(cfg.mac.enabled, "override survives Off → Med transition");
    }

    #[test]
    fn has_overrides_detects_a_single_explicit_field() {
        let with_override: RawConfig = toml::from_str("[mac]\nenabled = false\n").unwrap();
        assert!(with_override.has_overrides());

        let no_overrides: RawConfig = toml::from_str("profile = \"med\"\n").unwrap();
        assert!(!no_overrides.has_overrides());

        let empty: RawConfig = toml::from_str("").unwrap();
        assert!(!empty.has_overrides());
    }

    #[test]
    fn agr_baseline_resolves_with_every_breaking_knob_on() {
        let raw: RawConfig = toml::from_str("profile = \"agr\"\n").unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.profile, Profile::Agr);
        assert!(cfg.discovery.ssdp_block);
        assert!(cfg.discovery.wsd_block);
        assert!(cfg.enterprise_wifi.anonymous_outer_identity);
        assert!(cfg.stack.suppress_gratuitous_arp);
        assert!(cfg.captive_portal.fresh_mac_per_visit);
    }

    #[test]
    fn raw_config_round_trips_through_toml() {
        let cfg = Config::default();
        let raw = cfg.to_raw_explicit();
        let s = toml::to_string(&raw).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let resolved = parsed.resolve();
        assert_eq!(resolved.profile, cfg.profile);
        assert_eq!(resolved.mac.enabled, cfg.mac.enabled);
        assert_eq!(resolved.dhcp.enabled, cfg.dhcp.enabled);
    }

    #[test]
    fn rf_section_round_trips_through_toml() {
        let toml_str = r#"
profile = "med"

[rf]
tx_power_reduce = true
tx_power_reduction_db = 9
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert!(cfg.rf.tx_power_reduce);
        assert_eq!(cfg.rf.tx_power_reduction_db, 9);
        let raw2 = cfg.to_raw_explicit();
        let s = toml::to_string(&raw2).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        assert!(back.rf.tx_power_reduce);
        assert_eq!(back.rf.tx_power_reduction_db, 9);
    }

    #[test]
    fn timers_round_trip_through_toml() {
        let toml_str = r#"
profile = "med"

[timers.rotate]
interval = "1h"

[timers.check]
interval = "30s"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
        let resolved = raw.resolve();
        let raw_back = resolved.to_raw_explicit();
        let s = toml::to_string(&raw_back).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let resolved_back = parsed.resolve();
        assert_eq!(resolved_back.timers.rotate.interval, "1h");
        assert_eq!(resolved_back.timers.check.interval, "30s");
    }

    #[test]
    fn timer_user_override_survives_profile_change_med_to_high() {
        let toml_str = r#"
profile = "med"

[timers.rotate]
interval = "1h"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.clone().resolve();
        assert_eq!(cfg.timers.rotate.interval, "1h");
        assert_eq!(cfg.timers.check.interval, "5m");

        let mut switched = raw;
        switched.profile = Some(Profile::High);
        let cfg = switched.resolve();
        assert_eq!(
            cfg.timers.rotate.interval, "1h",
            "user override should survive profile change"
        );
        assert_eq!(
            cfg.timers.check.interval, "2m",
            "non-overridden timer should follow new profile"
        );
    }

    #[test]
    fn off_profile_short_circuits_timer_overrides() {
        let toml_str = r#"
profile = "off"

[timers.rotate]
interval = "30m"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.timers.rotate.interval, "never");
        assert_eq!(cfg.timers.check.interval, "never");
    }

    #[test]
    fn timers_section_alone_triggers_has_overrides() {
        let raw: RawConfig = toml::from_str("[timers.rotate]\ninterval = \"1h\"\n").unwrap();
        assert!(raw.has_overrides());
    }

    #[test]
    fn backend_default_driver_is_auto() {
        assert_eq!(BackendConfig::default().driver, "auto");
        assert_eq!(Config::default().backend.driver, "auto");
    }

    #[test]
    fn backend_section_round_trips() {
        let toml_str = r#"
profile = "med"

[backend]
driver = "networkd"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
        let cfg = raw.resolve();
        assert_eq!(cfg.backend.driver, "networkd");
    }

    #[test]
    fn backend_invalid_driver_falls_back_to_default() {
        let toml_str = r#"
[backend]
driver = "garbage"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(
            cfg.backend.driver, "auto",
            "invalid driver must not bleed into resolved config"
        );
    }

    /// Roadmap Milestone 3: a `[per_ssid."<ssid>"]` block must round-trip
    /// through TOML losslessly so the on-disk shape is the authoritative
    /// representation.
    #[test]
    fn per_ssid_block_round_trips_through_toml() {
        let toml_str = r#"
profile = "med"

[per_ssid."coffee-shop"]
persona = "iphone-15"
aggressiveness_profile = "high"
pin_mac = "aa:bb:cc:dd:ee:ff"
rotate_interval = "30m"
portal_policy = "fresh-mac-per-visit"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
        let cfg = raw.resolve();
        let entry = cfg
            .per_ssid
            .get("coffee-shop")
            .expect("coffee-shop entry should be present");
        assert_eq!(entry.persona.as_deref(), Some("iphone-15"));
        assert_eq!(entry.aggressiveness_profile.as_deref(), Some("high"));
        assert_eq!(entry.pin_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(entry.rotate_interval.as_deref(), Some("30m"));
        assert_eq!(entry.portal_policy.as_deref(), Some("fresh-mac-per-visit"));

        let raw2 = cfg.to_raw_explicit();
        let s = toml::to_string(&raw2).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        let entry_back = back.per_ssid.get("coffee-shop").unwrap();
        assert_eq!(entry_back.persona.as_deref(), Some("iphone-15"));
        assert_eq!(entry_back.portal_policy.as_deref(), Some("fresh-mac-per-visit"));
    }

    #[test]
    fn per_ssid_partial_block_keeps_other_fields_none() {
        let toml_str = r#"
profile = "med"

[per_ssid."home-lan"]
pin_mac = "12:34:56:78:9a:bc"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        let entry = cfg.per_ssid.get("home-lan").unwrap();
        assert_eq!(entry.pin_mac.as_deref(), Some("12:34:56:78:9a:bc"));
        assert!(entry.persona.is_none());
        assert!(entry.aggressiveness_profile.is_none());
        assert!(entry.rotate_interval.is_none());
        assert!(entry.portal_policy.is_none());
    }

    #[test]
    fn per_ssid_section_alone_triggers_has_overrides() {
        let toml_str = r#"
[per_ssid."conference-wifi"]
aggressiveness_profile = "agr"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
    }

    #[test]
    fn empty_per_ssid_map_does_not_serialize_a_section() {
        let cfg = Config::default();
        assert!(cfg.per_ssid.is_empty());
        let raw = cfg.to_raw_explicit();
        let s = toml::to_string(&raw).unwrap();
        assert!(
            !s.contains("[per_ssid"),
            "default config must not emit a [per_ssid] section: {s}"
        );
    }
}
