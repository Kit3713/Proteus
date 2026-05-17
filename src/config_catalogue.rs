// SPDX-License-Identifier: GPL-3.0-or-later

//! Static documentation catalogue for `proteus config explain <key>` (#394).
//!
//! Each entry maps a dotted config key (e.g. `mac.rotation_interval`) to the
//! user-facing doc string, an optional risk warning, and a wiki cross-link.
//! Type + default still come from the live schema (the `default_document()`
//! helper in `commands::config_cmd`); the catalogue only carries the prose
//! that the `///` doc-comments in `config.rs` already express in source.
//!
//! Why a hand-maintained catalogue rather than auto-deriving from rustdoc
//! JSON: the doc strings here are *operator-facing* (one-paragraph English,
//! risk surface called out, wiki pointer) whereas the in-source `///`
//! comments are *developer-facing* (mechanism, trade-off, issue refs).
//! The two audiences benefit from different framings. The drift cost is
//! contained by the `entries_map_to_real_keys` test in this module: if a
//! future schema rename drops `mac.rotation_interval` from the live keyspace,
//! that test fails and the catalogue entry has to follow in the same PR.
//!
//! See `src/commands/config_cmd.rs::explain` for the renderer.

/// One documentation entry. Kept lean: the catalogue is read at runtime by
/// the explain command, so every field is a borrowed `&'static str` to keep
/// the binary footprint minimal.
#[derive(Debug, Clone, Copy)]
pub struct ExplainEntry {
    /// Dotted key (e.g. `"mac.rotation_interval"`). Must match a leaf in
    /// the live schema; covered by the drift test below.
    pub key: &'static str,
    /// One-paragraph operator-facing description. Soft-wrap is the
    /// renderer's job; keep these as a single line.
    pub doc: &'static str,
    /// Optional risk surface — empty string when the knob has no
    /// documented foot-gun. Surfaced as the `Risk:` line in human output
    /// and as `null` in JSON.
    pub risk: &'static str,
    /// Wiki page basename. Becomes `proteus wiki <page>` in human output.
    /// Empty string when no wiki page covers the key in enough depth.
    pub wiki: &'static str,
}

