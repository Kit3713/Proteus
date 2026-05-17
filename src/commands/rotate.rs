// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::backend::{BackendDevice, BackendKind, ConnectionRef, NetworkBackend};
use crate::config::Config;
use crate::display::display_safe;
use crate::exit;
use crate::mac::generator::{
    self, ByteSuffixPattern, CandidateAttempt, GenerateOptions, ProbeOptions, RejectionReason,
};
use crate::mac::oui;
use crate::mac::probe::{Probe, SystemProbe};
use crate::mac::{Mac, arp, factory};
use crate::persona;
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
pub(crate) struct RotateReport {
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

#[derive(Debug, Clone, Serialize)]
struct ExplainCandidate {
    mac: String,
    token: String,
    reason: String,
}

/// Issue #395: per-iface JSON summary line. `outcome` is the categorical
/// verb (`rotated` / `skipped`) chosen so GUI/dispatcher callers can
/// branch on a stable token instead of regex-matching the human prose.
/// `explain` is populated only when `--explain` is set; serde skips it
/// otherwise so the common-case wire format stays minimal.
#[derive(Debug, Serialize)]
struct RotateSummary {
    iface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connection: Option<String>,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    explain: Option<ExplainSummary>,
}

/// Per-iface explain trace folded into the JSON summary so a caller
/// doesn't need to correlate two arrays by iface name.
#[derive(Debug, Serialize)]
struct ExplainSummary {
    chosen_token: String,
    oui_fallbacks: usize,
    candidates: Vec<ExplainCandidate>,
}

#[derive(Debug, Serialize)]
struct RotateSummaryEnvelope {
    results: Vec<RotateSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_persona: Option<String>,
}

/// Issue #294: maximum byte length of an operator-supplied `--reason`
/// string. Anything longer is truncated at byte 253 + `"..."` so the
/// rotated state.json record and the rotate-time log line are bounded.
/// The cap is in bytes (not chars) so a hostile multi-byte input can't
/// silently bloat state.json — and we truncate on a char boundary so
/// the stored form is still valid UTF-8.
pub(crate) const REASON_MAX_BYTES: usize = 256;

/// Issue #294: normalise an operator-supplied `--reason` string into the
/// shape we persist + log:
///
/// 1. Strip control bytes (C0 + DEL + C1 + bidi overrides) so a hostile
///    paste can't inject ANSI escapes or NUL into journald output. This
///    is the same display-safety class Stream 9 already enforces at the
///    print sites; doing it once at intake means every downstream caller
///    (`state.json` reader, `tracing::info!` log, future JSON shape) is
///    safe by construction.
/// 2. Truncate to [`REASON_MAX_BYTES`] bytes, on a char boundary, with
///    a trailing `"..."` marker so the operator sees the clamp landed.
///
/// Empty inputs (`Some("")` or all-control bytes) collapse to `None` so
/// the persisted state stays compact (no `reason: ""` entries) and the
/// log line is suppressed.
pub(crate) fn sanitize_reason(raw: &str) -> Option<String> {
    // Strip the same class of bytes `display::display_safe` escapes
    // (C0 + DEL + C1 + bidi overrides), then trim outer whitespace so
    // `--reason "  foo  "` lands as `"foo"`. Backslashes pass through:
    // unlike a *display* path that has to round-trip unambiguously,
    // the stored audit string is plain text and a literal `\` carries
    // no terminal hazard.
    let mut cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            if *c == ' ' {
                return true;
            }
            if cp < 0x20 || cp == 0x7f {
                return false;
            }
            if (0x80..=0x9f).contains(&cp) {
                return false;
            }
            if matches!(cp, 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069) {
                return false;
            }
            true
        })
        .collect();
    // Inner whitespace from stripped control bytes (e.g. `foo\n\tbar`
    // after the filter is `foobar`) is intentional, but the outer trim
    // above doesn't catch padding that's only revealed after stripping.
    // A second trim handles `--reason " \x00 "` → empty.
    let after_strip = cleaned.trim();
    if after_strip.is_empty() {
        return None;
    }
    if after_strip.len() != cleaned.len() {
        cleaned = after_strip.to_string();
    }
    if cleaned.len() > REASON_MAX_BYTES {
        // Truncate to 253 bytes on a char boundary, then append "..."
        // so the total is <= 256 bytes. `floor_char_boundary` would be
        // ideal but is unstable; walk down until we land on one.
        let mut cut = 253;
        while cut > 0 && !cleaned.is_char_boundary(cut) {
            cut -= 1;
        }
        cleaned.truncate(cut);
        cleaned.push_str("...");
        tracing::warn!(
            original_bytes = raw.len(),
            truncated_bytes = cleaned.len(),
            "rotate --reason: input exceeded {REASON_MAX_BYTES} bytes; truncated"
        );
    }
    Some(cleaned)
}

