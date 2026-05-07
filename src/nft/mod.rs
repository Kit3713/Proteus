// SPDX-License-Identifier: GPL-3.0-or-later

//! nftables rule manager for Proteus.
//!
//! Owns a single dedicated table `inet proteus` with separate chains for the
//! different fingerprint-reduction concerns Proteus addresses:
//!
//! - `icmp_drops` — always installed when nft is managed. Drops the pre-DHCP
//!   ICMP discovery vectors (RFC 792 timestamp/info/address-mask requests) plus
//!   a small ICMPv6 trim. See `proteus wiki stack-fingerprint`.
//! - `discovery_drops` — installed only when `[discovery] ssdp_block` and/or
//!   `wsd_block` are on. SSDP and WSD are off by default because they break
//!   KDE Connect and WS-Discovery printers respectively. See
//!   `proteus wiki discovery`.
//!
//! This module is pure rendering plus the `nft` invocation. Subcommand glue
//! lives in `crate::commands::nft`. Keeping the table dedicated means revert
//! is a single `delete table inet proteus` — no surgical removal of rules
//! from a shared table.
//!
//! Rule application is idempotent: we always emit a `delete table inet
//! proteus` (suppressing the not-found error) before adding the freshly
//! rendered ruleset. Running apply ten times converges to the same state as
//! running it once.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};

use crate::config::{DiscoveryConfig, NftConfig};
use crate::version;

/// Canonical table name. Single source of truth.
pub const TABLE_NAME: &str = "proteus";
/// Canonical address family.
pub const TABLE_FAMILY: &str = "inet";

/// Render the full ruleset for the given discovery + nft config.
///
/// Always includes the `icmp_drops` chain. Adds `discovery_drops` when at
/// least one of `ssdp_block` / `wsd_block` is on, and `extra_drops` when
/// any of the new opt-in `[nft]` knobs (timestamp/broadcast-ping/IGMP) are
/// set. Each chain has a distinct `(hook, priority)` so eval order between
/// them is deterministic — see issue #148.
pub fn render_ruleset(discovery: &DiscoveryConfig, nft: &NftConfig) -> String {
    render_ruleset_with_persona(discovery, nft, None)
}

/// Roadmap Milestone 4a: persona-aware nftables variants. Emits the
/// same baseline ruleset as [`render_ruleset`] plus an optional
/// `persona_drops` chain shaped by the active persona — for instance,
/// stealth covers that don't advertise mDNS get an inbound-5353 drop.
pub fn render_ruleset_with_persona(
    discovery: &DiscoveryConfig,
    nft: &NftConfig,
    persona: Option<&crate::persona::Persona>,
) -> String {
    let mut out = String::new();
    out.push_str(&render_header());
    out.push_str(&format!(
        "table {family} {table} {{\n",
        family = TABLE_FAMILY,
        table = TABLE_NAME
    ));
    out.push_str(&render_icmp_chain());
    if discovery.ssdp_block || discovery.wsd_block {
        out.push_str(&render_discovery_chain(discovery));
    }
    if extra_chain_active(nft) {
        out.push_str(&render_extra_chain(nft));
    }
    if let Some(p) = persona
        && persona_chain_active(p)
    {
        out.push_str(&render_persona_chain(p));
    }
    out.push_str("}\n");
    out
}

/// True iff the active persona supplies any nft-shaping rule. Today's
/// rule set is small (mDNS posture); growing this stays additive.
pub fn persona_chain_active(p: &crate::persona::Persona) -> bool {
    !p.mdns_advertise
}

/// True iff at least one of the opt-in `[nft]` knobs is on. Keeps the
/// "should we emit the chain at all" decision in one place.
pub fn extra_chain_active(nft: &NftConfig) -> bool {
    nft.icmpv4_timestamp_drop || nft.broadcast_ping_drop || nft.igmp_query_drop
}

fn render_header() -> String {
    format!(
        "# managed by proteus v{version}\n\
         # do not edit; manage via `proteus nft apply` / `proteus nft revert`\n",
        version = version::VERSION
    )
}

