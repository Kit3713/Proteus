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
    /// Event-driven trigger framework (Milestone 4c). Off by default
    /// for v0.3.x — operators opt in via `[events] enabled = true`,
    /// then start `proteus-events.service` (or run `proteus events
    /// run` directly). The four subscribed sources (NM connection-up,
    /// netlink link-flap, nl80211 reg-domain, captive-portal poller)
    /// each gracefully degrade when the host can't honour them, so
    /// turning the master switch on is safe even on partial
    /// platforms — degraded sources just don't fire.
    pub events: EventsConfig,
    /// Backend selector (Milestone 1). `driver = "auto"` walks
    /// NM → networkd → raw at runtime; only the NM impl is fully
    /// wired in this PR.
    pub backend: BackendConfig,
    /// Logging-layer redaction policy (roadmap 1.0.5). Controls how device
    /// identifiers (MAC / SSID / hostname / 802.1X) are rendered at *log*
    /// sites. Default `"redacted"`. This never affects `--json` output or
    /// CLI display, which always show real values — it shapes journald /
    /// stderr only. See `crate::redaction`.
    pub logging: LoggingConfig,
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
    ///
    /// Issue #229: cross-field validation runs after resolve so an
    /// out-of-range `[probes].quorum_n` (or any other invariant added
    /// to `Config::validate`) bails with a clear message instead of
    /// letting `proteus probe` run silently misclassified.
    ///
    /// Issue #227: the parse step uses `#[serde(deny_unknown_fields)]`
    /// on every `Raw*` struct, so a typo'd key fails at `toml::from_str`
    /// with `unknown field` rather than silently no-op'ing. After
    /// resolution, `RawConfig::validate_ranges` (run via the same
    /// `validate` entrypoint) enforces numeric ranges so e.g.
    /// `quorum_n = 0` and `timeout_secs = u64::MAX` bail loudly.
    pub fn default_or_loaded(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let raw: RawConfig =
                    toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
                raw.validate_ranges()
                    .with_context(|| format!("validating {}", path.display()))?;
                let cfg = raw.resolve();
                cfg.validate()
                    .with_context(|| format!("validating {}", path.display()))?;
                // Roadmap 1.0.5: install the logging-layer redaction policy
                // from the resolved config. First-writer-wins inside
                // `set_policy`; done here (not in `logging::init`) because the
                // logger is set up before config is read. `validate_ranges`
                // above already rejected an unknown value, so `parse` succeeds
                // for any value that reaches here; `unwrap_or_default` keeps
                // the safe `Redacted` fallback for total robustness.
                crate::redaction::set_policy(
                    crate::redaction::IdentifierPolicy::parse(&cfg.logging.identifiers)
                        .unwrap_or_default(),
                );
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Self::default();
                crate::redaction::set_policy(
                    crate::redaction::IdentifierPolicy::parse(&cfg.logging.identifiers)
                        .unwrap_or_default(),
                );
                Ok(cfg)
            }
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Issue #229: validate cross-field invariants the per-section TOML
    /// shape can't express on its own. Today this only checks the
    /// probe-quorum trio, but the function is the documented home for
    /// any future "this combination is nonsensical" rule.
    ///
    /// Probe rules:
    /// - `quorum_n > 0` (a 0 quorum is always-Clear, nonsense as a
    ///   classifier).
    /// - `quorum_n <= endpoints.len()` (otherwise the round can never
    ///   reach Clear and `proteus probe` returns Inconclusive forever).
    pub fn validate(&self) -> Result<()> {
        if self.probes.quorum_n == 0 {
            anyhow::bail!("[probes] quorum_n must be > 0");
        }
        if self.probes.quorum_n as usize > self.probes.endpoints.len() {
            anyhow::bail!(
                "[probes] quorum_n ({}) exceeds endpoint count ({}); the round can never reach Clear",
                self.probes.quorum_n,
                self.probes.endpoints.len()
            );
        }
        Ok(())
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
            events: EventsConfig::default(),
            backend: BackendConfig::default(),
            logging: LoggingConfig::default(),
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
                keep_iaid_stable_across_rotation: Some(self.dhcp.keep_iaid_stable_across_rotation),
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
            events: Some(RawEventsConfig {
                enabled: Some(self.events.enabled),
                portal_poll_secs: Some(self.events.portal_poll_secs),
                link_flap_window_secs: Some(self.events.link_flap_window_secs),
            }),
            backend: Some(RawBackendConfig {
                driver: Some(self.backend.driver.clone()),
            }),
            logging: Some(RawLoggingConfig {
                identifiers: Some(self.logging.identifiers.clone()),
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
#[serde(default, deny_unknown_fields)]
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
    pub events: Option<RawEventsConfig>,
    pub backend: Option<RawBackendConfig>,
    pub logging: Option<RawLoggingConfig>,
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
        // Logging redaction is a cross-cutting safety concern, not a
        // hardening *feature*, so it must apply even under `profile =
        // "off"`. Merge it before the Off short-circuit so an operator
        // who runs Proteus-off-but-logging-on still gets redaction.
        if let Some(l) = &self.logging
            && let Some(v) = &l.identifiers
        {
            cfg.logging.identifiers = v.clone();
        }
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
            if let Some(v) = d.keep_iaid_stable_across_rotation {
                cfg.dhcp.keep_iaid_stable_across_rotation = v;
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
        if let Some(e) = self.events {
            if let Some(v) = e.enabled {
                cfg.events.enabled = v;
            }
            if let Some(v) = e.portal_poll_secs {
                cfg.events.portal_poll_secs = v;
            }
            if let Some(v) = e.link_flap_window_secs {
                cfg.events.link_flap_window_secs = v;
            }
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

    /// Roadmap #404: report which **section** of the resolved config has at
    /// least one user-supplied override on top of the profile baseline.
    /// Returned values are section names (e.g. `"mac"`, `"timers"`); a
    /// section appears in the map iff the user explicitly set at least one
    /// of its fields in `config.toml`. Used by `proteus config show
    /// --annotate` to label each section with `file` vs `profile:<name>`
    /// vs `default`.
    ///
    /// Per-SSID entries are not reported here — they are surfaced separately
    /// via `per_ssid` map keys (one entry per SSID), since each entry has its
    /// own provenance label (`per-ssid:<ssid>`).
    pub fn explicit_sections(&self) -> std::collections::BTreeSet<&'static str> {
        let mut out = std::collections::BTreeSet::new();
        macro_rules! mark_if_any {
            ($name:expr, $section:expr, [$($field:ident),+ $(,)?]) => {
                if let Some(s) = $section
                    && ( $( s.$field.is_some() )||+ )
                {
                    out.insert($name);
                }
            };
        }
        mark_if_any!("mac", &self.mac, [enabled, rotation_interval, oui_pool]);
        mark_if_any!(
            "bluetooth",
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
        mark_if_any!(
            "hostname",
            &self.hostname,
            [enabled, mode, pinned_value, rotate_with_mac]
        );
        mark_if_any!("dns", &self.dns, [strip_edns_client_subnet]);
        mark_if_any!("resolved", &self.resolved, [mdns_off, llmnr_off]);
        mark_if_any!("ntp", &self.ntp, [enabled, ntp_servers, fallback_servers]);
        mark_if_any!(
            "nft",
            &self.nft,
            [icmpv4_timestamp_drop, broadcast_ping_drop, igmp_query_drop]
        );
        mark_if_any!(
            "discovery",
            &self.discovery,
            [mdns_silence, llmnr_silence, ssdp_block, wsd_block]
        );
        mark_if_any!(
            "probes",
            &self.probes,
            [quorum_n, quorum_total, interval, cooldown, endpoints]
        );
        mark_if_any!(
            "ipv6",
            &self.ipv6,
            [enabled, use_temp_addresses, addr_gen_mode, ndp_hardening]
        );
        mark_if_any!(
            "enterprise_wifi",
            &self.enterprise_wifi,
            [
                anonymous_outer_identity,
                realm_strip_strategy,
                anonymous_realm
            ]
        );
        mark_if_any!(
            "stack",
            &self.stack,
            [
                tcp_timestamps_off,
                icmpv6_hardening,
                suppress_gratuitous_arp,
                icmp_info_replies_drop
            ]
        );
        mark_if_any!(
            "dhcp",
            &self.dhcp,
            [
                enabled,
                suppress_hostname,
                suppress_vendor_class,
                rotate_client_id,
                renew_on_apply,
                keep_iaid_stable_across_rotation
            ]
        );
        mark_if_any!(
            "captive_portal",
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
        mark_if_any!(
            "rf",
            &self.rf,
            [tx_power_reduce, tx_power_reduction_db, scan_random_mac]
        );
        if let Some(t) = &self.timers
            && ((t
                .rotate
                .as_ref()
                .and_then(|r| r.interval.as_ref())
                .is_some())
                || (t.check.as_ref().and_then(|c| c.interval.as_ref()).is_some()))
        {
            out.insert("timers");
        }
        if let Some(p) = &self.persona
            && p.active.is_some()
        {
            out.insert("persona");
        }
        mark_if_any!(
            "events",
            &self.events,
            [enabled, portal_poll_secs, link_flap_window_secs]
        );
        mark_if_any!("backend", &self.backend, [driver]);
        mark_if_any!("logging", &self.logging, [identifiers]);
        out
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
                renew_on_apply,
                keep_iaid_stable_across_rotation
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
        any_some!(
            &self.events,
            [enabled, portal_poll_secs, link_flap_window_secs]
        );
        any_some!(&self.backend, [driver]);
        any_some!(&self.logging, [identifiers]);
        if !self.per_ssid.is_empty() {
            return true;
        }
        false
    }

    /// Issue #227: enforce numeric ranges and structural sanity on the
    /// user-supplied raw values before resolution. Catches `quorum_n =
    /// 0` (always-Clear, defeats connectivity-loss detection),
    /// `timeout_secs = u64::MAX` (multi-millennium hang), oversized
    /// `tx_power_reduction_db`, empty endpoint pools, and unparseable
    /// timer / probe durations. Profile baselines are trusted; only the
    /// user's explicit overrides are checked here. The `Config::validate`
    /// post-resolve step then enforces cross-field invariants like
    /// `quorum_n <= endpoints.len()`.
    pub fn validate_ranges(&self) -> Result<()> {
        // ---- discovery: nothing numeric, nothing to validate ----
        // ---- mac: oui_pool may be empty (caller falls back), ranges trivial ----
        if let Some(m) = &self.mac {
            if let Some(p) = &m.oui_pool
                && p.is_empty()
            {
                anyhow::bail!("[mac] oui_pool must not be empty (omit the key to use defaults)");
            }
            if let Some(s) = &m.rotation_interval {
                validate_timer_interval("mac.rotation_interval", s)?;
            }
        }
        // ---- bluetooth: alias_source must be one of the known strings ----
        if let Some(b) = &self.bluetooth
            && let Some(s) = &b.alias_source
            && !matches!(s.as_str(), "generic" | "pinned" | "wordlist")
        {
            anyhow::bail!(
                "[bluetooth] alias_source '{s}' must be one of: generic, pinned, wordlist"
            );
        }
        // ---- hostname: mode must be one of the known strings ----
        if let Some(h) = &self.hostname {
            if let Some(m) = &h.mode
                && !matches!(m.as_str(), "wordlist" | "pinned" | "kernel")
            {
                anyhow::bail!("[hostname] mode '{m}' must be one of: wordlist, pinned, kernel");
            }
            if let Some(v) = &h.pinned_value
                && v.trim().is_empty()
            {
                anyhow::bail!("[hostname] pinned_value must not be empty");
            }
        }
        // ---- ipv6 ----
        if let Some(i) = &self.ipv6
            && let Some(m) = &i.addr_gen_mode
            && !matches!(m.as_str(), "stable-privacy" | "eui64" | "random")
        {
            anyhow::bail!(
                "[ipv6] addr_gen_mode '{m}' must be one of: stable-privacy, eui64, random"
            );
        }
        // ---- enterprise_wifi ----
        if let Some(e) = &self.enterprise_wifi
            && let Some(s) = &e.realm_strip_strategy
            && !matches!(s.as_str(), "auto" | "manual")
        {
            anyhow::bail!(
                "[enterprise_wifi] realm_strip_strategy '{s}' must be one of: auto, manual"
            );
        }
        // ---- ntp ----
        if let Some(n) = &self.ntp {
            if let Some(s) = &n.ntp_servers
                && s.is_empty()
            {
                anyhow::bail!("[ntp] ntp_servers must not be empty (omit the key to use defaults)");
            }
            if let Some(s) = &n.fallback_servers
                && s.is_empty()
            {
                anyhow::bail!(
                    "[ntp] fallback_servers must not be empty (omit the key to use defaults)"
                );
            }
        }
        // ---- probes ----
        if let Some(p) = &self.probes {
            if let Some(n) = p.quorum_n
                && n == 0
            {
                anyhow::bail!(
                    "[probes] quorum_n must be >= 1 (a 0 quorum makes connectivity-loss detection trivially pass)"
                );
            }
            if let Some(t) = p.quorum_total
                && t == 0
            {
                anyhow::bail!("[probes] quorum_total must be >= 1");
            }
            if let (Some(n), Some(t)) = (p.quorum_n, p.quorum_total)
                && n > t
            {
                anyhow::bail!("[probes] quorum_n ({n}) must not exceed quorum_total ({t})");
            }
            if let Some(e) = &p.endpoints
                && e.is_empty()
            {
                anyhow::bail!(
                    "[probes] endpoints must not be empty (omit the key to use defaults)"
                );
            }
            if let Some(s) = &p.interval {
                validate_timer_interval("probes.interval", s)?;
            }
            if let Some(s) = &p.cooldown {
                validate_timer_interval("probes.cooldown", s)?;
            }
        }
        // ---- captive_portal ----
        if let Some(c) = &self.captive_portal {
            if let Some(t) = c.timeout_secs {
                if t == 0 {
                    anyhow::bail!("[captive_portal] timeout_secs must be >= 1");
                }
                if t > 86_400 {
                    anyhow::bail!(
                        "[captive_portal] timeout_secs ({t}) exceeds 86400 (1 day); pick a sane HTTP timeout"
                    );
                }
            }
            if let Some(p) = &c.policy
                && !matches!(p.as_str(), "rotate-before-auth" | "preserve-mac" | "ask")
            {
                anyhow::bail!(
                    "[captive_portal] policy '{p}' must be one of: rotate-before-auth, preserve-mac, ask"
                );
            }
        }
        // ---- rf ----
        if let Some(r) = &self.rf
            && let Some(db) = r.tx_power_reduction_db
            && db > 30
        {
            anyhow::bail!(
                "[rf] tx_power_reduction_db ({db}) exceeds 30 dB; hardware caps and 30 dB is already extreme"
            );
        }
        // ---- timers ----
        if let Some(t) = &self.timers {
            if let Some(r) = &t.rotate
                && let Some(s) = &r.interval
            {
                validate_timer_interval("timers.rotate.interval", s)?;
            }
            if let Some(c) = &t.check
                && let Some(s) = &c.interval
            {
                validate_timer_interval("timers.check.interval", s)?;
            }
        }
        // ---- persona ----
        if let Some(p) = &self.persona
            && let Some(a) = &p.active
            && a.trim().is_empty()
        {
            anyhow::bail!("[persona] active must not be empty (omit the key for no persona)");
        }
        // ---- events ----
        if let Some(e) = &self.events {
            if let Some(s) = e.portal_poll_secs {
                if s == 0 {
                    anyhow::bail!("[events] portal_poll_secs must be >= 1");
                }
                if s > 86_400 {
                    anyhow::bail!(
                        "[events] portal_poll_secs ({s}) exceeds 86400 (1 day); the captive-portal sampler should poll more often"
                    );
                }
            }
            if let Some(s) = e.link_flap_window_secs {
                if s == 0 {
                    anyhow::bail!("[events] link_flap_window_secs must be >= 1");
                }
                if s > 3_600 {
                    anyhow::bail!(
                        "[events] link_flap_window_secs ({s}) exceeds 3600 (1 hour); a flap window longer than that is meaningless"
                    );
                }
            }
        }
        // ---- backend ----
        // Driver string is validated softly in `resolve()` (warns and falls
        // back); harden it here to a hard reject so a typo can't silently
        // make `[backend].driver` ineffective.
        if let Some(b) = &self.backend
            && let Some(d) = &b.driver
            && !crate::backend::select::is_valid_driver(d)
        {
            anyhow::bail!("[backend] driver '{d}' must be one of: auto, nm, networkd, raw");
        }
        // ---- persona: V6 / GH#345 / #339 — validate `[persona] active`
        // against the built-in catalogue with a closest-match suggestion.
        // User personas live in `/etc/proteus/personas/` and are accepted
        // unconditionally because the loader resolves them at apply time;
        // we only want to catch typos against the deterministic shipped
        // catalogue.
        if let Some(p) = &self.persona
            && let Some(active) = &p.active
        {
            check_persona_id_known("persona.active", active)?;
        }
        // ---- backend ----
        // (already done above; left here so the section ordering reads
        // top-to-bottom but the actual check sits earlier.)

        // ---- logging (roadmap 1.0.5) ----
        // The identifier-redaction policy must be one of the three known
        // forms. A typo here would silently fall back to the safe default
        // at `set_policy` time, but a hard reject surfaces it in `proteus
        // doctor` so the operator knows their `full-view` debug request
        // never took effect.
        if let Some(l) = &self.logging
            && let Some(s) = &l.identifiers
            && crate::redaction::parse(s).is_none()
        {
            anyhow::bail!("[logging] identifiers '{s}' must be one of: off, redacted, full-view");
        }

        // ---- per_ssid: per-entry sanity ----
        for (ssid, policy) in &self.per_ssid {
            if let Some(p) = &policy.aggressiveness_profile
                && Profile::parse(p).is_none()
            {
                let suggestion = closest_match(p, &profile_names())
                    .map(|s| format!(" — did you mean '{s}'?"))
                    .unwrap_or_default();
                anyhow::bail!(
                    "[per_ssid.\"{ssid}\"] aggressiveness_profile '{p}' must be one of: off, min, low, med, high, agr{suggestion}"
                );
            }
            if let Some(s) = &policy.rotate_interval {
                // Same accepted shapes as the resolver in per_ssid.rs.
                if !is_valid_per_ssid_duration(s) {
                    anyhow::bail!(
                        "[per_ssid.\"{ssid}\"] rotate_interval '{s}' must be like '30s', '5m', '2h', '1d'"
                    );
                }
            }
            if let Some(p) = &policy.persona {
                if p.trim().is_empty() {
                    anyhow::bail!("[per_ssid.\"{ssid}\"] persona must not be empty");
                }
                check_persona_id_known(&format!("per_ssid.\"{ssid}\".persona"), p)?;
            }
            if let Some(m) = &policy.pin_mac {
                // V7: validate pin_mac at load so a hand-edited typo
                // (`pin_mac = "aa:bb:cc:dd:ee"` — 5 octets) fails at the
                // user, not later when the orchestrator hands it to
                // `Mac::from_str`.
                if m.parse::<crate::mac::Mac>().is_err() {
                    anyhow::bail!(
                        "[per_ssid.\"{ssid}\"] pin_mac '{m}' is not a valid 6-octet MAC (expected 'aa:bb:cc:dd:ee:ff' or dash/none separator)"
                    );
                }
            }
        }
        Ok(())
    }
}

/// V2 / V6: known profile names for closest-match suggestions on a typo.
fn profile_names() -> Vec<&'static str> {
    Profile::all().iter().map(|p| p.name()).collect()
}

/// V6 / GH#345 / #339: validate a persona id against the built-in
/// catalogue, surfacing a closest-match suggestion when the id is unknown
/// AND looks like a near-miss. Empty / whitespace-only ids are rejected
/// at the call site (existing behaviour). User personas under
/// `/etc/proteus/personas/` are accepted unconditionally — see
/// `persona::load::builtin_ids` for the rationale on why we don't I/O the
/// user-root at config-validation time.
fn check_persona_id_known(field: &str, id: &str) -> Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("[{field}] persona id must not be empty");
    }
    let known = crate::persona::load::builtin_ids();
    if known.contains(&trimmed) {
        return Ok(());
    }
    // Non-builtin id: accept silently (it may be a user persona). Only
    // surface a suggestion when the id is *close to* a known builtin —
    // a clear typo signal — by emitting a tracing::warn so the operator
    // sees the candidate in -v mode without breaking apply.
    if let Some(suggestion) = closest_match(trimmed, &known) {
        tracing::warn!(
            field = field,
            persona = trimmed,
            did_you_mean = suggestion,
            "persona id is not a built-in; treating as user persona but the closest builtin is suggested in case of typo"
        );
    }
    Ok(())
}

/// Closest-match helper used by the validation suggestions. Returns the
/// candidate with the smallest Levenshtein distance to `needle`, capped
/// at 3 — beyond that the suggestion is more confusing than helpful.
/// Empty `haystack` returns `None`.
///
/// Issue #394: exposed crate-wide so `proteus config explain` can hand the
/// same "did-you-mean" line to operators on an unknown key.
pub(crate) fn closest_match<'a>(needle: &str, haystack: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for cand in haystack {
        let d = levenshtein(needle, cand);
        if d > 3 {
            continue;
        }
        match best {
            Some((_, bd)) if d >= bd => {}
            _ => best = Some((cand, d)),
        }
    }
    best.map(|(c, _)| c)
}

