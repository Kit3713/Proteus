// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::mac::generator::{
    self, ByteSuffixPattern, CandidateAttempt, GenerateOptions, ProbeOptions, RejectionReason,
};
use crate::mac::oui;
use crate::mac::probe::{Probe, SystemProbe};
use crate::mac::{Mac, arp, factory};
use crate::nm::{self, DeviceInfo, DeviceKind};
use crate::persona;
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
struct RotateReport {
    rotated: Vec<RotatedEntry>,
    skipped: Vec<SkippedEntry>,
    /// Roadmap M2: per-iface explain trace, populated only when `--explain`
    /// is set. `serde(skip_serializing_if)` keeps the wire format stable
    /// for callers that don't use `--explain` (the JSON shape doesn't
    /// change for them).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    explain: Vec<ExplainEntry>,
    /// Active persona id, when one shaped this rotation. Surfaced under
    /// `--explain` so the operator sees which `oui_pool` was actually
    /// in scope. `None` means the global `[mac] oui_pool` was in use
    /// (the v0.2.x slider path).
    #[serde(skip_serializing_if = "Option::is_none")]
    active_persona: Option<String>,
}

#[derive(Debug, Serialize)]
struct RotatedEntry {
    iface: String,
    previous: Option<String>,
    new: String,
    connection: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkippedEntry {
    iface: String,
    reason: String,
}

/// Per-interface explain trace. Surfaces every candidate the generator
/// considered + the reason it was rejected, so the operator can see why
/// the final MAC was picked.
#[derive(Debug, Serialize)]
struct ExplainEntry {
    iface: String,
    chosen_token: String,
    oui_fallbacks: usize,
    candidates: Vec<ExplainCandidate>,
}

#[derive(Debug, Serialize)]
struct ExplainCandidate {
    mac: String,
    token: String,
    reason: String,
}

pub fn run(
    iface_filter: Option<&str>,
    yes: bool,
    explain: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if explain {
        tracing::debug!("rotate --explain enabled (verbose candidate trace)");
    }
    if let Err(code) = super::require_yes(
        yes,
        "'rotate' is mutating (writes new MACs to NetworkManager)",
        "proteus help rotate",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #126: serialize concurrent rotates on <state-dir>/.lock.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;
    let mut state = State::load_or_default(&state_path)?;

    // Roadmap M2: layered exclusion set —
    // 1. Live `/proc/net/arp` snapshot,
    // 2. Default-gateway MAC,
    // 3. Recent-neighbour ledger (keyed by MAC, default 5-minute window).
    // The ledger is reseeded each rotation from the kernel snapshot so it
    // stays useful even though `proteus rotate` isn't a long-lived daemon.
    let arp_macs = arp::read_arp_macs();
    let recent = arp::RecentNeighbourTable::new();
    recent.record_all(arp_macs.iter().copied());
    let gateway_mac = arp::read_default_gateway_mac();
    let mut avoid: HashSet<Mac> = arp_macs;
    if let Some(gw) = gateway_mac {
        avoid.insert(gw);
    }
    for m in recent.current_macs() {
        avoid.insert(m);
    }

    // Production probe — falls back to Unsupported when CAP_NET_RAW is
    // missing, which the generator handles transparently.
    let probe = SystemProbe::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result: Result<RotateReport> = rt.block_on(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        let devices = nm::list_devices(&conn).await?;
        rotate_devices(
            &conn,
            devices,
            iface_filter,
            &config,
            &avoid,
            &probe,
            explain,
            &mut state,
            &state_path,
        )
        .await
    });

    let report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("proteus: rotate failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    if report.rotated.is_empty() && report.skipped.is_empty() {
        eprintln!("proteus: no NetworkManager-managed interfaces matched");
        return Ok(exit::GENERIC_ERROR);
    }

    print_report(&report, explain);
    Ok(exit::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
async fn rotate_devices<P: Probe + ?Sized>(
    conn: &zbus::Connection,
    devices: Vec<DeviceInfo>,
    iface_filter: Option<&str>,
    config: &Config,
    avoid: &HashSet<Mac>,
    probe: &P,
    explain: bool,
    state: &mut State,
    state_path: &Path,
) -> Result<RotateReport> {
    let mut report = RotateReport {
        rotated: Vec::new(),
        skipped: Vec::new(),
        explain: Vec::new(),
        active_persona: persona::active_for(config, None, persona::resolve::default_user_root())
            .map(|p| p.id),
    };
    for dev in devices {
        if let Some(f) = iface_filter
            && dev.interface != f
        {
            continue;
        }
        if !matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) {
            if iface_filter.is_some() {
                report.skipped.push(SkippedEntry {
                    iface: dev.interface.clone(),
                    reason: format!("device kind {:?} not supported", dev.kind),
                });
            }
            continue;
        }
        if !dev.managed && iface_filter.is_none() {
            // Quietly skip when iterating all devices.
            continue;
        }
        if let Some(rec) = state.managed.interfaces.get(&dev.interface)
            && let Some(pin) = &rec.pinned
        {
            report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("pinned to {pin}"),
            });
            continue;
        }
        match rotate_one(conn, &dev, config, avoid, probe, state, state_path).await {
            Ok((entry, attempts, chosen_token, oui_fallbacks)) => {
                if explain {
                    report.explain.push(ExplainEntry {
                        iface: dev.interface.clone(),
                        chosen_token,
                        oui_fallbacks,
                        candidates: attempts
                            .into_iter()
                            .map(explain_candidate_from_attempt)
                            .collect(),
                    });
                }
                report.rotated.push(entry);
            }
            Err(e) => report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    Ok(report)
}

async fn rotate_one<P: Probe + ?Sized>(
    conn: &zbus::Connection,
    dev: &DeviceInfo,
    config: &Config,
    avoid: &HashSet<Mac>,
    probe: &P,
    state: &mut State,
    state_path: &Path,
) -> Result<(RotatedEntry, Vec<CandidateAttempt>, String, usize)> {
    if dev.connections.is_empty() {
        anyhow::bail!("no NM connection profile available");
    }

    // Capture-then-save-then-mutate: the original factory MAC must be
    // durable on disk BEFORE we ask NetworkManager to set a cloned MAC.
    // Otherwise a crash between the DBus write and the final state.save()
    // at the end of `run` would lose the factory MAC and turn `revert` into
    // a no-op (sacred-originals invariant; issue #119).
    capture_original_mac(state, &dev.interface, dev.hw_address.as_deref());
    persist_capture_metadata(state);
    state.save(state_path)?;

    let forbidden = build_forbidden(state, dev.hw_address.as_deref());
    // Roadmap M2 "Integration": when a persona is active, its `oui_pool`
    // and `mac_byte_pattern` shape the generator. Falling back to the
    // global `[mac] oui_pool` keeps the v0.2.x slider behaviour the
    // default — no regression for users who haven't opted into a persona.
    let active_persona = persona::active_for(config, None, persona::resolve::default_user_root());
    let (effective_pool, suffix_pattern) = persona_shape_for(&active_persona, config);
    let opts = GenerateOptions {
        pool: &effective_pool,
        forbidden: &forbidden,
        avoid,
        suffix_pattern,
    };
    // Roadmap M2: probe-aware path runs the RFC 5227 ARP probe and the
    // IPv6 DAD probe inline with adaptive backoff. SystemProbe falls back
    // to Unsupported (=> passive checks only) when CAP_NET_RAW is missing.
    let probe_opts = ProbeOptions::for_iface(&dev.interface);
    let outcome = generator::generate_with_probe(&opts, probe, &probe_opts)?;
    let new_mac = outcome.chosen;

    // Issue #122: iterate every connection profile bound to the device,
    // not just the first one. Otherwise roaming between SSIDs surfaces
    // the un-cloned factory MAC for the profiles that didn't get touched.
    // The display-id label of the first profile is reported back as the
    // "primary" so the rotated_entry keeps the existing schema. Failures
    // on later profiles are logged but don't fail the whole rotate.
    let mut primary_id: Option<String> = None;
    for connection_path in &dev.connections {
        let id = nm::apply::read_connection_id(conn, connection_path)
            .await
            .ok()
            .flatten();
        let uuid = nm::apply::read_connection_uuid(conn, connection_path)
            .await
            .ok()
            .flatten();
        if let Err(e) = nm::apply::set_cloned_mac(conn, connection_path, dev.kind, new_mac).await {
            tracing::warn!(
                profile = ?id,
                "set_cloned_mac failed for profile: {e:#}"
            );
            continue;
        }
        // Roadmap Milestone 4b: piggyback the per-scan MAC randomization
        // write on the same rotate pass for Wi-Fi profiles. Probe-request
        // hygiene (`mac-address-randomization = 2` + scan-rand-mac=random)
        // means saved-SSID lists stop leaking and each scan burst carries
        // a fresh source MAC. Failure here is non-fatal — the profile's
        // cloned MAC is already updated; this is privacy polish on top.
        if matches!(dev.kind, DeviceKind::Wifi) && config.rf.scan_random_mac {
            if let Err(e) = nm::apply::set_scan_rand_mac(conn, connection_path).await {
                tracing::warn!(
                    profile = ?id,
                    "set_scan_rand_mac failed for profile: {e:#}"
                );
            }
        }
        if primary_id.is_none() {
            primary_id = id.clone();
        }
        if let Some(uuid) = uuid {
            let crec = state.managed.connections.entry(uuid).or_default();
            crec.current_mac = Some(new_mac.to_string());
            crec.last_rotated = Some(super::now_iso8601());
            crec.rotation_count += 1;
        }
    }

    let rec = state
        .managed
        .interfaces
        .entry(dev.interface.clone())
        .or_default();
    let previous = rec.current_mac.clone().or_else(|| dev.hw_address.clone());
    rec.current_mac = Some(new_mac.to_string());
    rec.last_rotated = Some(super::now_iso8601());
    rec.rotation_count += 1;

    let entry = RotatedEntry {
        iface: dev.interface.clone(),
        previous,
        new: new_mac.to_string(),
        connection: primary_id,
    };
    Ok((entry, outcome.attempts, outcome.chosen_token, outcome.oui_fallbacks))
}

fn explain_candidate_from_attempt(a: CandidateAttempt) -> ExplainCandidate {
    let reason = match &a.reason {
        RejectionReason::Accepted => "accepted".to_string(),
        RejectionReason::Forbidden => "forbidden (sacred original or state-cached)".to_string(),
        RejectionReason::AvoidList => {
            "avoid-list (live ARP/ND neighbour or gateway)".to_string()
        }
        RejectionReason::NotAssignable(e) => format!("not-assignable: {e}"),
        RejectionReason::ActiveCollision { peer_ip } => {
            format!(
                "active-collision (peer={})",
                peer_ip.as_deref().unwrap_or("?")
            )
        }
        RejectionReason::ProbeUnsupported(s) => format!("probe-unsupported: {s}"),
    };
    ExplainCandidate {
        mac: a.mac.to_string(),
        token: a.token,
        reason,
    }
}

/// Issue #123 / #208: cache the BURNED-IN factory MAC, never a live (possibly
/// cloned) value.
///
/// The kernel surfaces the current netdev MAC at
/// `/sys/class/net/<iface>/address`, which after even one prior rotation is
/// the cloned value — caching that as "original" makes `proteus revert`
/// restore to a non-original. We consult `factory::permanent_address` which
/// prefers `phy80211/macaddress` (Wi-Fi) then `ethtool -P` (ethernet) and
/// only accepts the live `address` when `addr_assign_type == NET_ADDR_PERM`.
///
/// Issue #208 dropped the previous `hw_hint` fallback that consulted NM's
/// live `HwAddress`. NM surfaces whatever the kernel currently reports — on
/// a driver without phy80211 *and* without `ETHTOOL_GPERMADDR`, that's the
/// live address, which post-rotation is the cloned MAC. Caching it as
/// "factory" silently undid the #123 guard. The new contract: when
/// `factory::permanent_address` returns `None`, we leave `original_macs`
/// untouched and let `proteus status` surface "no factory MAC captured" so
/// the operator can intervene rather than the tool quietly recording a
/// known-cloned value as the restoration target.
fn capture_original_mac(state: &mut State, iface: &str, _hw_hint: Option<&str>) {
    capture_original_mac_under(state, iface, |i| factory::permanent_address(i))
}

/// Test-injectable form of [`capture_original_mac`]. The closure stands in
/// for `factory::permanent_address` so unit tests don't have to read the
/// real `/sys/class/net`. Issue #200.
fn capture_original_mac_under(
    state: &mut State,
    iface: &str,
    permanent: impl Fn(&str) -> Option<String>,
) {
    if state.original_macs.contains_key(iface) {
        return;
    }
    if let Some(mac) = permanent(iface) {
        state.original_macs.insert(iface.to_string(), mac);
    }
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(super::now_iso8601());
    }
}

fn build_forbidden(state: &State, hw: Option<&str>) -> HashSet<Mac> {
    let mut set = HashSet::new();
    for mac_str in state.original_macs.values() {
        if let Ok(m) = mac_str.parse::<Mac>() {
            set.insert(m);
        }
    }
    if let Some(h) = hw
        && let Ok(m) = h.parse::<Mac>()
    {
        set.insert(m);
    }
    for rec in state.managed.interfaces.values() {
        if let Some(m) = rec.current_mac.as_ref().and_then(|s| s.parse::<Mac>().ok()) {
            set.insert(m);
        }
    }
    set
}

/// Map an active [`persona::Persona`] to the (pool, suffix_pattern) the
/// generator should use. When `persona` is `None` we fall through to the
/// global `[mac] oui_pool` — the v0.2.x default. When `persona` is set
/// but its `oui_pool` doesn't resolve to any known prefixes (every token
/// was unknown / unparseable), the persona pool is dropped and the
/// global slider's pool wins so apply doesn't fail loud on a typo.
fn persona_shape_for(
    persona: &Option<crate::persona::Persona>,
    config: &Config,
) -> (Vec<String>, Option<ByteSuffixPattern>) {
    let Some(p) = persona else {
        return (config.mac.oui_pool.clone(), None);
    };
    // Only Stealth personas drive the OUI pool today; Randomizer mirrors
    // are content-identical to the existing slider and use the global
    // pool. Per roadmap Milestone 2: "Override `pool: &OuiPool` when
    // `kind = stealth`."
    let stealth = matches!(p.kind, crate::persona::PersonaKind::Stealth);
    let resolved = oui::resolve_vendor_tokens(&p.oui_pool);
    let pool = if stealth && !resolved.is_empty() {
        // The generator iterates token strings, not raw prefixes. Use
        // the persona's own token list so adaptive backoff can advance
        // through `["apple", "intel"]` token-by-token. This relies on
        // `Vendor::from_pool_token` understanding every persona token —
        // verified by `oui::resolve_vendor_tokens` having returned a
        // non-empty vec just now.
        p.oui_pool.clone()
    } else {
        config.mac.oui_pool.clone()
    };
    let suffix = p
        .mac_byte_pattern
        .as_deref()
        .and_then(|s| match ByteSuffixPattern::parse(s) {
            Ok(pat) => Some(pat),
            Err(e) => {
                tracing::warn!(
                    persona = %p.id,
                    pattern = %s,
                    error = %format!("{e:#}"),
                    "ignoring malformed mac_byte_pattern; rolling all 3 trailing bytes"
                );
                None
            }
        });
    (pool, suffix)
}

fn print_report(report: &RotateReport, explain: bool) {
    for r in &report.rotated {
        let prev = r.previous.as_deref().unwrap_or("?");
        match &r.connection {
            Some(id) => println!("rotated {} ({}): {} -> {}", r.iface, id, prev, r.new),
            None => println!("rotated {}: {} -> {}", r.iface, prev, r.new),
        }
    }
    for s in &report.skipped {
        println!("skipped {}: {}", s.iface, s.reason);
    }
    if explain {
        // Persona banner: the operator wants to see which OUI pool was
        // actually in scope. Empty when no persona is active.
        match &report.active_persona {
            Some(id) => println!("explain: active persona = '{id}' (persona oui_pool in use)"),
            None => println!("explain: no persona active; global [mac] oui_pool in use"),
        }
        for entry in &report.explain {
            println!(
                "explain {}: chosen-token={} oui-fallbacks={} candidates={}",
                entry.iface,
                entry.chosen_token,
                entry.oui_fallbacks,
                entry.candidates.len()
            );
            for c in &entry.candidates {
                println!("  - {} [{}] {}", c.mac, c.token, c.reason);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::mac::probe::{MockProbe, ProbeOutcome};

    /// Build a stub `permanent_address` lookup so tests don't poke real sysfs.
    /// Issue #200: the previous test read `/sys/class/net/eth0` directly which
    /// flaked on hosts that actually had an `eth0`. The injected closure is
    /// the production-equivalent of `factory::permanent_address_under`.
    fn stub_permanent(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |iface| map.get(iface).map(|s| s.to_string())
    }

    /// Issue #119 — sacred-originals invariant. `rotate_one` now saves
    /// state.json AFTER `capture_original_mac` and BEFORE the
    /// `nm::set_cloned_mac` DBus write. This test pins the round-trip half:
    /// a captured factory MAC must survive a crash between save and the
    /// DBus mutation so revert can restore it.
    #[test]
    fn captured_factory_mac_persists_to_disk() {
        let dir = crate::testing::TempRoot::new("rotate");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        let lookup = stub_permanent(HashMap::from([
            ("wlan0", "aa:bb:cc:dd:ee:ff"),
            ("eth0", "11:22:33:44:55:66"),
        ]));
        capture_original_mac_under(&mut state, "wlan0", &lookup);
        capture_original_mac_under(&mut state, "eth0", &lookup);
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "wlan0 factory MAC must be on disk before any DBus mutation"
        );
        assert_eq!(
            loaded.original_macs.get("eth0").map(String::as_str),
            Some("11:22:33:44:55:66")
        );
        assert!(loaded.captured_at.is_some());
    }

    /// `capture_original_mac` is capture-once: a second call with a
    /// different MAC must not clobber the first capture.
    #[test]
    fn capture_original_mac_is_idempotent() {
        let mut state = State::default();
        let first = stub_permanent(HashMap::from([("wlan0", "aa:bb:cc:dd:ee:ff")]));
        let second = stub_permanent(HashMap::from([("wlan0", "00:00:00:00:00:00")]));
        capture_original_mac_under(&mut state, "wlan0", &first);
        capture_original_mac_under(&mut state, "wlan0", &second);
        assert_eq!(
            state.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "second capture must not overwrite (sacred-originals)"
        );
    }

    /// Issue #208 — when no factory source produces a MAC (no phy80211, no
    /// `ethtool -P`, and the live address fails the `addr_assign_type` guard),
    /// `capture_original_mac` must leave `original_macs` empty rather than
    /// papering over with a known-cloned value. The previous behaviour fell
    /// back to NM's live `HwAddress`, silently undoing the #123 guard on
    /// drivers without phy80211 / `ETHTOOL_GPERMADDR`.
    #[test]
    fn capture_skips_when_factory_lookup_yields_none() {
        let mut state = State::default();
        let empty = stub_permanent(HashMap::new());
        capture_original_mac_under(&mut state, "eth0", &empty);
        assert!(
            state.original_macs.get("eth0").is_none(),
            "no factory source — must not cache the live (cloned) address"
        );
    }

    // === Roadmap M2 — collision-handling integration via MockProbe ===

    /// Pin the explain-candidate formatter shape so the human/JSON output
    /// stays stable. The test caller doesn't have to spin up a full NM
    /// device; we just exercise the conversion.
    #[test]
    fn explain_candidate_from_attempt_renders_each_reason() {
        let mac: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let cases = [
            (RejectionReason::Accepted, "accepted"),
            (RejectionReason::Forbidden, "forbidden"),
            (RejectionReason::AvoidList, "avoid-list"),
            (
                RejectionReason::NotAssignable("oops".into()),
                "not-assignable",
            ),
            (
                RejectionReason::ActiveCollision {
                    peer_ip: Some("10.0.0.1".into()),
                },
                "active-collision",
            ),
            (
                RejectionReason::ProbeUnsupported("no cap"),
                "probe-unsupported",
            ),
        ];
        for (reason, needle) in cases {
            let attempt = CandidateAttempt {
                mac,
                token: "apple".into(),
                reason,
            };
            let rendered = explain_candidate_from_attempt(attempt);
            assert!(
                rendered.reason.contains(needle),
                "expected reason to contain {needle}, got {:?}",
                rendered.reason
            );
        }
    }

    /// `--explain` mode must collect at least one candidate-considered line
    /// per successful rotation. Drives the probe-aware generator directly
    /// (no DBus needed) to confirm the data flow.
    #[test]
    fn explain_mode_records_at_least_one_candidate_per_rotation() {
        let pool: Vec<String> = vec!["apple".into()];
        let forbidden = HashSet::new();
        let avoid = HashSet::new();
        let probe = MockProbe::responds(false);
        let opts = GenerateOptions {
            pool: &pool,
            forbidden: &forbidden,
            avoid: &avoid,
            suffix_pattern: None,
        };
        let probe_opts = {
            let mut p = ProbeOptions::for_iface("wlan0");
            p.run_nd_probe = false;
            p
        };
        let outcome = generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
        let candidates: Vec<ExplainCandidate> = outcome
            .attempts
            .into_iter()
            .map(explain_candidate_from_attempt)
            .collect();
        assert!(!candidates.is_empty(), "--explain must surface candidates");
        assert!(
            candidates.last().unwrap().reason.contains("accepted"),
            "last attempt must be the accepted one, got {:?}",
            candidates.last().unwrap().reason
        );
    }

    /// Gateway-MAC exclusion: pin that the avoid set's gateway MAC is
    /// never selected even with many trials.
    #[test]
    fn gateway_mac_in_avoid_set_is_never_picked() {
        let pool: Vec<String> = vec!["apple".into()];
        let forbidden = HashSet::new();
        let gw: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let mut avoid = HashSet::new();
        avoid.insert(gw);
        let probe = MockProbe::responds(false);
        let opts = GenerateOptions {
            pool: &pool,
            forbidden: &forbidden,
            avoid: &avoid,
            suffix_pattern: None,
        };
        let mut probe_opts = ProbeOptions::for_iface("wlan0");
        probe_opts.run_nd_probe = false;
        for _ in 0..100 {
            let outcome =
                generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
            assert_ne!(outcome.chosen, gw, "gateway MAC must not be chosen");
        }
    }

    /// Recent-neighbour table feeds `avoid` so a neighbour that briefly
    /// dropped off `/proc/net/arp` is still excluded.
    #[test]
    fn recent_table_member_lands_in_avoid_set() {
        let table = arp::RecentNeighbourTable::with_window(std::time::Duration::from_secs(300));
        let m: Mac = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        table.record_at(m, 1_000_000);
        let mut avoid = HashSet::new();
        for v in table.current_macs_at(1_000_001) {
            avoid.insert(v);
        }
        assert!(
            avoid.contains(&m),
            "recent-neighbour table must contribute to avoid set"
        );
    }

    // === Roadmap M2 "Integration" — persona-aware MAC OUI shaping ===
    //
    // The integration tests assert that when `[persona] active = "iphone-15"`
    // is set, `generate_with_probe` picks from Apple OUIs (not the global
    // mac.oui_pool), and that when no persona is active the v0.2.x
    // behaviour is preserved.

    use crate::config::PerSsidPolicy;
    use crate::mac::oui::APPLE;

    fn cfg_with_global_pool() -> Config {
        let mut c = crate::profile::Profile::Med.baseline();
        c.persona.active = None;
        c.per_ssid.clear();
        // Use a non-Apple global pool so the test below distinguishes
        // "persona path took us to Apple" vs "global path also has Apple".
        c.mac.oui_pool = vec!["intel".into(), "samsung".into()];
        c
    }

    /// Persona-active path: with `active = "iphone-15"`, the generator
    /// picks from Apple OUIs even though the global `mac.oui_pool` is
    /// Intel + Samsung.
    #[test]
    fn persona_active_drives_generator_to_apple_oui_pool() {
        let mut cfg = cfg_with_global_pool();
        cfg.persona.active = Some("iphone-15".into());
        let persona = persona::active_for(&cfg, None, persona::resolve::default_user_root())
            .expect("iphone-15 must load from built-ins");
        let (pool, _suffix) = persona_shape_for(&Some(persona), &cfg);
        // The persona declares `oui_pool = ["apple"]` — that's what the
        // shape helper must surface.
        assert_eq!(pool, vec!["apple".to_string()]);

        // Drive the generator with the resolved pool and verify every MAC
        // it produces lives in the Apple range.
        let forbidden = HashSet::new();
        let avoid = HashSet::new();
        let probe = MockProbe::responds(false);
        let opts = GenerateOptions {
            pool: &pool,
            forbidden: &forbidden,
            avoid: &avoid,
            suffix_pattern: None,
        };
        let mut probe_opts = ProbeOptions::for_iface("wlan0");
        probe_opts.run_nd_probe = false;
        for _ in 0..32 {
            let out = generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
            let oui = &out.chosen.octets()[..3];
            assert!(
                APPLE.iter().any(|p| p.as_slice() == oui),
                "persona-shaped MAC {} should land in an Apple OUI",
                out.chosen
            );
        }
    }

    /// Persona-unset path: with no persona, the global pool drives the
    /// generator and behaviour is exactly v0.2.x. Surface: the shape
    /// helper returns the config's pool verbatim and `suffix_pattern` is
    /// `None`.
    #[test]
    fn persona_unset_keeps_global_pool_and_no_suffix_pattern() {
        let cfg = cfg_with_global_pool();
        let persona = persona::active_for(&cfg, None, persona::resolve::default_user_root());
        let (pool, suffix) = persona_shape_for(&persona, &cfg);
        assert_eq!(pool, cfg.mac.oui_pool);
        assert!(suffix.is_none());
    }

    /// Per-SSID override beats the globally-active persona at shape time.
    #[test]
    fn per_ssid_persona_override_beats_global_persona() {
        let mut cfg = cfg_with_global_pool();
        cfg.persona.active = Some("iphone-15".into());
        cfg.per_ssid.insert(
            "coffee-shop".into(),
            PerSsidPolicy {
                persona: Some("pixel-8".into()),
                ..PerSsidPolicy::default()
            },
        );
        let p = persona::active_for(&cfg, Some("coffee-shop"), persona::resolve::default_user_root())
            .expect("pixel-8 must load");
        assert_eq!(p.id, "pixel-8");
        // pixel-8's pool resolves through the vendor table; Google is the
        // canonical one.
        let (pool, _) = persona_shape_for(&Some(p), &cfg);
        assert!(pool.iter().any(|t| t == "google"));
    }

    /// Persona missing on disk is not fatal: the resolver warn-logs and
    /// returns `None`, the rotate path falls through to the global pool.
    #[test]
    fn persona_id_set_but_unknown_falls_through_to_global() {
        let mut cfg = cfg_with_global_pool();
        cfg.persona.active = Some("definitely-not-a-real-persona-xyz".into());
        let p = persona::active_for(&cfg, None, persona::resolve::default_user_root());
        assert!(p.is_none());
        let (pool, _) = persona_shape_for(&p, &cfg);
        assert_eq!(pool, cfg.mac.oui_pool);
    }

    /// Randomizer-kind personas don't drive the OUI pool — they're
    /// content-identical to the global slider. Pin the surface so a
    /// future persona type can't silently force the pool through.
    #[test]
    fn randomizer_persona_keeps_global_pool() {
        let mut cfg = cfg_with_global_pool();
        cfg.persona.active = Some("randomizer-med".into());
        let p = persona::active_for(&cfg, None, persona::resolve::default_user_root())
            .expect("randomizer-med builtin must load");
        assert!(matches!(p.kind, crate::persona::PersonaKind::Randomizer));
        let (pool, _) = persona_shape_for(&Some(p), &cfg);
        assert_eq!(pool, cfg.mac.oui_pool, "randomizer mirrors keep slider pool");
    }

    /// `mac_byte_pattern` literal bytes pin the corresponding trailing
    /// MAC byte. Pin the parser surface here; the generator-level
    /// behaviour is tested in `mac::generator::tests`.
    #[test]
    fn mac_byte_pattern_parses_xx_and_literal_slots() {
        let p = ByteSuffixPattern::parse("01:23:xx").unwrap();
        assert_eq!(p.bytes, [Some(0x01), Some(0x23), None]);
        let p = ByteSuffixPattern::parse("xx-xx-xx").unwrap();
        assert_eq!(p.bytes, [None, None, None]);
        // Wrong byte count -> error so a hand-edited persona surfaces it.
        assert!(ByteSuffixPattern::parse("01:23").is_err());
        assert!(ByteSuffixPattern::parse("zz:bb:cc").is_err());
    }

    /// Active probe collision -> retry; eventual acceptance. End-to-end
    /// shape: collide once, then succeed, and confirm the rotated MAC is
    /// not the one that collided.
    #[test]
    fn collision_then_success_chooses_a_different_candidate() {
        let pool: Vec<String> = vec!["apple".into()];
        let forbidden = HashSet::new();
        let avoid = HashSet::new();
        let probe = MockProbe::new();
        probe.queue_arp(ProbeOutcome::Collision {
            peer_ip: Some("192.168.1.5".into()),
        });
        let opts = GenerateOptions {
            pool: &pool,
            forbidden: &forbidden,
            avoid: &avoid,
            suffix_pattern: None,
        };
        let mut probe_opts = ProbeOptions::for_iface("wlan0");
        probe_opts.run_nd_probe = false;
        let outcome = generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
        let collided = outcome
            .attempts
            .iter()
            .find(|a| matches!(a.reason, RejectionReason::ActiveCollision { .. }))
            .expect("a collision was recorded")
            .mac;
        assert_ne!(
            outcome.chosen, collided,
            "chosen MAC must not be the collided one"
        );
    }
}