/// Priority for the `icmp_drops` chain. -100 sits above conntrack
/// (NF_IP_PRI_CONNTRACK = -200) and below raw (NF_IP_PRI_RAW = -300) — the
/// same slot firewalld uses for its pre-routing chains.
pub(crate) const ICMP_CHAIN_PRIORITY: i32 = -100;
/// Priority for the `discovery_drops` chain. We deliberately offset it
/// from `icmp_drops` (issue #148) so two chains never share the same
/// `(hook, priority)` slot — sharing is technically legal in nftables but
/// the eval order between equal-priority chains is undefined, which
/// matters when one chain might accept a packet another would drop.
pub(crate) const DISCOVERY_CHAIN_PRIORITY: i32 = -99;
/// Priority for the new `extra_drops` chain (Milestone 4a). Same rationale
/// as above: pick a distinct slot so eval order between the three chains is
/// always deterministic.
pub(crate) const EXTRA_CHAIN_PRIORITY: i32 = -98;
/// Priority for the persona-aware `persona_drops` chain (Milestone 4a
/// follow-up). Distinct from the three chains above so the eval-order
/// invariant from issue #148 stays explicit.
pub(crate) const PERSONA_CHAIN_PRIORITY: i32 = -97;

fn render_icmp_chain() -> String {
    // policy accept means we don't disturb existing input rulesets — we
    // only drop the specific ICMP types.
    let mut out = String::new();
    out.push_str("    chain icmp_drops {\n");
    out.push_str(&format!(
        "        type filter hook input priority {ICMP_CHAIN_PRIORITY}; policy accept;\n"
    ));
    out.push_str("        # ICMP info-request, timestamp-request, address-mask-request (RFC 792 fingerprint vectors)\n");
    out.push_str(
        "        icmp type { timestamp-request, info-request, address-mask-request } drop\n",
    );
    out.push_str(
        "        # ICMPv6 small trim — node-info-query is rarely used in modern userspace\n",
    );
    out.push_str("        icmpv6 type { nd-redirect, mld-listener-query } drop\n");
    out.push_str("    }\n");
    out
}

fn render_extra_chain(nft: &NftConfig) -> String {
    let mut out = String::new();
    out.push_str("    chain extra_drops {\n");
    out.push_str(&format!(
        "        type filter hook input priority {EXTRA_CHAIN_PRIORITY}; policy accept;\n"
    ));
    if nft.icmpv4_timestamp_drop {
        out.push_str(
            "        # ICMPv4 timestamp-request — narrow fingerprint vector kept opt-in\n",
        );
        out.push_str("        icmp type timestamp-request drop\n");
    }
    if nft.broadcast_ping_drop {
        // 255.255.255.255 covers the limited broadcast; subnet-broadcast
        // probes resolve via routing so the simplest portable rule is to
        // drop ICMPv4 echo-request to the limited-broadcast destination.
        out.push_str(
            "        # ICMPv4 echo-request to the limited broadcast (smurf-style probes)\n",
        );
        out.push_str("        ip daddr 255.255.255.255 icmp type echo-request drop\n");
    }
    if nft.igmp_query_drop {
        // IGMP membership queries advertise the host as a multicast
        // listener and leak per-router-pair fingerprints. Default off
        // because dropping queries breaks multicast-aware applications
        // (mDNS already handled separately).
        out.push_str("        # IGMP membership-query suppression — leaks listener state\n");
        out.push_str("        ip protocol igmp drop\n");
    }
    out.push_str("    }\n");
    out
}

/// Render persona-aware drops. The cover identity should accept or
/// reject discovery traffic the way the modelled device does. Today's
/// shape: `mdns_advertise = false` personas drop UDP 5353 inbound
/// (stealth phones / laptops that don't expose Bonjour). TVs, printers,
/// and randomizers leave it open.
fn render_persona_chain(p: &crate::persona::Persona) -> String {
    let mut out = String::new();
    out.push_str("    chain persona_drops {\n");
    out.push_str(&format!(
        "        type filter hook input priority {PERSONA_CHAIN_PRIORITY}; policy accept;\n"
    ));
    if !p.mdns_advertise {
        out.push_str(
            "        # persona does not advertise mDNS; drop inbound 5353\n",
        );
        out.push_str("        udp dport 5353 drop\n");
    }
    out.push_str("    }\n");
    out
}