/// The catalogue itself. Entries are stored as a flat slice so the
/// drift test can iterate over them without a `BTreeMap` allocation.
/// Sorted alphabetically by key for stable diffs.
pub const ENTRIES: &[ExplainEntry] = &[
    // ---- backend ----
    ExplainEntry {
        key: "backend.driver",
        doc: "Network backend selector: 'auto' walks NM, networkd, raw in order; or pin one explicitly.",
        risk: "Pinning to a backend that is not installed makes apply/rotate exit with SYSTEM_NOT_SUPPORTED.",
        wiki: "backend",
    },
    // ---- bluetooth ----
    ExplainEntry {
        key: "bluetooth.alias_source",
        doc: "How the adapter alias is chosen: 'generic' (model-only), 'persona' (active persona), or 'pinned'.",
        risk: "",
        wiki: "bluetooth",
    },
    ExplainEntry {
        key: "bluetooth.ble_rpa",
        doc: "Force Bluetooth LE Resolvable Private Addresses on the adapter. Re-randomized periodically by the controller.",
        risk: "Some accessories (older fitness trackers, car kits) refuse to pair against an RPA.",
        wiki: "bluetooth",
    },
    ExplainEntry {
        key: "bluetooth.discoverable",
        doc: "Whether the adapter advertises itself in inquiry scans. Default off so passersby cannot enumerate the device.",
        risk: "",
        wiki: "bluetooth",
    },
    ExplainEntry {
        key: "bluetooth.enabled",
        doc: "Master switch for the Bluetooth shaping path (alias, discoverable flag, BLE RPA).",
        risk: "",
        wiki: "bluetooth",
    },
    ExplainEntry {
        key: "bluetooth.generic_alias",
        doc: "Replace the adapter alias with a generic model name (e.g. 'Bluetooth Device') rather than the laptop hostname.",
        risk: "",
        wiki: "bluetooth",
    },
    // `bluetooth.pinned_alias` is intentionally absent: the field is
    // Option<String> and is omitted from the serialized default schema
    // when `None`. `proteus config get` already rejects it; explain
    // matches that contract.
    // ---- captive_portal ----
    ExplainEntry {
        key: "captive_portal.detect_url",
        doc: "URL fetched by `proteus probe` to classify the link as clear / portal-required / portal-authed.",
        risk: "Pointing this at a host the operator does not control leaks a captive-portal-style probe to that host every cycle.",
        wiki: "captive-portals",
    },
    ExplainEntry {
        key: "captive_portal.enabled",
        doc: "Master switch for captive-portal detection + the fresh-MAC-per-visit policy.",
        risk: "",
        wiki: "captive-portals",
    },
    ExplainEntry {
        key: "captive_portal.expected_response",
        doc: "Body substring the probe expects from a clear link (typically 'success').",
        risk: "",
        wiki: "captive-portals",
    },
    ExplainEntry {
        key: "captive_portal.fresh_mac_per_visit",
        doc: "Roll a new MAC every time the operator hits a captive-portal SSID so the previous identity stays unlinked.",
        risk: "Combined with sticky portals that key off the L2 identity, this forces a re-auth every visit.",
        wiki: "captive-portals",
    },
    ExplainEntry {
        key: "captive_portal.policy",
        doc: "How to react on portal-required: 'detect' (just classify), 'rotate' (force a fresh MAC), or 'block'.",
        risk: "'block' refuses to bring the link up until the operator manually clears the portal — useful on hostile lounges, fatal on a phone tether.",
        wiki: "captive-portals",
    },
    ExplainEntry {
        key: "captive_portal.timeout_secs",
        doc: "How long the probe waits for the detect URL before giving up. Default 5s.",
        risk: "",
        wiki: "captive-portals",
    },
    // ---- dhcp ----
    ExplainEntry {
        key: "dhcp.enabled",
        doc: "Master switch for the DHCP-side fingerprint suppression (hostname, vendor class, client-id rotation).",
        risk: "",
        wiki: "dhcp",
    },
    ExplainEntry {
        key: "dhcp.keep_iaid_stable_across_rotation",
        doc: "DHCPv6 IAID stability across MAC rotation: false rotates the IAID with the MAC; true pins to NM's 'stable' derivation so sticky-pool servers can re-issue the same lease.",
        risk: "Stable IAID is a small unlinkability cost; broken stable-DHCPv6 leases are the cost of the default 'rotate everything' shape.",
        wiki: "dhcp",
    },
    ExplainEntry {
        key: "dhcp.renew_on_apply",
        doc: "Run `dhcp renew` automatically after apply so the upstream DHCP server hands out a fresh lease against the new identity.",
        risk: "",
        wiki: "dhcp",
    },
    ExplainEntry {
        key: "dhcp.rotate_client_id",
        doc: "Force a new DHCP client-id alongside MAC rotation so the (MAC, client-id) tuple changes together.",
        risk: "",
        wiki: "dhcp",
    },
    ExplainEntry {
        key: "dhcp.suppress_hostname",
        doc: "Strip the local hostname from DHCP option 12 (the most common identity leak after the MAC).",
        risk: "",
        wiki: "dhcp",
    },
    ExplainEntry {
        key: "dhcp.suppress_vendor_class",
        doc: "Strip DHCP option 60 (vendor-class-identifier) which otherwise advertises the OS family in the clear.",
        risk: "",
        wiki: "dhcp",
    },
    // ---- discovery ----
    ExplainEntry {
        key: "discovery.llmnr_silence",
        doc: "Stop the host from answering Link-Local Multicast Name Resolution queries (Windows-flavoured passive discovery).",
        risk: "",
        wiki: "discovery",
    },
    ExplainEntry {
        key: "discovery.mdns_silence",
        doc: "Stop the host from answering multicast DNS queries (Avahi/Bonjour). Outbound queries from the operator are unaffected.",
        risk: "AirPrint discovery, Chromecast handshakes, and other mDNS-based UX disappear while this is on.",
        wiki: "discovery",
    },
    ExplainEntry {
        key: "discovery.ssdp_block",
        doc: "Drop SSDP (Simple Service Discovery Protocol) traffic — the UPnP-flavoured chatter consoles and TVs emit.",
        risk: "",
        wiki: "discovery",
    },
    ExplainEntry {
        key: "discovery.wsd_block",
        doc: "Drop WS-Discovery (Windows-flavoured printer/scanner discovery).",
        risk: "",
        wiki: "discovery",
    },
    // ---- dns ----
    ExplainEntry {
        key: "dns.strip_edns_client_subnet",
        doc: "Suppress EDNS Client Subnet on outbound DNS so recursive resolvers cannot use your subnet to geolocate the query.",
        risk: "CDN-geolocated traffic (Netflix, Cloudflare) may route to a slightly less-optimal POP.",
        wiki: "dns",
    },
    // ---- enterprise_wifi ----
    ExplainEntry {
        key: "enterprise_wifi.anonymous_outer_identity",
        doc: "Use an anonymous outer identity for 802.1X (eduroam, corporate) so the bare username does not appear in plaintext.",
        risk: "Some auth servers reject mismatched outer/inner identities; test on a sandbox SSID before relying on this in the field.",
        wiki: "enterprise-wifi",
    },
    ExplainEntry {
        key: "enterprise_wifi.anonymous_realm",
        doc: "Literal realm written into the anonymous outer identity when realm_strip_strategy = 'manual'.",
        risk: "",
        wiki: "enterprise-wifi",
    },
    ExplainEntry {
        key: "enterprise_wifi.realm_strip_strategy",
        doc: "How to pick the outer-identity realm: 'auto' (extract from inner identity) or 'manual' (use anonymous_realm verbatim).",
        risk: "",
        wiki: "enterprise-wifi",
    },
    // ---- events ----
    ExplainEntry {
        key: "events.enabled",
        doc: "Master switch for the long-lived event daemon (`proteus events run`) that triggers rotations on link / portal / regdom transitions.",
        risk: "",
        wiki: "rotation",
    },
    ExplainEntry {
        key: "events.link_flap_window_secs",
        doc: "Window inside which two down->up carrier transitions count as a 'flap' and pin a real roam event.",
        risk: "",
        wiki: "rotation",
    },
    ExplainEntry {
        key: "events.portal_poll_secs",
        doc: "Captive-portal sampler poll cadence in seconds. Below 5s wastes battery; above 300s misses portal-auth windows.",
        risk: "",
        wiki: "rotation",
    },
    // ---- hostname ----
    ExplainEntry {
        key: "hostname.enabled",
        doc: "Master switch for the hostname shaping path (rotation, pinning, transient/static management).",
        risk: "",
        wiki: "hostname-recipes",
    },
    ExplainEntry {
        key: "hostname.mode",
        doc: "How to pick the next hostname: 'random' (from a wordlist), 'persona' (active persona's template), or 'pinned'.",
        risk: "",
        wiki: "hostname-recipes",
    },
    // `hostname.pinned_value` is intentionally absent — Option<String>
    // omitted from the serialized schema when `None`. Use
    // `proteus hostname pin <name>` instead.
    ExplainEntry {
        key: "hostname.rotate_with_mac",
        doc: "Rotate the hostname every time the MAC rotates, so the (MAC, hostname) tuple stays correlated only with itself.",
        risk: "Apps and SSH known-hosts files keyed off the hostname will see frequent churn.",
        wiki: "hostname-recipes",
    },
    // ---- ipv6 ----
    ExplainEntry {
        key: "ipv6.addr_gen_mode",
        doc: "Per-iface IPv6 address-generation mode (e.g. 'stable-privacy', 'random', 'eui64'). 'stable-privacy' is the RFC 7217 default.",
        risk: "'eui64' leaks the MAC into the IPv6 address; never set this on a randomized interface.",
        wiki: "ipv6",
    },
    ExplainEntry {
        key: "ipv6.enabled",
        doc: "Master switch for the IPv6 shaping path (privacy extensions, addr-gen mode, NDP hardening).",
        risk: "",
        wiki: "ipv6",
    },
    ExplainEntry {
        key: "ipv6.ndp_hardening",
        doc: "Tighten Neighbor Discovery sysctls so the host stops emitting redundant solicitations that fingerprint OS family.",
        risk: "",
        wiki: "ipv6",
    },
    ExplainEntry {
        key: "ipv6.use_temp_addresses",
        doc: "Enable RFC 4941 temporary IPv6 addresses so outbound flows use a short-lived suffix.",
        risk: "",
        wiki: "ipv6",
    },
    // ---- mac ----
    ExplainEntry {
        key: "mac.enabled",
        doc: "Master switch for MAC address rotation. The single highest-value knob in the system.",
        risk: "",
        wiki: "mac-recipes",
    },
    ExplainEntry {
        key: "mac.oui_pool",
        doc: "OUI prefixes the random-MAC generator draws from. Empty = full random byte; populated = constrained to vendor-ish prefixes.",
        risk: "An obviously-randomized OUI (e.g. all-zeros) is itself a fingerprint on some captive portals.",
        wiki: "mac-recipes",
    },
    ExplainEntry {
        key: "mac.rotation_interval",
        doc: "How often the MAC rotation timer should fire. Compact durations like '5m', '2h', or systemd-style cadences.",
        risk: "Rotating too aggressively can break long-lived TCP sessions and trigger captive-portal re-auth on every cycle.",
        wiki: "rotation",
    },
    // ---- nft ----
    ExplainEntry {
        key: "nft.broadcast_ping_drop",
        doc: "Drop ICMPv4 echo-request to broadcast addresses (smurf-style probes).",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "nft.icmpv4_timestamp_drop",
        doc: "Drop ICMPv4 timestamp-request packets. Timestamps are an underused fingerprint vector (clock skew + uptime).",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "nft.igmp_query_drop",
        doc: "Suppress IGMP membership-query replies on input so the host stops answering multicast-group probes.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    // ---- ntp ----
    ExplainEntry {
        key: "ntp.enabled",
        doc: "Master switch for the timesyncd drop-in that pins the NTP pool. Skipped if chronyd or ntpd is installed.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "ntp.fallback_servers",
        doc: "Fallback NTP servers used when the primary pool is unreachable.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "ntp.ntp_servers",
        doc: "Primary NTP pool. Privacy-preserving by default; persona-aware variants are a Milestone 4a follow-up.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    // `persona.active` is intentionally absent — Option<String> omitted
    // from the serialized schema when `None`. Use
    // `proteus persona use <id>` / `proteus persona clear` instead.
    // ---- probes ----
    ExplainEntry {
        key: "probes.cooldown",
        doc: "Cooldown between probe rounds so a failing AP does not spam the operator's terminal.",
        risk: "",
        wiki: "probes",
    },
    ExplainEntry {
        key: "probes.endpoints",
        doc: "Probe endpoints used to classify a link as clear / portal-required / inconclusive. Must include >= quorum_n entries.",
        risk: "Pointing probes at a host the operator does not control leaks a per-cycle probe.",
        wiki: "probes",
    },
    ExplainEntry {
        key: "probes.interval",
        doc: "Per-probe timeout. Compact duration string ('1s', '500ms').",
        risk: "",
        wiki: "probes",
    },
    ExplainEntry {
        key: "probes.quorum_n",
        doc: "How many probes must agree for a 'Clear' verdict. Must be > 0 and <= probes.endpoints.len().",
        risk: "Setting quorum_n above endpoints.len() makes `proteus probe` return Inconclusive forever.",
        wiki: "probes",
    },
    ExplainEntry {
        key: "probes.quorum_total",
        doc: "Total probes fired per round. Must be >= quorum_n.",
        risk: "",
        wiki: "probes",
    },
    // `profile` is a top-level scalar (no `section.` prefix), and the
    // existing `proteus config get` / `set` already reject it as
    // unrecognised. Operators use `proteus config set-profile <name>`
    // to switch profiles; `explain` mirrors that contract.
    // ---- resolved ---- (alphabetically `re` < `rf`, so this block must
    // precede the `rf.*` block.)
    ExplainEntry {
        key: "resolved.llmnr_off",
        doc: "Disable LLMNR responder + resolver in systemd-resolved. Hard-guards on a foreign drop-in.",
        risk: "",
        wiki: "dns",
    },
    ExplainEntry {
        key: "resolved.mdns_off",
        doc: "Disable resolved's mDNS responder + resolver. The Avahi path (if installed) is unaffected.",
        risk: "",
        wiki: "dns",
    },
    // ---- rf ----
    ExplainEntry {
        key: "rf.scan_random_mac",
        doc: "Randomize the scan-time source MAC at the supplicant layer so passive captures do not see the real MAC during AP scans.",
        risk: "",
        wiki: "rf-fingerprinting",
    },
    ExplainEntry {
        key: "rf.tx_power_reduce",
        doc: "Master switch for TX-power reduction. Off in Min/Low/Med profiles, on in High and Agr.",
        risk: "Reduces effective range from the AP; in marginal-signal environments this can drop the link entirely.",
        wiki: "rf-fingerprinting",
    },
    ExplainEntry {
        key: "rf.tx_power_reduction_db",
        doc: "dB below the regulatory maximum to clamp TX power at. Default 6dB (~quarter the radiated power). Hardware-clamped on actual write.",
        risk: "Excessive reduction (>10dB) makes the link unusable on most APs.",
        wiki: "rf-fingerprinting",
    },
    // ---- stack ----
    ExplainEntry {
        key: "stack.icmp_info_replies_drop",
        doc: "Suppress ICMP info-reply emissions so the host stops answering ICMP echo/timestamp/address-mask probes.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "stack.icmpv6_hardening",
        doc: "Tighten ICMPv6 sysctls (rate limits, redirect handling) so the host's stack stops leaking ICMPv6-shaped fingerprints.",
        risk: "",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "stack.suppress_gratuitous_arp",
        doc: "Stop emitting gratuitous ARP on link-up so the host does not announce its (new) MAC to the broadcast domain.",
        risk: "Some failover protocols (carp, vrrp) rely on gratuitous ARP — turn this off on routers/HA pairs.",
        wiki: "stack-fingerprint",
    },
    ExplainEntry {
        key: "stack.tcp_timestamps_off",
        doc: "Disable RFC 1323 TCP timestamps. Timestamps leak uptime + reveal a stable host across NAT.",
        risk: "Some middleboxes (older load balancers) misbehave without TCP timestamps.",
        wiki: "stack-fingerprint",
    },
    // ---- timers ----
    ExplainEntry {
        key: "timers.check.interval",
        doc: "Cadence for the `proteus-check.timer` periodic-verifier unit. Compact durations or systemd OnCalendar expressions.",
        risk: "",
        wiki: "timer",
    },
    ExplainEntry {
        key: "timers.rotate.interval",
        doc: "Cadence for the `proteus-rotate.timer` unit. Compact durations or systemd OnCalendar expressions. The sentinel 'never' disables the timer.",
        risk: "Pairs with [mac].rotation_interval — when both are set the timer cadence wins.",
        wiki: "timer",
    },
];

/// Find the catalogue entry for a dotted key. Linear scan — the catalogue
/// is ~60 entries and the explain command runs once per invocation, so a
/// BTreeMap would be overkill.
pub fn lookup(key: &str) -> Option<&'static ExplainEntry> {
    ENTRIES.iter().find(|e| e.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every catalogue entry must point at a real key in the live schema.
    /// Failing this test is the drift signal: rename a struct field in
    /// `config.rs`, forget to update the catalogue, and CI bails here.
    #[test]
    fn entries_map_to_real_keys() {
        use crate::config::Config;
        use toml_edit::DocumentMut;

        let s = toml::to_string_pretty(&Config::default()).expect("default config serializes");
        let doc: DocumentMut = s.parse().expect("default config re-parses as toml_edit");

        let mut live_keys: Vec<String> = Vec::new();
        // Walk the live schema and collect every dotted leaf key plus the
        // top-level scalar `profile`. Mirrors the walk in
        // `commands::config_cmd::enumerate_keys` so the two stay in sync.
        for (section, item) in doc.iter() {
            if let Some(v) = item.as_value() {
                // Top-level scalars (just `profile` today).
                let _ = v;
                live_keys.push(section.to_string());
            } else if let Some(table) = item.as_table() {
                collect_leaves(&[section], table, &mut live_keys);
            }
        }

        for entry in ENTRIES {
            assert!(
                live_keys.iter().any(|k| k == entry.key),
                "catalogue key '{}' is not in the live schema; remove it or follow the rename",
                entry.key,
            );
        }
    }

    /// Entries must be alphabetically sorted by key so diffs stay readable.
    #[test]
    fn entries_are_sorted_by_key() {
        for pair in ENTRIES.windows(2) {
            assert!(
                pair[0].key < pair[1].key,
                "catalogue entries out of order: '{}' should come before '{}'",
                pair[1].key,
                pair[0].key,
            );
        }
    }

    /// Spec target: cover at least the top 30 keys across the major
    /// sections. Pin a floor so a future refactor cannot quietly shrink
    /// coverage.
    #[test]
    fn entries_cover_at_least_thirty_keys() {
        assert!(
            ENTRIES.len() >= 30,
            "catalogue must cover >= 30 keys (current: {})",
            ENTRIES.len(),
        );
    }

    /// `lookup` returns Some for known keys and None for unknown ones.
    #[test]
    fn lookup_finds_known_and_misses_unknown() {
        assert!(lookup("mac.rotation_interval").is_some());
        assert!(lookup("dhcp.enabled").is_some());
        assert!(lookup("nonexistent.key").is_none());
    }

    fn collect_leaves(path: &[&str], table: &toml_edit::Table, out: &mut Vec<String>) {
        for (field, sub) in table.iter() {
            if sub.as_value().is_some() {
                let key = if path.is_empty() {
                    field.to_string()
                } else {
                    format!("{}.{field}", path.join("."))
                };
                out.push(key);
            } else if let Some(t) = sub.as_table() {
                let mut nested: Vec<&str> = path.to_vec();
                nested.push(field);
                collect_leaves(&nested, t, out);
            }
        }
    }
}