/// Plain Levenshtein distance. O(len_a * len_b) memory, fine for our
/// short ids and profile names. Not exposed publicly.
fn levenshtein(a: &str, b: &str) -> usize {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let (n, m) = (av.len(), bv.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// Issue #227: thin wrapper over `timer::parse_interval` that also
/// allows the documented `"never"` sentinel which only the timers
/// section honours. Returns an error tagged with the originating field
/// so the user lands on the right key.
fn validate_timer_interval(field: &str, s: &str) -> Result<()> {
    if crate::timer::is_never(s) {
        // Only `[timers.*]` honours `never`; reject everywhere else.
        if field.starts_with("timers.") {
            return Ok(());
        }
        anyhow::bail!("[{field}] 'never' is only valid under [timers.*]");
    }
    crate::timer::parse_interval(s)
        .map(|_| ())
        .with_context(|| format!("[{field}] '{s}'"))
}

/// Issue #257 / per-SSID resolver: same compact-duration grammar
/// `per_ssid.rs::parse_duration` accepts (`30s`/`5m`/`2h`/`1d`).
/// Centralised so the load-time validator and the SSID `set` writer
/// stay in lock-step.
///
/// Issue N12.5 / GH#272 sibling: previously this used
/// `s.split_at(s.len() - 1)` which slices on a byte boundary. A trailing
/// multibyte UTF-8 character (e.g. `5µ`) lands the split mid-codepoint and
/// panics. With `panic = abort` set crate-wide a hostile or hand-edited
/// `[per_ssid.<x>] rotate_interval = "5µ"` would abort the process at
/// config-load time. Split on the last *char* boundary via `char_indices`
/// and reject any non-ASCII suffix as off-format.
///
/// P1: a length check via `s.len() < 2` is byte-based; an input that is a
/// single multibyte character has byte-length >= 2 and would have passed
/// the old guard. The `char_indices` approach inherently handles that
/// case — `next_back()` returns `None` on empty, and the unit-length check
/// rejects multibyte suffixes — so the function never reaches the
/// would-panic split.
pub(crate) fn is_valid_per_ssid_duration(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // Find the last char boundary; bail if there is no char (empty after trim).
    let Some((last_idx, _)) = s.char_indices().next_back() else {
        return false;
    };
    if last_idx == 0 {
        // Single character only — no numeric prefix, off-format.
        return false;
    }
    let (num, unit) = s.split_at(last_idx);
    if unit.len() != 1 || !unit.is_ascii() {
        return false;
    }
    let Ok(n) = num.parse::<u64>() else {
        return false;
    };
    if n == 0 {
        return false;
    }
    matches!(unit, "s" | "m" | "h" | "d")
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawMacConfig {
    pub enabled: Option<bool>,
    pub rotation_interval: Option<String>,
    pub oui_pool: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawBluetoothConfig {
    pub enabled: Option<bool>,
    pub generic_alias: Option<bool>,
    pub alias_source: Option<String>,
    pub pinned_alias: Option<String>,
    pub discoverable: Option<bool>,
    pub ble_rpa: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawHostnameConfig {
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub pinned_value: Option<String>,
    pub rotate_with_mac: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawDnsConfig {
    pub strip_edns_client_subnet: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawResolvedConfig {
    pub mdns_off: Option<bool>,
    pub llmnr_off: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawNtpConfig {
    pub enabled: Option<bool>,
    pub ntp_servers: Option<Vec<String>>,
    pub fallback_servers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawNftConfig {
    pub icmpv4_timestamp_drop: Option<bool>,
    pub broadcast_ping_drop: Option<bool>,
    pub igmp_query_drop: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawDiscoveryConfig {
    pub mdns_silence: Option<bool>,
    pub llmnr_silence: Option<bool>,
    pub ssdp_block: Option<bool>,
    pub wsd_block: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawProbesConfig {
    pub quorum_n: Option<u8>,
    pub quorum_total: Option<u8>,
    pub interval: Option<String>,
    pub cooldown: Option<String>,
    pub endpoints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawIpv6Config {
    pub enabled: Option<bool>,
    pub use_temp_addresses: Option<bool>,
    pub addr_gen_mode: Option<String>,
    pub ndp_hardening: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawEnterpriseWifiConfig {
    pub anonymous_outer_identity: Option<bool>,
    pub realm_strip_strategy: Option<String>,
    pub anonymous_realm: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawStackConfig {
    pub tcp_timestamps_off: Option<bool>,
    pub icmpv6_hardening: Option<bool>,
    pub suppress_gratuitous_arp: Option<bool>,
    pub icmp_info_replies_drop: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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
    /// NBE.3: see `DhcpConfig::keep_iaid_stable_across_rotation`.
    pub keep_iaid_stable_across_rotation: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawCaptivePortalConfig {
    pub enabled: Option<bool>,
    pub detect_url: Option<String>,
    pub expected_response: Option<String>,
    pub policy: Option<String>,
    pub fresh_mac_per_visit: Option<bool>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawRfConfig {
    pub tx_power_reduce: Option<bool>,
    pub tx_power_reduction_db: Option<u8>,
    /// Roadmap Milestone 4b — see `RfConfig::scan_random_mac`.
    pub scan_random_mac: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawTimersConfig {
    pub rotate: Option<RawTimerConfig>,
    pub check: Option<RawTimerConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawTimerConfig {
    pub interval: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawPersonaConfig {
    pub active: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawBackendConfig {
    pub driver: Option<String>,
}

/// Raw on-disk shape of `[logging]`. Roadmap 1.0.5.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawLoggingConfig {
    pub identifiers: Option<String>,
}

/// Raw on-disk shape of `[events]`. Roadmap Milestone 4c.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawEventsConfig {
    pub enabled: Option<bool>,
    pub portal_poll_secs: Option<u64>,
    pub link_flap_window_secs: Option<u64>,
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
    /// NBE.3: DUID/IAID asymmetry on rotate.
    ///
    /// NM's `ipv6.dhcp-duid = "ll"` derives the DHCPv6 DUID from the
    /// MAC's link-layer address — which means the DUID is reborn on
    /// every rotation. NM's `ipv6.dhcp-iaid = "mac"` does the same
    /// for the IAID (DHCPv6 identifier per interface association).
    /// Both rotating together is the strongest unlinkability story
    /// but also breaks DHCPv6-only networks that hand out stable
    /// leases keyed by (DUID, IAID).
    ///
    /// The tradeoff:
    ///
    /// - Default (`false`): DUID + IAID both rotate, full
    ///   unlinkability, broken stable-DHCPv6.
    /// - `true`: pin IAID to the DUID derivation (NM's `"stable"`
    ///   mode), keeping the IAID stable across rotations while the
    ///   DUID itself rotates. The DHCPv6 server still sees a fresh
    ///   client identity (DUID changes) but the per-iface IAID stays
    ///   constant so a sticky-pool server can re-issue the same
    ///   lease. Slight unlinkability cost for compatibility on
    ///   IPv6-stable networks.
    ///
    /// The knob ships now so the schema is stable; the wire impl
    /// (writing `"stable"` instead of `"mac"` on `ipv6.dhcp-iaid`
    /// when the knob is set) lands on the same backend `Update` path
    /// the existing `apply_dhcp_settings` uses.
    pub keep_iaid_stable_across_rotation: bool,
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
#[serde(default, deny_unknown_fields)]
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

/// Event-driven trigger framework. Roadmap Milestone 4c.
///
/// Default `enabled = false`: the daemon (`proteus events run`) only
/// kicks in once the operator opts in. Once on, the four
/// `EventSource` impls run in a tokio task and feed
/// `RotationTrigger`s through the in-process `EventRegistry`. Each
/// source gracefully degrades when the host can't honour it (no
/// `CAP_NET_ADMIN`, no NM on the bus, no nl80211 in the kernel) so
/// flipping the master switch is safe even on partial platforms.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EventsConfig {
    /// Master switch. Off by default for v0.3.x — the
    /// `proteus-events.service` unit checks this knob and refuses
    /// to start if it's `false`, so the long-lived daemon stays
    /// dormant on a stock install.
    pub enabled: bool,
    /// Captive-portal sampler poll cadence in seconds. Default 30 s
    /// matches the `[captive_portal] timeout_secs` budget without
    /// stacking timeouts. Below 5 s wastes battery; above 300 s
    /// misses portal-auth windows on hostile networks.
    pub portal_poll_secs: u64,
    /// Window in seconds inside which two `down→up` carrier
    /// transitions count as a "flap." Default 10 s — long enough to
    /// absorb the kernel's bring-up jitter, short enough to pin a
    /// real roam.
    pub link_flap_window_secs: u64,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            portal_poll_secs: 30,
            link_flap_window_secs: 10,
        }
    }
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

/// Logging-layer identifier-redaction policy. Roadmap 1.0.5.
///
/// `identifiers` is one of `off` | `redacted` | `full-view` and controls
/// how device identifiers (MAC / SSID / hostname / 802.1X) are rendered at
/// log sites. The default `"redacted"` is safe (real values never reach
/// journald / stderr); `"full-view"` is the only weakening form and is
/// opt-in, warns once at startup, and is documented in `config explain`.
/// `--json` output and CLI display are unaffected — they always show real
/// values. See `crate::redaction`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub identifiers: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            identifiers: "redacted".into(),
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
            keep_iaid_stable_across_rotation: false,
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

    /// Roadmap 1.0.5: the default `[logging] identifiers` is the safe
    /// `redacted` form — real values must never reach logs unless the
    /// operator explicitly opts into `full-view`.
    #[test]
    fn logging_identifiers_defaults_to_redacted() {
        let cfg = Config::default();
        assert_eq!(cfg.logging.identifiers, "redacted");
    }

    #[test]
    fn logging_section_round_trips_through_toml() {
        let toml_str = r#"
profile = "med"

[logging]
identifiers = "full-view"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.has_overrides());
        let cfg = raw.resolve();
        assert_eq!(cfg.logging.identifiers, "full-view");
        let raw2 = cfg.to_raw_explicit();
        let s = toml::to_string(&raw2).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        assert_eq!(back.logging.identifiers, "full-view");
    }

    /// Redaction must apply even when `profile = "off"` — it is a
    /// cross-cutting safety concern, not a hardening feature, so the Off
    /// short-circuit must not drop the logging override.
    #[test]
    fn logging_override_survives_profile_off() {
        let toml_str = r#"
profile = "off"

[logging]
identifiers = "off"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.profile, Profile::Off);
        assert_eq!(cfg.logging.identifiers, "off");
    }

    #[test]
    fn validate_ranges_rejects_unknown_logging_identifiers() {
        let raw: RawConfig = toml::from_str("[logging]\nidentifiers = \"loud\"\n").unwrap();
        let err = raw.validate_ranges().unwrap_err().to_string();
        assert!(err.contains("off, redacted, full-view"), "got: {err}");
    }

    #[test]
    fn validate_ranges_accepts_known_logging_identifiers() {
        for v in ["off", "redacted", "full-view"] {
            let raw: RawConfig =
                toml::from_str(&format!("[logging]\nidentifiers = \"{v}\"\n")).unwrap();
            assert!(
                raw.validate_ranges().is_ok(),
                "'{v}' should be a valid logging.identifiers value"
            );
        }
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

    /// Milestone 4c: `[events]` defaults are off + 30 / 10. The
    /// systemd unit refuses to start when `enabled = false`, so this
    /// is the load-bearing default that keeps the daemon dormant on
    /// stock installs.
    #[test]
    fn events_defaults_are_off_and_baseline_cadence() {
        let cfg = Config::default();
        assert!(!cfg.events.enabled, "events daemon must be off by default");
        assert_eq!(cfg.events.portal_poll_secs, 30);
        assert_eq!(cfg.events.link_flap_window_secs, 10);
    }

    /// `[events]` round-trips through TOML losslessly.
    #[test]
    fn events_section_round_trips_through_toml() {
        let toml_str = r#"
profile = "med"

[events]
enabled = true
portal_poll_secs = 60
link_flap_window_secs = 5
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(
            raw.has_overrides(),
            "non-default events fields must trip has_overrides"
        );
        let cfg = raw.resolve();
        assert!(cfg.events.enabled);
        assert_eq!(cfg.events.portal_poll_secs, 60);
        assert_eq!(cfg.events.link_flap_window_secs, 5);
        let raw_back = cfg.to_raw_explicit();
        let s = toml::to_string(&raw_back).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        assert!(back.events.enabled);
        assert_eq!(back.events.portal_poll_secs, 60);
        assert_eq!(back.events.link_flap_window_secs, 5);
    }

    /// `Profile::Off` short-circuits user `[events]` overrides — the
    /// master switch must stay off when the panic-disable profile is
    /// active, even if the operator left `enabled = true` in the
    /// file. Mirrors the same contract every other section honours.
    #[test]
    fn events_off_profile_keeps_daemon_disabled() {
        let toml_str = r#"
profile = "off"

[events]
enabled = true
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert!(
            !cfg.events.enabled,
            "Profile::Off must keep events daemon disabled regardless of user overrides"
        );
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
        assert_eq!(
            entry_back.portal_policy.as_deref(),
            Some("fresh-mac-per-visit")
        );
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

    // ---- Issue #227: deny_unknown_fields + validate_ranges --------------

    /// A typo on a top-level section name fails at parse time. Without
    /// `deny_unknown_fields` on `RawConfig` the section would be
    /// silently ignored.
    #[test]
    fn unknown_top_level_section_is_rejected() {
        let toml_str = "[rotation]\ninterval = \"2h\"\n";
        let err = toml::from_str::<RawConfig>(toml_str).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown-field error, got: {err}"
        );
    }

    /// A typo on a per-section field name fails at parse time. The
    /// canonical key for "block multicast DNS" is `mdns_silence`; the
    /// example files used to write `mdns_responder = false` which was
    /// silently ignored. The new behaviour fails loud.
    #[test]
    fn unknown_field_inside_section_is_rejected() {
        let toml_str = "[discovery]\nmdns_responder = false\n";
        let err = toml::from_str::<RawConfig>(toml_str).unwrap_err();
        assert!(
            err.to_string().contains("unknown field") || err.to_string().contains("mdns_responder"),
            "expected unknown-field error, got: {err}"
        );
    }

    /// `quorum_n = 0` is the issue-#227 example: the round can never
    /// fail, so connectivity-loss detection silently breaks.
    #[test]
    fn validate_ranges_rejects_zero_probe_quorum() {
        let raw: RawConfig = toml::from_str("[probes]\nquorum_n = 0\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("quorum_n"));
    }

    /// `quorum_n > quorum_total` is a contradictory pair; reject early.
    #[test]
    fn validate_ranges_rejects_quorum_n_above_total() {
        let raw: RawConfig = toml::from_str("[probes]\nquorum_n = 5\nquorum_total = 3\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("quorum_n"));
    }

    /// Issue #227 motivating example: a wildly oversized timeout
    /// (here a year in seconds) would let a captive-portal probe hang
    /// effectively forever. Reject anything beyond 1 day.
    #[test]
    fn validate_ranges_rejects_oversized_captive_portal_timeout() {
        let toml_str = "[captive_portal]\ntimeout_secs = 31536000\n";
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("timeout_secs"));
    }

    /// `timeout_secs = 0` would mean "abandon the probe immediately" —
    /// nonsense for a portal detector. Reject.
    #[test]
    fn validate_ranges_rejects_zero_captive_portal_timeout() {
        let raw: RawConfig = toml::from_str("[captive_portal]\ntimeout_secs = 0\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    /// Captive-portal `policy` is a closed enum; a typo must fail.
    #[test]
    fn validate_ranges_rejects_unknown_captive_portal_policy() {
        let raw: RawConfig = toml::from_str("[captive_portal]\npolicy = \"yolo\"\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("policy"));
    }

    /// `tx_power_reduction_db > 30` is unphysical and probably a typo
    /// (e.g. user meant `3` and wrote `300`, but `u8` max is `255`).
    /// We cap at 30 so wild values bail.
    #[test]
    fn validate_ranges_rejects_extreme_tx_power_reduction() {
        let raw: RawConfig = toml::from_str("[rf]\ntx_power_reduction_db = 100\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("tx_power_reduction_db"));
    }

    /// `[events] portal_poll_secs = 0` would burn a CPU; reject.
    #[test]
    fn validate_ranges_rejects_zero_portal_poll_secs() {
        let raw: RawConfig = toml::from_str("[events]\nportal_poll_secs = 0\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    /// `[events] link_flap_window_secs > 1 hour` is meaningless.
    #[test]
    fn validate_ranges_rejects_oversized_link_flap_window() {
        let raw: RawConfig = toml::from_str("[events]\nlink_flap_window_secs = 7200\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    /// Garbage timer interval is rejected at validation time. The
    /// existing `parse_interval` takes care of the underlying check;
    /// the wrapper just labels the field.
    #[test]
    fn validate_ranges_rejects_garbage_timer_interval() {
        let raw: RawConfig = toml::from_str("[timers.rotate]\ninterval = \"garbage\"\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("timers.rotate.interval"));
    }

    /// `"never"` is the documented sentinel that disables a timer; the
    /// validator must accept it under `[timers.*]`.
    #[test]
    fn validate_ranges_accepts_timer_never() {
        let raw: RawConfig = toml::from_str("[timers.rotate]\ninterval = \"never\"\n").unwrap();
        raw.validate_ranges().unwrap();
    }

    /// `"never"` outside `[timers.*]` is meaningless and must reject.
    #[test]
    fn validate_ranges_rejects_never_under_probes() {
        let raw: RawConfig = toml::from_str("[probes]\ninterval = \"never\"\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    /// A garbage `[backend] driver` was previously soft-rejected by
    /// `resolve()` (warns and falls back to `auto`). The validator
    /// hardens this to a hard reject so the user can't mistype the
    /// driver name and silently end up on `auto`.
    #[test]
    fn validate_ranges_rejects_unknown_backend_driver() {
        let raw: RawConfig = toml::from_str("[backend]\ndriver = \"garbage\"\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("driver"));
    }

    /// `[per_ssid.x] aggressiveness_profile = "junk"` rejects: the
    /// resolver would otherwise silently fall through to the global
    /// profile, hiding the typo.
    #[test]
    fn validate_ranges_rejects_unknown_per_ssid_profile() {
        let toml_str = r#"
[per_ssid."home"]
aggressiveness_profile = "junk"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("aggressiveness_profile"));
    }

    /// `[per_ssid.x] rotate_interval` only honours `30s/5m/2h/1d`; a
    /// minute-suffix typo or empty unit must reject.
    #[test]
    fn validate_ranges_rejects_garbage_per_ssid_rotate_interval() {
        let toml_str = r#"
[per_ssid."home"]
rotate_interval = "junk"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(err.to_string().contains("rotate_interval"));
    }

    /// `[per_ssid.x]` with an unknown field rejects at parse time
    /// (deny_unknown_fields on `PerSsidPolicy`). Without this, a typo
    /// like `pinned_mac` would be silently dropped.
    #[test]
    fn unknown_field_inside_per_ssid_block_is_rejected() {
        let toml_str = r#"
[per_ssid."home"]
pinned_mac = "aa:bb:cc:dd:ee:ff"
"#;
        let err = toml::from_str::<RawConfig>(toml_str).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    /// Happy path: the documented schema with every section populated
    /// passes validation cleanly.
    #[test]
    fn validate_ranges_accepts_documented_full_config() {
        let toml_str = r#"
profile = "med"

[mac]
enabled = true
rotation_interval = "2h"
oui_pool = ["apple", "intel"]

[discovery]
mdns_silence = true

[probes]
quorum_n = 3
quorum_total = 4
interval = "5m"
cooldown = "60s"

[captive_portal]
enabled = true
policy = "rotate-before-auth"
timeout_secs = 5

[rf]
tx_power_reduce = false
tx_power_reduction_db = 6

[events]
enabled = false
portal_poll_secs = 30
link_flap_window_secs = 10

[timers.rotate]
interval = "2h"

[timers.check]
interval = "5m"

[backend]
driver = "auto"

[per_ssid."coffee-shop"]
persona = "iphone-15"
aggressiveness_profile = "high"
pin_mac = "aa:bb:cc:dd:ee:ff"
rotate_interval = "30m"
portal_policy = "fresh-mac-per-visit"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges()
            .expect("full documented config must validate");
    }

    /// All shipped example configs must parse and validate cleanly so
    /// onboarding stays a copy-paste affair. This is the regression
    /// guard for the docs-vs-code-drift class of bug (issue D1/D2/D3/D4).
    #[test]
    fn every_shipped_example_validates() {
        for name in [
            "examples/standard.toml",
            "examples/aggressive.toml",
            "examples/captive-portal-heavy.toml",
            "examples/development.toml",
            "examples/disabled.toml",
            "examples/minimal.toml",
            "examples/paranoid.toml",
        ] {
            let body = std::fs::read_to_string(name).unwrap_or_else(|e| panic!("read {name}: {e}"));
            let raw: RawConfig =
                toml::from_str(&body).unwrap_or_else(|e| panic!("parse {name}: {e}"));
            raw.validate_ranges()
                .unwrap_or_else(|e| panic!("validate {name}: {e}"));
            // resolve must not panic either
            let cfg = raw.resolve();
            cfg.validate()
                .unwrap_or_else(|e| panic!("resolved validate {name}: {e}"));
        }
    }

    #[test]
    fn is_valid_per_ssid_duration_recognises_each_unit() {
        assert!(is_valid_per_ssid_duration("30s"));
        assert!(is_valid_per_ssid_duration("5m"));
        assert!(is_valid_per_ssid_duration("2h"));
        assert!(is_valid_per_ssid_duration("1d"));
        assert!(!is_valid_per_ssid_duration(""));
        assert!(!is_valid_per_ssid_duration("0s"));
        assert!(!is_valid_per_ssid_duration("xx"));
        assert!(!is_valid_per_ssid_duration("3w"));
        assert!(!is_valid_per_ssid_duration("garbage"));
    }

    /// N12.5 / GH#272 sibling: feeding a multibyte trailing char into
    /// `is_valid_per_ssid_duration` previously panicked at the byte-
    /// boundary `split_at`. With `panic = abort` set crate-wide the
    /// process aborts. The test must exit cleanly and return `false`.
    #[test]
    fn is_valid_per_ssid_duration_rejects_multibyte_without_panic() {
        // Two-byte UTF-8 trailing char.
        assert!(!is_valid_per_ssid_duration("5µ"));
        // Four-byte emoji.
        assert!(!is_valid_per_ssid_duration("5🦀"));
        // Three-byte CJK ideograph.
        assert!(!is_valid_per_ssid_duration("5日"));
        // Lone multibyte char with no numeric prefix.
        assert!(!is_valid_per_ssid_duration("µ"));
        // Multibyte interior plus ASCII suffix — numeric parse fails.
        assert!(!is_valid_per_ssid_duration("1µs"));
    }
}

// =====================================================================
// config::validation_tests — Stream 2 schema-validation regression suite
// =====================================================================
//
// Every test in this module loads a malformed (or load-bearing happy-path)
// TOML sample through `RawConfig::validate_ranges` (and where the rule
// only surfaces post-resolve, through `Config::validate`) and asserts the
// specific error path. The point of having a separate module is so a
// failing assertion lands a clear "schema validation regressed" signal,
// not a generic "config tests are flaky."
//
// `cargo test --release` is the load-bearing run because `panic = abort`
// is set crate-wide; a regression in the multibyte / overflow / split_at
// class would manifest as a SIGABRT during a release-mode test, not a
// graceful failure.
#[cfg(test)]
mod validation_tests {
    use super::*;

    // ---- V1: zero / empty rotation interval rejected at load time ----

    #[test]
    fn v1_zero_seconds_timer_interval_rejected() {
        let raw: RawConfig = toml::from_str("[timers.rotate]\ninterval = \"0s\"\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("timers.rotate.interval") || msg.contains("> 0"),
            "expected zero-interval reject, got: {msg}"
        );
    }

    #[test]
    fn v1_empty_string_timer_interval_rejected() {
        let raw: RawConfig = toml::from_str("[timers.rotate]\ninterval = \"\"\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    #[test]
    fn v1_zero_minutes_timer_interval_rejected() {
        let raw: RawConfig = toml::from_str("[timers.check]\ninterval = \"0m\"\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    #[test]
    fn v1_mac_rotation_interval_zero_rejected() {
        let raw: RawConfig = toml::from_str("[mac]\nrotation_interval = \"0s\"\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    // ---- V3: quorum_n <= quorum_total ----

    #[test]
    fn v3_quorum_above_total_rejected() {
        let raw: RawConfig = toml::from_str("[probes]\nquorum_n = 10\nquorum_total = 4\n").unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(format!("{err:#}").contains("quorum_n"));
    }

    #[test]
    fn v3_quorum_equal_total_accepted() {
        let raw: RawConfig = toml::from_str("[probes]\nquorum_n = 4\nquorum_total = 4\n").unwrap();
        raw.validate_ranges().expect("equal quorum is valid");
    }

    // ---- V4: bound second-precision durations ----

    #[test]
    fn v4_captive_portal_timeout_upper_bound() {
        // 86_401 just past the 1-day cap.
        let toml_str = "[captive_portal]\ntimeout_secs = 86401\n";
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    #[test]
    fn v4_events_link_flap_window_upper_bound() {
        // > 1 hour is meaningless.
        let raw: RawConfig = toml::from_str("[events]\nlink_flap_window_secs = 3601\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    // ---- V5: tx_power_reduction_db ----

    #[test]
    fn v5_tx_power_reduction_db_capped() {
        let raw: RawConfig = toml::from_str("[rf]\ntx_power_reduction_db = 31\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    #[test]
    fn v5_tx_power_reduction_db_at_cap_accepted() {
        let raw: RawConfig = toml::from_str("[rf]\ntx_power_reduction_db = 30\n").unwrap();
        raw.validate_ranges().unwrap();
    }

    // ---- V2 / V6: profile + persona name validation ----

    #[test]
    fn v2_per_ssid_aggressiveness_profile_typo_suggests_correction() {
        let toml_str = r#"
[per_ssid."home"]
aggressiveness_profile = "hgh"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let err = raw.validate_ranges().unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("aggressiveness_profile"));
        assert!(
            msg.contains("did you mean 'high'"),
            "closest-match suggestion missing: {msg}"
        );
    }

    #[test]
    fn v6_persona_active_unknown_id_does_not_hard_fail() {
        // Unknown id is treated as a possible user persona; validation
        // accepts it (the loader will surface the not-found at apply
        // time). The closest-match suggestion is emitted via tracing,
        // not a hard error, so this test only asserts the validation
        // does not bail.
        let raw: RawConfig =
            toml::from_str("[persona]\nactive = \"my-custom-user-persona\"\n").unwrap();
        raw.validate_ranges()
            .expect("user-persona-shaped id must validate");
    }

    #[test]
    fn v6_per_ssid_persona_empty_string_rejected() {
        let toml_str = r#"
[per_ssid."x"]
persona = ""
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    // ---- V7: pin_mac format validation ----

    #[test]
    fn v7_pin_mac_too_few_octets_rejected() {
        let toml_str = r#"
[per_ssid."x"]
pin_mac = "aa:bb:cc:dd:ee"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let err = raw.validate_ranges().unwrap_err();
        assert!(format!("{err:#}").contains("pin_mac"));
    }

    #[test]
    fn v7_pin_mac_garbage_rejected() {
        let toml_str = r#"
[per_ssid."x"]
pin_mac = "not-a-mac"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    #[test]
    fn v7_pin_mac_well_formed_accepted() {
        let toml_str = r#"
[per_ssid."x"]
pin_mac = "aa:bb:cc:dd:ee:ff"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
    }

    #[test]
    fn v7_pin_mac_dash_separator_accepted() {
        let toml_str = r#"
[per_ssid."x"]
pin_mac = "aa-bb-cc-dd-ee-ff"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
    }

    // ---- V10: round-trip coverage for arrays / numerics / enums ----

    /// Probe endpoints (string array), quorum_n / quorum_total
    /// (numeric u8), backend driver (enum-string) — all round-trip
    /// through `to_raw_explicit` and back.
    #[test]
    fn v10_arrays_numerics_enums_round_trip() {
        let toml_str = r#"
profile = "high"

[probes]
quorum_n = 2
quorum_total = 5
endpoints = ["1.2.3.4:443", "5.6.7.8:80"]

[mac]
oui_pool = ["apple", "intel", "samsung"]

[backend]
driver = "nm"

[rf]
tx_power_reduction_db = 12
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let cfg = raw.resolve();
        assert_eq!(cfg.probes.quorum_n, 2);
        assert_eq!(cfg.probes.quorum_total, 5);
        assert_eq!(cfg.probes.endpoints.len(), 2);
        assert_eq!(cfg.mac.oui_pool, vec!["apple", "intel", "samsung"]);
        assert_eq!(cfg.backend.driver, "nm");
        assert_eq!(cfg.rf.tx_power_reduction_db, 12);

        // Round-trip back through TOML.
        let raw2 = cfg.to_raw_explicit();
        let s = toml::to_string(&raw2).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        assert_eq!(back.probes.endpoints, cfg.probes.endpoints);
        assert_eq!(back.probes.quorum_n, cfg.probes.quorum_n);
        assert_eq!(back.probes.quorum_total, cfg.probes.quorum_total);
        assert_eq!(back.mac.oui_pool, cfg.mac.oui_pool);
        assert_eq!(back.backend.driver, cfg.backend.driver);
        assert_eq!(back.rf.tx_power_reduction_db, cfg.rf.tx_power_reduction_db);
        assert_eq!(back.profile, cfg.profile);
    }

    /// Empty array fields are accepted on the resolved side as
    /// "fall through to defaults" by `validate_ranges`'s NTP/probes/mac
    /// rules — but on the raw side an explicit empty list is rejected.
    /// Pin both sides so the contract stays explicit.
    #[test]
    fn v10_empty_array_fields_are_rejected_at_load() {
        let raw: RawConfig = toml::from_str("[mac]\noui_pool = []\n").unwrap();
        assert!(raw.validate_ranges().is_err());
        let raw: RawConfig = toml::from_str("[probes]\nendpoints = []\n").unwrap();
        assert!(raw.validate_ranges().is_err());
        let raw: RawConfig = toml::from_str("[ntp]\nntp_servers = []\n").unwrap();
        assert!(raw.validate_ranges().is_err());
    }

    // ---- V12: SSID-key TOML special-character coverage ----

    /// SSIDs with spaces, dots, dashes, brackets, and backslashes must
    /// round-trip through TOML when escaped per spec (basic-string keys).
    /// The previous test suite only exercised plain kebab-case keys —
    /// this regression guard pins the actual hostile-AP / messy-deployment
    /// shapes operators will encounter.
    #[test]
    fn v12_ssid_keys_with_spaces_round_trip() {
        let toml_str = r#"
[per_ssid."Coffee Shop Wi-Fi"]
aggressiveness_profile = "high"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
        let cfg = raw.resolve();
        assert!(cfg.per_ssid.contains_key("Coffee Shop Wi-Fi"));
        // Round-trip back.
        let raw2 = cfg.to_raw_explicit();
        let s = toml::to_string(&raw2).unwrap();
        let parsed: RawConfig = toml::from_str(&s).unwrap();
        assert!(parsed.per_ssid.contains_key("Coffee Shop Wi-Fi"));
    }

    #[test]
    fn v12_ssid_keys_with_dots_round_trip() {
        let toml_str = r#"
[per_ssid."guest.lan.example"]
pin_mac = "aa:bb:cc:dd:ee:ff"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
        let cfg = raw.resolve();
        assert!(cfg.per_ssid.contains_key("guest.lan.example"));
    }

    #[test]
    fn v12_ssid_keys_with_brackets_round_trip() {
        // Brackets in an SSID need TOML quoting to round-trip.
        let toml_str = r#"
[per_ssid."[guest]"]
aggressiveness_profile = "med"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
        let cfg = raw.resolve();
        assert!(cfg.per_ssid.contains_key("[guest]"));
    }

    #[test]
    fn v12_ssid_keys_with_unicode_round_trip() {
        let toml_str = r#"
[per_ssid."café-📶"]
aggressiveness_profile = "agr"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
        let cfg = raw.resolve();
        assert!(cfg.per_ssid.contains_key("café-📶"));
    }

    #[test]
    fn v12_ssid_keys_with_backslash_escape_round_trip() {
        // TOML basic strings honour `\\` as a backslash escape.
        let toml_str = r#"
[per_ssid."weird\\name"]
aggressiveness_profile = "min"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        raw.validate_ranges().unwrap();
        let cfg = raw.resolve();
        assert!(cfg.per_ssid.contains_key("weird\\name"));
    }

    // ---- closest_match / levenshtein helpers ----

    #[test]
    fn levenshtein_known_distances() {
        assert_eq!(super::levenshtein("kitten", "sitting"), 3);
        assert_eq!(super::levenshtein("abc", "abc"), 0);
        assert_eq!(super::levenshtein("", "abc"), 3);
        assert_eq!(super::levenshtein("abc", ""), 3);
    }

    #[test]
    fn closest_match_picks_nearest_known() {
        let names = vec!["off", "min", "low", "med", "high", "agr"];
        assert_eq!(super::closest_match("hgh", &names), Some("high"));
        assert_eq!(super::closest_match("loww", &names), Some("low"));
        // Way off → no suggestion.
        assert_eq!(super::closest_match("zzzzzzz", &names), None);
    }
}