fn render_discovery_chain(discovery: &DiscoveryConfig) -> String {
    let mut out = String::new();
    out.push_str("    chain discovery_drops {\n");
    out.push_str(&format!(
        "        type filter hook input priority {DISCOVERY_CHAIN_PRIORITY}; policy accept;\n"
    ));
    if discovery.ssdp_block {
        out.push_str("        # SSDP (UPnP) — breaks KDE Connect when blocked; opt-in\n");
        out.push_str("        udp dport 1900 drop\n");
    }
    if discovery.wsd_block {
        out.push_str(
            "        # WSD (Web Services for Devices) — breaks WSD-only printers; opt-in\n",
        );
        out.push_str("        udp dport 3702 drop\n");
        out.push_str("        tcp dport 5357 drop\n");
    }
    out.push_str("    }\n");
    out
}

/// Render the `add table; delete table` prelude that makes ruleset application
/// idempotent without erroring on a fresh system.
///
/// `add table` is a no-op in nft when the table already exists; the following
/// `delete table` then succeeds whether the table was installed or not. This
/// avoids needing the `destroy` keyword, which only exists on nftables 1.0.5+.
pub fn render_delete_script() -> String {
    format!(
        "add table {family} {table}\ndelete table {family} {table}\n",
        family = TABLE_FAMILY,
        table = TABLE_NAME
    )
}

/// Detect whether the `nft` binary is on `$PATH`.
///
/// We don't actually invoke it here — `nft list tables` requires root for
/// full output, and `which`-style binary presence is the right gate for
/// status reads. Mutating commands surface errors from `nft -f -` directly.
pub fn nft_present() -> bool {
    Command::new("nft")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Outcome of probing for our table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableProbe {
    /// Table is installed; full ruleset returned in the inner string.
    Present(String),
    /// Table is absent. nft confirmed via "No such file or directory".
    Absent,
    /// nft refused to answer (typically `Operation not permitted` for non-root).
    /// Status callers can keep going; mutating callers should already be root.
    PermissionDenied,
}

/// Run `nft list table inet proteus` and classify the outcome.
///
/// Returns `Err(_)` only if `nft` itself fails for an unexpected reason.
pub fn list_our_table() -> Result<TableProbe> {
    let output = Command::new("nft")
        .args(["list", "table", TABLE_FAMILY, TABLE_NAME])
        .output()
        .context("invoking nft list")?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).into_owned();
        if s.trim().is_empty() {
            return Ok(TableProbe::Absent);
        }
        return Ok(TableProbe::Present(s));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The "No such file or directory" message is what nft prints when the
    // table is absent. Not an error condition for us.
    if stderr.contains("No such file") || stderr.contains("does not exist") {
        return Ok(TableProbe::Absent);
    }
    // Non-root callers (status invoked as a normal user) hit this. We don't
    // want to surface a stack trace for the common case.
    if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        return Ok(TableProbe::PermissionDenied);
    }
    Err(anyhow!(
        "nft list table {} {} exited {}: {}",
        TABLE_FAMILY,
        TABLE_NAME,
        output.status,
        stderr.trim()
    ))
}

/// Apply the rendered ruleset via a single `nft -f -` invocation.
///
/// Idempotent in one transaction: `add table` is a no-op when the table
/// already exists, the following `delete table` then clears prior chains,
/// and the final ruleset re-installs them.
pub fn apply_ruleset(discovery: &DiscoveryConfig, nft: &NftConfig) -> Result<()> {
    apply_ruleset_with_persona(discovery, nft, None)
}