pub fn run(
    iface_filter: Option<&str>,
    yes: bool,
    explain: bool,
    json: bool,
    reason: Option<&str>,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if explain {
        tracing::debug!("rotate --explain enabled (verbose candidate trace)");
    }
    // Issue #294: sanitize + log the audit string up front so the log
    // line lands even if the rotation later skips or fails. The
    // persisted form goes onto each rotated iface's state record below.
    let reason_owned = reason.and_then(sanitize_reason);
    if let Some(r) = reason_owned.as_deref() {
        tracing::info!(reason = %display_safe(r), "rotate: operator-supplied reason");
    }
    if let Err(code) = super::require_yes(
        yes,
        "'rotate' is mutating (writes new MACs to the configured backend)",
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
        // Roadmap Milestone 1: every NM-specific zbus call on the
        // command path goes through `crate::backend::*` so a future
        // `--backend networkd|raw` flag lifts straight into here.
        let backend = crate::backend::select::select(&config.backend.driver).await?;
        run_with_backend(
            backend.as_ref(),
            iface_filter,
            &config,
            &avoid,
            &probe,
            explain,
            reason_owned.as_deref(),
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
        eprintln!(
            "proteus: no managed interfaces matched (backend = {})",
            config.backend.driver
        );
        if json {
            let envelope = RotateSummaryEnvelope {
                results: Vec::new(),
                active_persona: report.active_persona.clone(),
            };
            super::print_json(&envelope)?;
        }
        return Ok(exit::GENERIC_ERROR);
    }

    if json {
        let envelope = build_summary_envelope(&report, explain);
        super::print_json(&envelope)?;
    } else {
        print_report(&report, explain);
    }
    Ok(exit::SUCCESS)
}

/// Async core split out for testability. Drives the rotation loop
/// against any [`NetworkBackend`] — production gives it the
/// auto-selected one, unit tests give it a `MockBackend`.
///
/// Issue #294: `reason` is the sanitized audit string (already routed
/// through [`sanitize_reason`]) and is stamped onto every rotated
/// iface's state record. Passing `None` keeps the pre-#294 behaviour —
/// rotations made without `--reason` leave the state record's
/// `reason` field untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_backend<P: Probe + ?Sized>(
    backend: &dyn NetworkBackend,
    iface_filter: Option<&str>,
    config: &Config,
    avoid: &HashSet<Mac>,
    probe: &P,
    explain: bool,
    reason: Option<&str>,
    state: &mut State,
    state_path: &Path,
) -> Result<RotateReport> {
    let devices = backend.list_devices().await?;
    let mut report = RotateReport {
        rotated: Vec::new(),
        skipped: Vec::new(),
        explain: Vec::new(),
        active_persona: persona::active_for(config, None, persona::resolve::default_user_root())
            .map(|p| p.id),
    };
    for dev in devices {
        if let Some(f) = iface_filter
            && dev.iface != f
        {
            continue;
        }
        if !matches!(dev.kind, BackendKind::Wifi | BackendKind::Ethernet) {
            if iface_filter.is_some() {
                report.skipped.push(SkippedEntry {
                    iface: dev.iface.clone(),
                    reason: format!("device kind {:?} not supported", dev.kind),
                });
            }
            continue;
        }
        if !dev.managed && iface_filter.is_none() {
            // Quietly skip when iterating all devices.
            continue;
        }
        if let Some(rec) = state.managed.interfaces.get(&dev.iface)
            && let Some(pin) = &rec.pinned
        {
            report.skipped.push(SkippedEntry {
                iface: dev.iface.clone(),
                reason: format!("pinned to {pin}"),
            });
            continue;
        }
        match rotate_one(
            backend, &dev, config, avoid, probe, reason, state, state_path,
        )
        .await
        {
            Ok((entry, attempts, chosen_token, oui_fallbacks)) => {
                if explain {
                    report.explain.push(ExplainEntry {
                        iface: dev.iface.clone(),
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
                iface: dev.iface.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn rotate_one<P: Probe + ?Sized>(
    backend: &dyn NetworkBackend,
    dev: &BackendDevice,
    config: &Config,
    avoid: &HashSet<Mac>,
    probe: &P,
    reason: Option<&str>,
    state: &mut State,
    state_path: &Path,
) -> Result<(RotatedEntry, Vec<CandidateAttempt>, String, usize)> {
    if dev.connections.is_empty() {
        anyhow::bail!("no connection profile available; see proteus wiki rotation");
    }

    // Capture-then-save-then-mutate: the original factory MAC must be
    // durable on disk BEFORE we ask the backend to set a cloned MAC.
    // Otherwise a crash between the backend write and the final
    // state.save() at the end of `run` would lose the factory MAC and
    // turn `revert` into a no-op (sacred-originals invariant; issue #119).
    capture_original_mac(state, &dev.iface, dev.hw_address.as_deref());
    persist_capture_metadata(state);
    state.save(state_path)?;

    let forbidden = build_forbidden(state, &dev.iface, dev.hw_address.as_deref());
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
    let probe_opts = ProbeOptions::for_iface(&dev.iface);
    let outcome = generator::generate_with_probe(&opts, probe, &probe_opts)?;
    let new_mac = outcome.chosen;

    // GH#366: read connection metadata BEFORE the backend write but
    // commit `state.managed.connections` AFTER `set_cloned_mac` returns
    // Ok. Pre-fix, the per-connection state book-keeping happened in
    // the same loop as the metadata read — landing on disk regardless
    // of whether the backend actually applied the new MAC. A backend
    // failure (NM `Update` rejected, version mismatch under concurrent
    // edit, etc.) then left a permanent ghost: `state.json` claimed
    // the connection had MAC X while NM still held the old MAC, and
    // `proteus revert` would walk the ghost back into a state nobody
    // ever observed live.
    let mut primary_id: Option<String> = None;
    let mut uuid_writes: Vec<String> = Vec::new();
    let connections: Vec<ConnectionRef> = backend.list_connections(dev).await?;
    for cref in &connections {
        let id = backend.read_connection_id(cref).await.ok().flatten();
        let uuid = backend.read_connection_uuid(cref).await.ok().flatten();
        if primary_id.is_none() {
            primary_id = id.clone();
        }
        if let Some(u) = uuid {
            uuid_writes.push(u);
        }
    }
    backend
        .set_cloned_mac(dev, new_mac)
        .await
        .with_context(|| format!("setting cloned MAC on {}", dev.iface))?;

    // Backend confirmed — now safe to record the post-rotate state.
    // Both the per-connection map and the per-iface record land in the
    // same critical section so a partial state.save() can't catch the
    // managed.connections write without the matching managed.interfaces
    // bump.
    let now = super::now_iso8601();
    for uuid in uuid_writes {
        let crec = state.managed.connections.entry(uuid).or_default();
        crec.current_mac = Some(new_mac.to_string());
        crec.last_rotated = Some(now.clone());
        crec.rotation_count += 1;
    }

    let rec = state
        .managed
        .interfaces
        .entry(dev.iface.clone())
        .or_default();
    let previous = rec.current_mac.clone().or_else(|| dev.hw_address.clone());
    rec.current_mac = Some(new_mac.to_string());
    rec.last_rotated = Some(now);
    rec.rotation_count += 1;
    // Issue #294: stamp the operator-supplied audit string alongside
    // `last_rotated`. A `None` reason leaves the previous value
    // untouched — automated callers (apply / events daemon) that don't
    // pass `--reason` should not silently wipe an operator-supplied
    // `--reason` from the most-recent prior rotation. The audit-string
    // shape is per-rotation by intent, so the next rotate that *does*
    // pass `--reason` overwrites the prior value as expected.
    if let Some(r) = reason {
        rec.reason = Some(r.to_string());
    }

    let entry = RotatedEntry {
        iface: dev.iface.clone(),
        previous,
        new: new_mac.to_string(),
        connection: primary_id,
    };
    Ok((
        entry,
        outcome.attempts,
        outcome.chosen_token,
        outcome.oui_fallbacks,
    ))
}

fn explain_candidate_from_attempt(a: CandidateAttempt) -> ExplainCandidate {
    let reason = match &a.reason {
        RejectionReason::Accepted => "accepted".to_string(),
        RejectionReason::Forbidden => "forbidden (sacred original or state-cached)".to_string(),
        RejectionReason::AvoidList => "avoid-list (live ARP/ND neighbour or gateway)".to_string(),
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
/// Issue #208 dropped the previous `hw_hint` fallback that consulted the
/// backend's live `HwAddress`. Backends surface whatever the kernel
/// currently reports — on a driver without phy80211 *and* without
/// `ETHTOOL_GPERMADDR`, that's the live address, which post-rotation is
/// the cloned MAC. Caching it as "factory" silently undid the #123
/// guard. The new contract: when `factory::permanent_address` returns
/// `None`, we leave `original_macs` untouched and let `proteus status`
/// surface "no factory MAC captured" so the operator can intervene
/// rather than the tool quietly recording a known-cloned value as the
/// restoration target.
fn capture_original_mac(state: &mut State, iface: &str, _hw_hint: Option<&str>) {
    capture_original_mac_under(state, iface, factory::permanent_address)
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

/// GH#379: forbid every sacred-original MAC and every OTHER interface's
/// currently-applied clone, but NOT this interface's own live MAC. The
/// previous shape always added the device's current `hw` to the
/// forbidden set, which made `apply` non-idempotent: a re-run with no
/// config changes had to pick a different MAC, because the one we'd
/// just set was now off-limits.
///
/// The current MAC for the iface being rotated is implicitly a valid
/// candidate again — the generator may re-pick it. The change is
/// observable as: two back-to-back `apply` runs against an unchanged
/// system can land on the same MAC. Originals and other-iface clones
/// remain forbidden so cross-iface collisions are still impossible.
fn build_forbidden(state: &State, iface: &str, hw: Option<&str>) -> HashSet<Mac> {
    let mut set = HashSet::new();
    // Sacred originals: every captured factory MAC is permanently off
    // the candidate list. Includes this iface's own factory MAC, so the
    // generator can never accidentally pick it.
    for mac_str in state.original_macs.values() {
        if let Ok(m) = mac_str.parse::<Mac>() {
            set.insert(m);
        }
    }
    // Other interfaces' clones: forbid them so two ifaces never end up
    // sharing a MAC. Skip THIS iface's record so a re-apply can re-pick
    // the current value (idempotency).
    for (rec_iface, rec) in &state.managed.interfaces {
        if rec_iface == iface {
            continue;
        }
        if let Some(m) = rec.current_mac.as_ref().and_then(|s| s.parse::<Mac>().ok()) {
            set.insert(m);
        }
    }
    // `hw` (the live driver-reported MAC) is intentionally NOT added.
    // Pre-#379 it was — that's exactly the non-idempotency. On a first
    // rotation `hw` equals the factory MAC, which is already covered
    // by the `original_macs` loop above. On a subsequent rotation it
    // equals this iface's current_mac, which we want to leave
    // pickable.
    let _ = hw;
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

/// Issue #395: fold `RotateReport` into the per-iface JSON summary
/// envelope. The categorical `outcome` token is what a dispatcher
/// branches on; the human reason rides along on skips. iface and
/// connection id flow through `display_safe` because they originate
/// from NM / kernel surfaces (issue #367).
fn build_summary_envelope(report: &RotateReport, explain: bool) -> RotateSummaryEnvelope {
    let mut results: Vec<RotateSummary> =
        Vec::with_capacity(report.rotated.len() + report.skipped.len());
    for r in &report.rotated {
        let explain_entry = if explain {
            report
                .explain
                .iter()
                .find(|e| e.iface == r.iface)
                .map(explain_summary_from_entry)
        } else {
            None
        };
        results.push(RotateSummary {
            iface: display_safe(&r.iface).into_owned(),
            old_mac: r.previous.clone(),
            new_mac: Some(r.new.clone()),
            connection: r
                .connection
                .as_deref()
                .map(|c| display_safe(c).into_owned()),
            outcome: "rotated",
            reason: None,
            explain: explain_entry,
        });
    }
    for s in &report.skipped {
        results.push(RotateSummary {
            iface: display_safe(&s.iface).into_owned(),
            old_mac: None,
            new_mac: None,
            connection: None,
            outcome: "skipped",
            reason: Some(s.reason.clone()),
            explain: None,
        });
    }
    RotateSummaryEnvelope {
        results,
        active_persona: report.active_persona.clone(),
    }
}

fn explain_summary_from_entry(entry: &ExplainEntry) -> ExplainSummary {
    ExplainSummary {
        chosen_token: entry.chosen_token.clone(),
        oui_fallbacks: entry.oui_fallbacks,
        candidates: entry.candidates.clone(),
    }
}

fn print_report(report: &RotateReport, explain: bool) {
    // Issue #367 (terminal-injection cluster): iface names come from
    // NetworkManager, which surfaces whatever the kernel reports — and on
    // virtual ifaces, what the kernel reports may include attacker-shaped
    // bytes (NM dispatcher driven by an SSID-derived connection name).
    // Sanitize before echo. Connection ids come from NM settings, which a
    // hostile profile push can populate; sanitize too.
    for r in &report.rotated {
        let prev = r.previous.as_deref().unwrap_or("?");
        let iface_safe = display_safe(&r.iface);
        match &r.connection {
            Some(id) => println!(
                "rotated {} ({}): {} -> {}",
                iface_safe,
                display_safe(id),
                prev,
                r.new
            ),
            None => println!("rotated {}: {} -> {}", iface_safe, prev, r.new),
        }
    }
    for s in &report.skipped {
        println!("skipped {}: {}", display_safe(&s.iface), s.reason);
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

/// `proteus rotate-if-needed --cooldown <secs>` — typed entry point
/// for the NM dispatcher (issue #206-C). Replaces the previous
/// `proteus current --json | sed -n 's/last_rotated/...'` shell
/// fragment with a single subcommand that returns the
/// [`crate::backend::RotateOutcome`] as a stable, easily-parseable
/// line plus the matching exit code.
///
/// Exit codes:
///
/// - `0` — `Rotated { new_mac }`
/// - `0` — `SkippedCooldown { remaining }` (no-op is success too)
/// - `0` — `NoFactoryMac` (operator action needed but not an error)
/// - `70` (SYSTEM_NOT_SUPPORTED) — `BackendUnavailable`
///
/// Output (stdout, one line):
///
/// - `rotated <iface>: <new-mac>`
/// - `skipped <iface>: cooldown <remaining_secs>s`
/// - `skipped <iface>: no factory MAC captured`
/// - `unavailable <iface>: backend reports unavailable`
#[allow(clippy::too_many_arguments)]
pub fn run_if_needed(
    iface: Option<&str>,
    cooldown_secs: u64,
    ssid: Option<&str>,
    yes: bool,
    explain: bool,
    reason: Option<&str>,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'rotate-if-needed' is mutating (rotates if the cooldown has elapsed)",
        "proteus help rotate",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #381: the state-path threading from the CLI ends here. The
    // backend trait's `rotate_if_needed` doesn't yet accept a state path,
    // so the cooldown read-back inside `backend/nm.rs::rotate_if_needed_inner`
    // hardcodes `crate::commands::DEFAULT_STATE_PATH`. Surface a warning
    // when an explicit `--state` was passed so the operator at least
    // sees that their override was dropped — the proper fix needs a
    // trait-signature change (Stream 5 / state-lock work) and is
    // tracked there. TODO(#381): remove this warning when the backend
    // trait grows a `state_path` parameter.
    if state_path.is_some() {
        tracing::warn!(
            "--state is currently ignored by `rotate-if-needed`; cooldown reads \
             {} (see #381)",
            crate::commands::DEFAULT_STATE_PATH
        );
    }
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();

    // Issue #294: sanitize + echo the operator-supplied reason up
    // front so the log line lands even if the rotation later trips a
    // per-SSID skip or a cooldown skip below. The persisted form is
    // threaded through the backend trait into the inner rotate so
    // each rotated iface's state record carries the same value.
    let reason_owned = reason.and_then(sanitize_reason);
    if let Some(r) = reason_owned.as_deref() {
        tracing::info!(
            reason = %display_safe(r),
            "rotate-if-needed: operator-supplied reason"
        );
    }

    // Roadmap Milestone 3: when the caller (typically the NM dispatcher)
    // tells us which SSID just came up, fold the per-SSID policy into
    // the cooldown decision before we hit the backend. A pinned MAC
    // means "never rotate on this SSID" — surface a typed skip line so
    // the dispatcher's logger captures it. A larger `rotate_interval`
    // raises the cooldown floor so per-SSID slow-rotate networks don't
    // get whip-sawed by the global cadence.
    let policy = ssid.map(|s| crate::per_ssid::resolve_for_ssid(&config, s));
    if let Some(p) = &policy
        && p.pin_mac.is_some()
    {
        let iface_label = iface.unwrap_or("(no iface)");
        if explain {
            // Issue #378: surface the policy that drove the skip so the
            // dispatcher's logger sees *why* this iface was excluded.
            println!(
                "explain rotate-if-needed: ssid={} per-SSID pin_mac is set → skipping (no rotation)",
                ssid.unwrap_or("(none)")
            );
        }
        println!("skipped {iface_label}: pinned by per-SSID policy");
        return Ok(exit::SUCCESS);
    }
    let effective_cooldown_secs = policy
        .as_ref()
        .and_then(|p| p.rotate_interval.map(|d| d.as_secs()))
        .map(|p_secs| p_secs.max(cooldown_secs))
        .unwrap_or(cooldown_secs);
    let cooldown = std::time::Duration::from_secs(effective_cooldown_secs);

    if explain {
        // Issue #378: triage hook — operators wrapping the dispatcher
        // need to see the policy and cooldown math the hot path is
        // making *before* the backend call. Print on stdout so journal
        // grep / CI capture can pick it up alongside the result line.
        let per_ssid_floor = policy
            .as_ref()
            .and_then(|p| p.rotate_interval.map(|d| d.as_secs()));
        println!(
            "explain rotate-if-needed: iface={} ssid={} \
             requested_cooldown_s={} per_ssid_floor_s={} effective_cooldown_s={} backend={}",
            iface.unwrap_or("(auto)"),
            ssid.unwrap_or("(none)"),
            cooldown_secs,
            per_ssid_floor
                .map(|s| s.to_string())
                .unwrap_or_else(|| "(none)".into()),
            effective_cooldown_secs,
            config.backend.driver,
        );
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let outcome = rt.block_on(async {
        let backend = crate::backend::select::select(&config.backend.driver).await?;
        // Pick the iface: the dispatcher always passes one. When it
        // doesn't, fall through to the first managed wifi/ethernet so
        // the command is still usable from the CLI.
        let target = match iface {
            Some(name) => name.to_string(),
            None => {
                let devs = backend.list_devices().await.unwrap_or_default();
                devs.into_iter()
                    .find(|d| {
                        matches!(d.kind, BackendKind::Wifi | BackendKind::Ethernet) && d.managed
                    })
                    .map(|d| d.iface)
                    .unwrap_or_default()
            }
        };
        if target.is_empty() {
            return Ok::<_, anyhow::Error>((
                "(no iface)".to_string(),
                crate::backend::RotateOutcome::BackendUnavailable,
            ));
        }
        let r = backend
            .rotate_if_needed(&target, cooldown, state_path, reason_owned.as_deref())
            .await?;
        Ok((target, r))
    });

    let (iface_name, outcome) = match outcome {
        Ok(t) => t,
        Err(e) => {
            eprintln!("proteus: rotate-if-needed failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    use crate::backend::RotateOutcome;
    // Issue #367 (terminal-injection cluster): iface from NM dispatcher.
    let iface_name = display_safe(&iface_name);
    match outcome {
        RotateOutcome::Rotated { new_mac } => {
            println!("rotated {iface_name}: {new_mac}");
            Ok(exit::SUCCESS)
        }
        RotateOutcome::SkippedCooldown { remaining } => {
            println!("skipped {iface_name}: cooldown {}s", remaining.as_secs());
            Ok(exit::SUCCESS)
        }
        RotateOutcome::NoFactoryMac => {
            println!("skipped {iface_name}: no factory MAC captured");
            Ok(exit::SUCCESS)
        }
        RotateOutcome::BackendUnavailable => {
            // NEV2.7: rc=70 conflates "backend unavailable" with the
            // genuine "missing nft / nl80211 / CAP_NET_ADMIN" branch the
            // dispatcher script also returns. The trait can't yet carry
            // the underlying-cause string back through the BackendUnavailable
            // variant (Stream 4 work), so at minimum we log the iface
            // and the configured driver at info-level so an operator
            // following the journal sees the context that's otherwise
            // lost. The backend-side path that rejects on permission
            // logs its own trail; this is the consumer-side hint.
            tracing::info!(
                iface = %iface_name,
                driver = %config.backend.driver,
                "rotate-if-needed: backend reports unavailable; \
                 likely missing CAP_NET_ADMIN, nl80211, or the configured driver"
            );
            println!("unavailable {iface_name}: backend reports unavailable");
            Ok(exit::SYSTEM_NOT_SUPPORTED)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::backend::mock::{MockBackend, MockCall};
    use crate::backend::{BackendDevice, BackendKind, ConnectionRef};
    use crate::mac::probe::{MockProbe, ProbeOutcome};

    /// Build a stub `permanent_address` lookup so tests don't poke real sysfs.
    /// Issue #200: the previous test read `/sys/class/net/eth0` directly which
    /// flaked on hosts that actually had an `eth0`. The injected closure is
    /// the production-equivalent of `factory::permanent_address_under`.
    fn stub_permanent(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |iface| map.get(iface).map(|s| s.to_string())
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn dev(iface: &str) -> BackendDevice {
        BackendDevice {
            iface: iface.into(),
            kind: BackendKind::Wifi,
            hw_address: Some("aa:bb:cc:dd:ee:ff".into()),
            identifier: format!("mock://{iface}"),
            connections: vec![ConnectionRef::new(format!("mock://{iface}/0"))],
            managed: true,
        }
    }

    fn cfg() -> Config {
        let mut c = crate::profile::Profile::Med.baseline();
        c.persona.active = None;
        c.per_ssid.clear();
        // Use 'apple' so the generator has a real OUI pool to pick from
        // without needing the persona machinery.
        c.mac.oui_pool = vec!["apple".into()];
        c
    }

    /// Issue #119 — sacred-originals invariant. `rotate_one` saves
    /// state.json AFTER `capture_original_mac` and BEFORE the
    /// `set_cloned_mac` write. This test pins the round-trip half:
    /// a captured factory MAC must survive a crash between save and the
    /// backend mutation so revert can restore it.
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
            "wlan0 factory MAC must be on disk before any backend mutation"
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

    /// Issue #208 — when no factory source produces a MAC, the
    /// capture path leaves `original_macs` empty rather than papering
    /// over with a known-cloned value.
    #[test]
    fn capture_skips_when_factory_lookup_yields_none() {
        let mut state = State::default();
        let empty = stub_permanent(HashMap::new());
        capture_original_mac_under(&mut state, "eth0", &empty);
        assert!(
            !state.original_macs.contains_key("eth0"),
            "no factory source — must not cache the live (cloned) address"
        );
    }

    // === Roadmap Milestone 1 — backend trait integration ===

    /// The headline acceptance assertion: `rotate::run_with_backend`
    /// drives a `MockBackend` through the trait surface, the mock
    /// records every call, and the recorded sequence shows
    /// `set_cloned_mac` got invoked on the seeded device. No NM, no
    /// DBus, no sysfs.
    #[test]
    fn rotate_run_calls_set_cloned_mac_on_mock_backend() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        let cref = device.connections[0].clone();
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        backend.insert_connection(&cref, Some("Home Wi-Fi"), Some("uuid-1"));

        let dir = crate::testing::TempRoot::new("rotate-mock");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        // Seed the factory MAC so capture-once doesn't try sysfs.
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        assert_eq!(report.rotated.len(), 1, "one iface rotated");
        assert_eq!(report.rotated[0].iface, "wlan0");

        let log = backend.call_log();
        assert!(log.iter().any(|c| matches!(c, MockCall::ListDevices)));
        assert!(
            log.iter()
                .any(|c| matches!(c, MockCall::SetClonedMac { iface, .. } if iface == "wlan0")),
            "set_cloned_mac must have landed exactly once for wlan0; log = {log:?}"
        );

        // Final mac on the mock matches what state.json carries.
        let stored = backend.cloned_mac_for("wlan0").expect("cloned mac written");
        let recorded = state.managed.interfaces["wlan0"]
            .current_mac
            .as_deref()
            .expect("state has new mac");
        assert_eq!(stored, recorded);
    }

    /// Pinned interfaces are skipped before any backend mutation runs.
    #[test]
    fn pinned_iface_is_skipped_without_set_cloned_mac() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        let dir = crate::testing::TempRoot::new("rotate-pinned");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        let rec = crate::state::InterfaceRecord {
            pinned: Some("02:00:00:00:00:99".into()),
            ..Default::default()
        };
        state.managed.interfaces.insert("wlan0".into(), rec);

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        assert_eq!(report.rotated.len(), 0);
        assert_eq!(report.skipped.len(), 1);
        let log = backend.call_log();
        assert!(
            !log.iter()
                .any(|c| matches!(c, MockCall::SetClonedMac { .. })),
            "pinned iface must not trigger set_cloned_mac"
        );
    }

    /// Iface filter mismatches every device → empty report. The trait
    /// is still consulted (so we know the filter walked the device
    /// list) but no mutator landed.
    #[test]
    fn iface_filter_no_match_yields_empty_report() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), Some("aa:bb:cc:dd:ee:ff".into()));
        let dir = crate::testing::TempRoot::new("rotate-filter");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan9"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        assert!(report.rotated.is_empty());
        assert!(report.skipped.is_empty());
    }

    /// Issue #122 mirror — `set_cloned_mac` runs even when the device
    /// has multiple connection profiles (the trait's NM impl iterates
    /// internally; the mock collapses to one but the entry-point still
    /// must call `set_cloned_mac` once with the device).
    #[test]
    fn rotate_invokes_set_cloned_mac_once_per_device() {
        let backend = MockBackend::new();
        let mut device = dev("wlan0");
        device
            .connections
            .push(ConnectionRef::new("mock://wlan0/1"));
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        let dir = crate::testing::TempRoot::new("rotate-multi");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let _ = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        let n = backend
            .call_log()
            .into_iter()
            .filter(|c| matches!(c, MockCall::SetClonedMac { .. }))
            .count();
        assert_eq!(
            n, 1,
            "one set_cloned_mac per device (the trait iterates profiles internally)"
        );
    }

    // === Roadmap M2 — collision-handling integration via MockProbe ===

    /// Pin the explain-candidate formatter shape so the human/JSON output
    /// stays stable. The test caller doesn't have to spin up a full backend
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
    /// (no backend needed) to confirm the data flow.
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
            let outcome = generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
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
        let p = persona::active_for(
            &cfg,
            Some("coffee-shop"),
            persona::resolve::default_user_root(),
        )
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
        assert_eq!(
            pool, cfg.mac.oui_pool,
            "randomizer mirrors keep slider pool"
        );
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

    // === Roadmap Milestone 1 — `rotate-if-needed` outcomes ===

    /// `RotateOutcome::Rotated` shape — exercised against a mock
    /// backend so the test doesn't depend on the live state.json
    /// arithmetic. Returned outcome carries the new MAC verbatim.
    #[test]
    fn rotate_if_needed_rotated_outcome_round_trips() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), Some("aa:bb:cc:dd:ee:ff".into()));
        let mac: Mac = "06:11:22:33:44:55".parse().unwrap();
        backend.set_rotate_outcome(
            "wlan0",
            crate::backend::RotateOutcome::Rotated { new_mac: mac },
        );
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", std::time::Duration::from_secs(60), None, None)
                .await
                .unwrap();
            assert_eq!(
                outcome,
                crate::backend::RotateOutcome::Rotated { new_mac: mac }
            );
        });
    }

    /// `RotateOutcome::SkippedCooldown` carries the remaining duration
    /// so the dispatcher can log it.
    #[test]
    fn rotate_if_needed_cooldown_outcome_round_trips() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), None);
        backend.set_rotate_outcome(
            "wlan0",
            crate::backend::RotateOutcome::SkippedCooldown {
                remaining: std::time::Duration::from_secs(15),
            },
        );
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", std::time::Duration::from_secs(60), None, None)
                .await
                .unwrap();
            assert!(matches!(
                outcome,
                crate::backend::RotateOutcome::SkippedCooldown { .. }
            ));
        });
    }

    /// `NoFactoryMac` outcome — surfaces "operator action needed but
    /// not an error". The dispatcher logs and moves on.
    #[test]
    fn rotate_if_needed_no_factory_mac_outcome_round_trips() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), None);
        backend.set_rotate_outcome("wlan0", crate::backend::RotateOutcome::NoFactoryMac);
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", std::time::Duration::from_secs(60), None, None)
                .await
                .unwrap();
            assert_eq!(outcome, crate::backend::RotateOutcome::NoFactoryMac);
        });
    }

    /// `BackendUnavailable` outcome — surfaces when the backend itself
    /// can't service the request. The dispatcher exits with
    /// SYSTEM_NOT_SUPPORTED.
    #[test]
    fn rotate_if_needed_backend_unavailable_outcome_round_trips() {
        let backend = MockBackend::new();
        backend.insert_device(dev("wlan0"), None);
        backend.set_rotate_outcome("wlan0", crate::backend::RotateOutcome::BackendUnavailable);
        rt().block_on(async {
            let outcome = backend
                .rotate_if_needed("wlan0", std::time::Duration::from_secs(60), None, None)
                .await
                .unwrap();
            assert_eq!(outcome, crate::backend::RotateOutcome::BackendUnavailable);
        });
    }

    /// GH#366: when `set_cloned_mac` returns Err (NM rejected the
    /// Update under a concurrent edit, version-mismatch, etc.) the
    /// state.managed.connections map MUST NOT carry a forward-looking
    /// `current_mac` for that profile. Pre-fix, the connection map was
    /// updated in the same loop as the metadata read — landing on
    /// disk before `set_cloned_mac` was even called — so a failed
    /// rotation persisted a ghost MAC that `proteus revert` would
    /// then walk back into a state nobody ever observed live.
    #[test]
    fn failed_set_cloned_mac_does_not_persist_ghost_in_managed_connections() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        backend.insert_device(device.clone(), Some("aa:bb:cc:dd:ee:ff".into()));
        // Seed a connection so list_connections returns a concrete UUID.
        let cref = device.connections[0].clone();
        backend.insert_connection(&cref, Some("Home"), Some("uuid-test-1234"));
        // Arm the failure: the next `set_cloned_mac` rejects.
        backend.fail_next_set_cloned_mac("simulated NM Update conflict");

        let dir = crate::testing::TempRoot::new("rotate-gh366");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        // The rotate is reported as a skip (the backend rejected the write).
        assert_eq!(report.rotated.len(), 0, "no rotation actually landed");
        assert_eq!(report.skipped.len(), 1, "one skip recorded");

        // Nothing in state.managed.connections claims a fresh MAC for
        // this UUID. Pre-fix, the entry would carry the would-have-been
        // current_mac written before set_cloned_mac was even called.
        let crec = state.managed.connections.get("uuid-test-1234");
        assert!(
            crec.map(|c| c.current_mac.is_none()).unwrap_or(true),
            "state.managed.connections must not carry a ghost MAC for failed rotations; \
             got {:?}",
            crec.and_then(|c| c.current_mac.clone()),
        );

        // And the per-iface record is similarly unbumped.
        let irec = state.managed.interfaces.get("wlan0");
        assert!(
            irec.map(|r| r.current_mac.is_none()).unwrap_or(true),
            "state.managed.interfaces must not carry a ghost MAC; got {:?}",
            irec.and_then(|r| r.current_mac.clone()),
        );
    }

    // ===== Issue #294 — `--reason` audit string =====

    /// Round-trip a state.json carrying a `reason` on a per-iface
    /// record. Old state files without the field must still load
    /// (covered by the existing `old_state_files_load` test in
    /// `state.rs`); this one pins the *forward* shape — once written,
    /// the value survives serialize → deserialize.
    #[test]
    fn reason_round_trips_through_state_json() {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".into(),
            crate::state::InterfaceRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                last_rotated: Some("2026-05-17T00:00:00Z".into()),
                rotation_count: 1,
                reason: Some("manual: lab test".into()),
                ..Default::default()
            },
        );
        let bytes = serde_json::to_vec(&s).expect("serialize");
        let back: State = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(
            back.managed
                .interfaces
                .get("wlan0")
                .and_then(|r| r.reason.as_deref()),
            Some("manual: lab test"),
            "reason must survive state.json round-trip"
        );
    }

    /// Sanitizer caps at 256 bytes — anything longer truncates to 253
    /// bytes of input + `"..."` so total <= 256.
    #[test]
    fn sanitize_reason_truncates_at_256_bytes() {
        let raw = "a".repeat(300);
        let out = sanitize_reason(&raw).expect("non-empty input");
        assert!(
            out.len() <= REASON_MAX_BYTES,
            "truncated reason must be <= {REASON_MAX_BYTES} bytes, got {} bytes: {out:?}",
            out.len()
        );
        assert!(
            out.ends_with("..."),
            "truncation marker must end the string, got {out:?}"
        );
        assert_eq!(out.len(), 256);
    }

    #[test]
    fn sanitize_reason_passes_through_at_threshold() {
        let raw = "a".repeat(REASON_MAX_BYTES);
        let out = sanitize_reason(&raw).expect("at-threshold input");
        assert_eq!(out.len(), REASON_MAX_BYTES);
        assert!(!out.ends_with("..."), "at-threshold must NOT truncate");
    }

    #[test]
    fn sanitize_reason_truncation_respects_char_boundaries() {
        let mut raw = String::new();
        raw.push_str(&"a".repeat(250));
        raw.push('🎉');
        raw.push_str(&"b".repeat(100));
        assert!(raw.len() > REASON_MAX_BYTES);
        let out = sanitize_reason(&raw).expect("non-empty");
        assert!(out.is_char_boundary(out.len()));
        assert!(out.ends_with("..."));
    }

    #[test]
    fn sanitize_reason_strips_control_bytes() {
        let cases = [
            ("foo\0bar", "foobar"),
            ("foo\x1bbar", "foobar"),
            ("foo\u{9b}bar", "foobar"),
            ("foo\x7fbar", "foobar"),
            ("foo\nbar", "foobar"),
            ("foo\rbar", "foobar"),
            ("foo\u{202e}bar", "foobar"),
            ("foo\u{200e}bar", "foobar"),
            ("foo bar", "foo bar"),
            ("foo\tbar", "foobar"),
            ("hello world!", "hello world!"),
        ];
        for (raw, want) in cases {
            let got = sanitize_reason(raw).unwrap_or_default();
            assert_eq!(
                got, want,
                "sanitize_reason({raw:?}) -> {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn sanitize_reason_collapses_empty_to_none() {
        assert!(sanitize_reason("").is_none());
        assert!(sanitize_reason("   ").is_none());
        assert!(sanitize_reason("\x00\x1b\x7f").is_none());
        assert!(sanitize_reason("  \x00  ").is_none());
    }

    /// End-to-end: `run_with_backend` stamps the sanitized `reason`
    /// into every rotated iface's state record.
    #[test]
    fn run_with_backend_stamps_reason_into_state() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        let cref = device.connections[0].clone();
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        backend.insert_connection(&cref, Some("Home Wi-Fi"), Some("uuid-r-1"));

        let dir = crate::testing::TempRoot::new("rotate-reason");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                Some("dispatcher: nm connection-up"),
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        assert_eq!(report.rotated.len(), 1);
        let rec = state
            .managed
            .interfaces
            .get("wlan0")
            .expect("iface record populated");
        assert_eq!(
            rec.reason.as_deref(),
            Some("dispatcher: nm connection-up"),
            "the sanitized reason must land on the per-iface state record",
        );
    }

    /// A `reason = None` MUST NOT clear a previously-stamped reason.
    #[test]
    fn run_with_backend_none_reason_preserves_prior_reason() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        let dir = crate::testing::TempRoot::new("rotate-reason-pres");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.managed.interfaces.insert(
            "wlan0".into(),
            crate::state::InterfaceRecord {
                reason: Some("prior: manual lab test".into()),
                ..Default::default()
            },
        );

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let _ = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });
        let rec = state.managed.interfaces.get("wlan0").expect("iface record");
        assert_eq!(
            rec.reason.as_deref(),
            Some("prior: manual lab test"),
            "a None reason must NOT wipe a prior operator-supplied audit string",
        );
    }

    // === Issue #395 — `proteus rotate --json` summary envelope ===

    #[test]
    fn rotate_json_summary_envelope_carries_expected_fields() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        let cref = device.connections[0].clone();
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        backend.insert_connection(&cref, Some("Home Wi-Fi"), Some("uuid-1"));

        let dir = crate::testing::TempRoot::new("rotate-json");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });

        let envelope = build_summary_envelope(&report, false);
        let serialized = serde_json::to_string(&envelope).expect("serialise");
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("round-trips as JSON");

        let results = parsed
            .get("results")
            .and_then(|v| v.as_array())
            .expect("results array present");
        assert_eq!(results.len(), 1, "exactly one iface rotated");
        let entry = &results[0];
        assert_eq!(entry.get("iface").and_then(|v| v.as_str()), Some("wlan0"));
        assert_eq!(
            entry.get("outcome").and_then(|v| v.as_str()),
            Some("rotated"),
            "outcome must be a stable categorical token"
        );
        assert_eq!(
            entry.get("old_mac").and_then(|v| v.as_str()),
            Some("aa:bb:cc:dd:ee:ff"),
        );
        let new_mac = entry
            .get("new_mac")
            .and_then(|v| v.as_str())
            .expect("new_mac present on a successful rotate");
        let _: Mac = new_mac.parse().expect("new_mac parses");
        assert_ne!(new_mac, "aa:bb:cc:dd:ee:ff");
        assert!(entry.get("explain").is_none());
        assert_eq!(
            entry.get("connection").and_then(|v| v.as_str()),
            Some("Home Wi-Fi"),
        );
    }

    #[test]
    fn rotate_json_summary_envelope_skipped_entry_carries_reason() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        let dir = crate::testing::TempRoot::new("rotate-json-skip");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        let rec = crate::state::InterfaceRecord {
            pinned: Some("02:00:00:00:00:99".into()),
            ..Default::default()
        };
        state.managed.interfaces.insert("wlan0".into(), rec);

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                false,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });

        let envelope = build_summary_envelope(&report, false);
        let parsed: serde_json::Value = serde_json::to_value(&envelope).expect("to_value");
        let results = parsed
            .get("results")
            .and_then(|v| v.as_array())
            .expect("results array");
        assert_eq!(results.len(), 1);
        let entry = &results[0];
        assert_eq!(
            entry.get("outcome").and_then(|v| v.as_str()),
            Some("skipped")
        );
        assert!(
            entry
                .get("reason")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("pinned")),
        );
        assert!(entry.get("new_mac").is_none());
    }

    #[test]
    fn rotate_json_summary_envelope_includes_explain_when_requested() {
        let backend = MockBackend::new();
        let device = dev("wlan0");
        let cref = device.connections[0].clone();
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        backend.insert_connection(&cref, Some("Home"), Some("uuid-explain"));

        let dir = crate::testing::TempRoot::new("rotate-json-explain");
        let state_path = dir.path.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());

        let cfg = cfg();
        let avoid: HashSet<Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let report = rt().block_on(async {
            run_with_backend(
                &backend,
                Some("wlan0"),
                &cfg,
                &avoid,
                &probe,
                true,
                None,
                &mut state,
                &state_path,
            )
            .await
            .unwrap()
        });

        let envelope = build_summary_envelope(&report, true);
        let parsed: serde_json::Value = serde_json::to_value(&envelope).expect("to_value");
        let entry = &parsed["results"][0];
        let explain = entry
            .get("explain")
            .and_then(|v| v.as_object())
            .expect("explain object attached under --explain");
        assert!(explain.contains_key("chosen_token"));
        let candidates = explain
            .get("candidates")
            .and_then(|v| v.as_array())
            .expect("candidates array");
        assert!(!candidates.is_empty());
    }
}