/// Roadmap Milestone 4a follow-up: apply the persona-aware variant.
/// `apply_ruleset` is now a thin wrapper over this with `persona = None`.
pub fn apply_ruleset_with_persona(
    discovery: &DiscoveryConfig,
    nft: &NftConfig,
    persona: Option<&crate::persona::Persona>,
) -> Result<()> {
    let mut script = render_delete_script();
    script.push_str(&render_ruleset_with_persona(discovery, nft, persona));
    run_nft_script(&script).context("applying proteus nft ruleset")
}

/// Revert by deleting our table. No-op if the table is absent.
pub fn revert_ruleset() -> Result<()> {
    if matches!(list_our_table()?, TableProbe::Absent) {
        return Ok(());
    }
    let script = format!(
        "delete table {family} {table}\n",
        family = TABLE_FAMILY,
        table = TABLE_NAME
    );
    run_nft_script(&script).context("removing proteus nft table")
}

fn run_nft_script(script: &str) -> Result<()> {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning nft -f -")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("could not open nft stdin"))?;
        stdin
            .write_all(script.as_bytes())
            .context("writing ruleset to nft stdin")?;
    }
    let output = child.wait_with_output().context("waiting on nft")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "nft -f - exited {}: {}",
        output.status,
        stderr.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(ssdp: bool, wsd: bool) -> DiscoveryConfig {
        DiscoveryConfig {
            ssdp_block: ssdp,
            wsd_block: wsd,
            ..DiscoveryConfig::default()
        }
    }

    fn nft_cfg(timestamp: bool, broadcast: bool, igmp: bool) -> NftConfig {
        NftConfig {
            icmpv4_timestamp_drop: timestamp,
            broadcast_ping_drop: broadcast,
            igmp_query_drop: igmp,
        }
    }

    #[test]
    fn ruleset_always_includes_table_and_icmp_chain() {
        let body = render_ruleset(&cfg(false, false), &NftConfig::default());
        assert!(body.contains("table inet proteus"), "missing table: {body}");
        assert!(
            body.contains("chain icmp_drops"),
            "missing icmp chain: {body}"
        );
        assert!(body.contains("type filter hook input priority -100"));
        assert!(body.contains("timestamp-request"));
        assert!(body.contains("info-request"));
        assert!(body.contains("address-mask-request"));
    }

    #[test]
    fn ruleset_includes_managed_header() {
        let body = render_ruleset(&cfg(false, false), &NftConfig::default());
        assert!(
            body.contains("# managed by proteus"),
            "missing managed-file header: {body}"
        );
    }

    #[test]
    fn ruleset_omits_discovery_chain_by_default() {
        let body = render_ruleset(&cfg(false, false), &NftConfig::default());
        assert!(
            !body.contains("chain discovery_drops"),
            "discovery chain should not appear when neither block is set: {body}"
        );
        assert!(!body.contains("udp dport 1900"));
        assert!(!body.contains("udp dport 3702"));
        assert!(!body.contains("tcp dport 5357"));
    }

    #[test]
    fn ssdp_block_adds_only_ssdp_rules() {
        let body = render_ruleset(&cfg(true, false), &NftConfig::default());
        assert!(
            body.contains("chain discovery_drops"),
            "missing chain: {body}"
        );
        assert!(body.contains("udp dport 1900 drop"));
        assert!(
            !body.contains("udp dport 3702"),
            "WSD UDP rule must not appear when only ssdp_block is set"
        );
        assert!(
            !body.contains("tcp dport 5357"),
            "WSD TCP rule must not appear when only ssdp_block is set"
        );
    }

    #[test]
    fn wsd_block_adds_only_wsd_rules() {
        let body = render_ruleset(&cfg(false, true), &NftConfig::default());
        assert!(body.contains("chain discovery_drops"));
        assert!(body.contains("udp dport 3702 drop"));
        assert!(body.contains("tcp dport 5357 drop"));
        assert!(
            !body.contains("udp dport 1900"),
            "SSDP rule must not appear when only wsd_block is set"
        );
    }

    #[test]
    fn both_blocks_render_both() {
        let body = render_ruleset(&cfg(true, true), &NftConfig::default());
        assert!(body.contains("udp dport 1900 drop"));
        assert!(body.contains("udp dport 3702 drop"));
        assert!(body.contains("tcp dport 5357 drop"));
    }

    #[test]
    fn extra_chain_absent_when_all_knobs_off() {
        let body = render_ruleset(&cfg(false, false), &nft_cfg(false, false, false));
        assert!(
            !body.contains("chain extra_drops"),
            "extra_drops chain must not appear when every nft knob is off: {body}"
        );
        assert!(!body.contains("icmp type timestamp-request drop"));
    }

    #[test]
    fn icmpv4_timestamp_drop_emits_only_timestamp_rule() {
        let body = render_ruleset(&cfg(false, false), &nft_cfg(true, false, false));
        assert!(body.contains("chain extra_drops"));
        assert!(body.contains("icmp type timestamp-request drop"));
        assert!(!body.contains("ip daddr 255.255.255.255"));
        assert!(!body.contains("ip protocol igmp drop"));
    }

    #[test]
    fn broadcast_ping_drop_emits_only_broadcast_rule() {
        let body = render_ruleset(&cfg(false, false), &nft_cfg(false, true, false));
        assert!(body.contains("chain extra_drops"));
        assert!(body.contains("ip daddr 255.255.255.255 icmp type echo-request drop"));
        assert!(!body.contains("icmp type timestamp-request drop"));
        assert!(!body.contains("ip protocol igmp drop"));
    }

    #[test]
    fn igmp_query_drop_emits_only_igmp_rule() {
        let body = render_ruleset(&cfg(false, false), &nft_cfg(false, false, true));
        assert!(body.contains("chain extra_drops"));
        assert!(body.contains("ip protocol igmp drop"));
        assert!(!body.contains("icmp type timestamp-request drop"));
        assert!(!body.contains("ip daddr 255.255.255.255"));
    }

    #[test]
    fn all_extra_knobs_render_all_rules() {
        let body = render_ruleset(&cfg(false, false), &nft_cfg(true, true, true));
        assert!(body.contains("chain extra_drops"));
        assert!(body.contains("icmp type timestamp-request drop"));
        assert!(body.contains("ip daddr 255.255.255.255 icmp type echo-request drop"));
        assert!(body.contains("ip protocol igmp drop"));
    }

    #[test]
    fn extra_chain_uses_distinct_priority() {
        // Three chains, three distinct priorities. Pin the invariant so a
        // future refactor can't recreate the issue-#148 ambiguity.
        assert_ne!(EXTRA_CHAIN_PRIORITY, ICMP_CHAIN_PRIORITY);
        assert_ne!(EXTRA_CHAIN_PRIORITY, DISCOVERY_CHAIN_PRIORITY);
        let body = render_ruleset(&cfg(true, true), &nft_cfg(true, true, true));
        let extra_marker =
            format!("type filter hook input priority {EXTRA_CHAIN_PRIORITY}; policy accept;");
        assert!(
            body.contains(&extra_marker),
            "missing extra priority {EXTRA_CHAIN_PRIORITY}: {body}"
        );
    }

    #[test]
    fn extra_chain_active_helper_matches_render_decision() {
        assert!(!extra_chain_active(&nft_cfg(false, false, false)));
        assert!(extra_chain_active(&nft_cfg(true, false, false)));
        assert!(extra_chain_active(&nft_cfg(false, true, false)));
        assert!(extra_chain_active(&nft_cfg(false, false, true)));
    }

    #[test]
    fn delete_script_is_idempotent_form() {
        let s = render_delete_script();
        assert!(
            s.contains("add table inet proteus"),
            "missing add prelude: {s}"
        );
        assert!(
            s.contains("delete table inet proteus"),
            "missing delete: {s}"
        );
        // add must come before delete so a not-found delete on a fresh system
        // doesn't abort the script.
        let add_pos = s.find("add table").unwrap();
        let del_pos = s.find("delete table").unwrap();
        assert!(add_pos < del_pos, "add must precede delete: {s}");
    }

    #[test]
    fn icmpv6_trim_present() {
        let body = render_ruleset(&cfg(false, false), &NftConfig::default());
        assert!(body.contains("icmpv6 type"), "missing icmpv6 trim: {body}");
    }

    fn persona_with_mdns(advertise: bool) -> crate::persona::Persona {
        crate::persona::Persona {
            id: "test-persona".into(),
            display_name: "Test".into(),
            kind: crate::persona::PersonaKind::Stealth,
            category: crate::persona::PersonaCategory::Phone,
            oui_pool: vec![],
            mac_byte_pattern: None,
            hostname_template: "{owner}".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: advertise,
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        }
    }

    /// Roadmap Milestone 4a follow-up: persona that does not advertise
    /// mDNS contributes a `persona_drops` chain with an inbound-5353
    /// drop. Personas that *do* advertise (TVs, printers, randomizers)
    /// don't add the chain at all.
    #[test]
    fn persona_chain_appears_only_when_persona_drops_mdns() {
        let p_quiet = persona_with_mdns(false);
        let p_loud = persona_with_mdns(true);
        let body_quiet =
            render_ruleset_with_persona(&cfg(false, false), &NftConfig::default(), Some(&p_quiet));
        assert!(body_quiet.contains("chain persona_drops"), "{body_quiet}");
        assert!(body_quiet.contains("udp dport 5353 drop"));

        let body_loud =
            render_ruleset_with_persona(&cfg(false, false), &NftConfig::default(), Some(&p_loud));
        assert!(!body_loud.contains("chain persona_drops"), "{body_loud}");

        let body_no_persona =
            render_ruleset_with_persona(&cfg(false, false), &NftConfig::default(), None);
        assert!(!body_no_persona.contains("chain persona_drops"));
    }

    #[test]
    fn persona_chain_uses_distinct_priority_from_others() {
        assert_ne!(PERSONA_CHAIN_PRIORITY, ICMP_CHAIN_PRIORITY);
        assert_ne!(PERSONA_CHAIN_PRIORITY, DISCOVERY_CHAIN_PRIORITY);
        assert_ne!(PERSONA_CHAIN_PRIORITY, EXTRA_CHAIN_PRIORITY);
        let p = persona_with_mdns(false);
        let body =
            render_ruleset_with_persona(&cfg(false, false), &NftConfig::default(), Some(&p));
        let marker =
            format!("type filter hook input priority {PERSONA_CHAIN_PRIORITY}; policy accept;");
        assert!(body.contains(&marker), "{body}");
    }

    #[test]
    fn persona_chain_active_helper_matches_render_decision() {
        let p_quiet = persona_with_mdns(false);
        let p_loud = persona_with_mdns(true);
        assert!(persona_chain_active(&p_quiet));
        assert!(!persona_chain_active(&p_loud));
    }

    #[test]
    fn render_ruleset_back_compat_is_persona_none() {
        // The legacy entry point and the persona-aware one with `None`
        // must produce identical output. Pin the back-compat invariant.
        let body_a = render_ruleset(&cfg(true, true), &nft_cfg(true, true, true));
        let body_b =
            render_ruleset_with_persona(&cfg(true, true), &nft_cfg(true, true, true), None);
        assert_eq!(body_a, body_b);
    }

    #[test]
    fn chains_use_distinct_priorities() {
        // Issue #148 — both chains used to share `(input, -100)`, leaving
        // the eval order between them undefined. Confirm they're now
        // separated.
        assert_ne!(ICMP_CHAIN_PRIORITY, DISCOVERY_CHAIN_PRIORITY);

        let body = render_ruleset(&cfg(true, true), &NftConfig::default());
        let icmp_marker =
            format!("type filter hook input priority {ICMP_CHAIN_PRIORITY}; policy accept;");
        let disc_marker =
            format!("type filter hook input priority {DISCOVERY_CHAIN_PRIORITY}; policy accept;");
        assert!(
            body.contains(&icmp_marker),
            "missing icmp priority {ICMP_CHAIN_PRIORITY}: {body}"
        );
        assert!(
            body.contains(&disc_marker),
            "missing discovery priority {DISCOVERY_CHAIN_PRIORITY}: {body}"
        );
    }
}
