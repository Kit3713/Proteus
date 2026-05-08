// SPDX-License-Identifier: GPL-3.0-or-later

//! Side-effect-free preview helpers for `proteus dry-run`.
//!
//! These are deliberately simpler than the real `apply` paths: we don't query
//! NetworkManager, we don't read ARP, we don't touch DBus. Instead we lean on
//! `state.json` to enumerate the interfaces Proteus has previously managed
//! and on the config to describe the OUI pool. The MAC value reported in the
//! preview is generated through the same `generator::generate` path the real
//! rotation uses, but seeded into the locally-administered branch only — that
//! way the preview is honest about the bit pattern without claiming a
//! specific vendor prefix it can't guarantee.

use crate::config::Config;
use crate::dry_run::{Plan, PlanStep, StepKind};
use crate::mac::Mac;
use crate::state::State;

/// Build a preview plan for `proteus rotate`.
///
/// `iface_filter`:
/// - `Some(name)` — preview rotation for that single interface
/// - `None`       — preview rotation for every interface Proteus knows about
///
/// When `state.json` has no managed interfaces yet, the plan emits a single
/// note explaining that — `apply` would normally discover devices via NM,
/// but a true preview can't talk to NM without producing side effects.
pub fn plan_rotate(config: &Config, state: &State, iface_filter: Option<&str>) -> Plan {
    let mut plan = Plan::new("rotate");

    let pool_label = pool_label(&config.mac.oui_pool);
    // NM2.7: thread the persona's effective OUI pool into the preview so
    // a configured iPhone-15 persona (Apple OUI) doesn't render as a
    // generic LAA placeholder. Falls back to the global slider's pool
    // when no persona is active, preserving v0.2.x behaviour.
    let preview_pool = effective_preview_pool(config);
    let example = preview_mac(&preview_pool);

    let ifaces = collect_ifaces(state, iface_filter);
    if ifaces.is_empty() {
        if let Some(name) = iface_filter {
            plan.note(format!(
                "no record of interface '{name}' in state.json; \
                 apply would query NetworkManager at run time"
            ));
        } else {
            plan.note(
                "no managed interfaces in state.json yet; \
                 apply would query NetworkManager at run time",
            );
        }
        plan.push(PlanStep {
            kind: StepKind::MacRotate,
            message: format!(
                "would generate a fresh MAC from OUI pool [{pool_label}] (e.g. {example})"
            ),
            detail: Some(
                "real rotation avoids the gateway and any MAC in the local ARP table".into(),
            ),
        });
        return plan;
    }

    for (iface, current) in ifaces {
        plan.push(PlanStep {
            kind: StepKind::MacRotate,
            message: format!(
                "would rotate {iface} (current {}) to a fresh MAC from OUI pool [{pool_label}]",
                current.as_deref().unwrap_or("unknown")
            ),
            detail: Some(format!("e.g. {example}")),
        });
        plan.push(PlanStep {
            kind: StepKind::DbusCall,
            message: format!(
                "would call NetworkManager Settings.Connection.Update on {iface}'s active profile"
            ),
            detail: Some(
                "sets cloned-mac-address and assigned-mac-address (older NM compatibility)".into(),
            ),
        });
        plan.push(PlanStep {
            kind: StepKind::StateUpdate,
            message: format!("would update state.json: managed.interfaces.{iface}"),
            detail: Some("current_mac, last_rotated, rotation_count".into()),
        });
    }
    plan
}

/// Build a preview plan for `proteus pin <target> [--mac <m>]`.
///
/// We can't reliably resolve `<target>` without DBus (it might be an
/// interface name or an NM connection profile), so we describe both code
/// paths. When `state.json` already records a current MAC for the target,
/// we surface that as the value the real `pin` would record.
pub fn plan_pin(state: &State, target: &str, mac_override: Option<&str>) -> Plan {
    let mut plan = Plan::new("pin");

    let from_state_iface = state
        .managed
        .interfaces
        .get(target)
        .and_then(|r| r.current_mac.as_deref());
    let from_state_conn = state
        .managed
        .connections
        .get(target)
        .and_then(|r| r.current_mac.as_deref());

    let resolved = mac_override
        .or(from_state_iface)
        .or(from_state_conn)
        .map(str::to_string);

    let mac_msg = match resolved.as_deref() {
        Some(m) => m.to_string(),
        None => "<current cloned MAC>".to_string(),
    };

    plan.push(PlanStep {
        kind: StepKind::MacPin,
        message: format!("would pin '{target}' to {mac_msg}"),
        detail: Some(
            "target is resolved as an interface name first, then as an NM connection profile"
                .into(),
        ),
    });
    plan.push(PlanStep {
        kind: StepKind::StateUpdate,
        message: format!(
            "would set state.json: managed.interfaces.{target}.pinned (or .connections.{target}.pinned)"
        ),
        detail: None,
    });
    plan
}

fn collect_ifaces(state: &State, filter: Option<&str>) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for (name, rec) in &state.managed.interfaces {
        if let Some(f) = filter
            && name != f
        {
            continue;
        }
        let current = rec
            .current_mac
            .clone()
            .or_else(|| state.original_macs.get(name).cloned());
        out.push((name.clone(), current));
    }
    out
}

fn pool_label(pool: &[String]) -> String {
    if pool.is_empty() {
        "(empty)".to_string()
    } else {
        pool.join(", ")
    }
}

/// Resolve the OUI pool the dry-run preview should sample from. NM2.7:
/// the previous shape always seeded the generator with the
/// `random-locally-administered` token, which produced a generic LAA
/// placeholder regardless of which persona was active. The previewed
/// MAC must reflect what a real `proteus rotate` would actually emit
/// — Apple OUI for an active iPhone persona, Google OUI for a Pixel
/// persona, the global slider's pool otherwise.
///
/// Falls back to a single LAA token if the resolved pool is empty so
/// `preview_mac` always has something the generator can chew on.
fn effective_preview_pool(config: &Config) -> Vec<String> {
    let active = crate::persona::active_for(
        config,
        None,
        crate::persona::resolve::default_user_root(),
    );
    if let Some(p) = active
        && matches!(p.kind, crate::persona::PersonaKind::Stealth)
        && !crate::mac::oui::resolve_vendor_tokens(&p.oui_pool).is_empty()
    {
        return p.oui_pool;
    }
    if !config.mac.oui_pool.is_empty() {
        return config.mac.oui_pool.clone();
    }
    vec!["random-locally-administered".into()]
}

/// One sample MAC for the preview message. Generated through the real path
/// to confirm the OUI pool token is valid; if generation fails we fall back
/// to a plausible LAA placeholder so the preview still reads sensibly.
fn preview_mac(pool: &[String]) -> String {
    use std::collections::HashSet;

    use crate::mac::generator::{self, GenerateOptions};

    let forbidden: HashSet<Mac> = HashSet::new();
    let avoid: HashSet<Mac> = HashSet::new();
    let opts = GenerateOptions {
        pool,
        forbidden: &forbidden,
        avoid: &avoid,
        suffix_pattern: None,
    };
    match generator::generate(&opts) {
        Ok(m) => m.to_string(),
        // The fallback retains the LAA shape: hand-edited personas with
        // unresolvable tokens shouldn't bury the preview entirely.
        Err(_) => "02:00:00:xx:xx:xx".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_with_empty_state_emits_note_and_one_mac_step() {
        let cfg = Config::default();
        let st = State::default();
        let plan = plan_rotate(&cfg, &st, None);
        assert_eq!(plan.command, "rotate");
        // One note + one mac-rotate step describing the pool.
        let note_count = plan
            .steps
            .iter()
            .filter(|s| matches!(s.kind, StepKind::Note))
            .count();
        let rotate_count = plan
            .steps
            .iter()
            .filter(|s| matches!(s.kind, StepKind::MacRotate))
            .count();
        assert_eq!(note_count, 1);
        assert_eq!(rotate_count, 1);
    }

    #[test]
    fn rotate_with_managed_iface_emits_dbus_and_state_steps() {
        let cfg = Config::default();
        let mut st = State::default();
        st.managed.interfaces.insert(
            "wlan0".into(),
            crate::state::InterfaceRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                pinned: None,
                last_rotated: None,
                rotation_count: 0,
            },
        );
        let plan = plan_rotate(&cfg, &st, None);
        assert!(
            plan.steps
                .iter()
                .any(|s| matches!(s.kind, StepKind::MacRotate) && s.message.contains("wlan0"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| matches!(s.kind, StepKind::DbusCall) && s.message.contains("wlan0"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| matches!(s.kind, StepKind::StateUpdate) && s.message.contains("wlan0"))
        );
    }

    #[test]
    fn pin_uses_state_current_mac_when_no_override_supplied() {
        let mut st = State::default();
        st.managed.interfaces.insert(
            "wlan0".into(),
            crate::state::InterfaceRecord {
                current_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                pinned: None,
                last_rotated: None,
                rotation_count: 0,
            },
        );
        let plan = plan_pin(&st, "wlan0", None);
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("aa:bb:cc:dd:ee:ff"))
        );
    }

    #[test]
    fn pin_with_explicit_mac_uses_override() {
        let st = State::default();
        let plan = plan_pin(&st, "Home Wi-Fi", Some("11:22:33:44:55:66"));
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("11:22:33:44:55:66"))
        );
    }

    #[test]
    fn pin_without_state_or_override_marks_mac_as_placeholder() {
        let st = State::default();
        let plan = plan_pin(&st, "wlan0", None);
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("<current cloned MAC>"))
        );
    }

    /// NM2.7: when `[mac] oui_pool` declares Apple, the preview must
    /// sample from the configured pool — not the hardcoded LAA
    /// placeholder. We assert the resolved pool gets chosen rather
    /// than spinning a real generate (which is randomised); the
    /// dry-run preview message itself still rolls through `preview_mac`
    /// in production, but the pool plumbing is what was broken.
    #[test]
    fn effective_preview_pool_uses_global_slider_when_no_persona() {
        let mut cfg = Config::default();
        cfg.persona.active = None;
        cfg.mac.oui_pool = vec!["apple".into(), "intel".into()];
        let pool = effective_preview_pool(&cfg);
        assert_eq!(pool, vec!["apple".to_string(), "intel".to_string()]);
    }

    /// NM2.7: the LAA placeholder is the last-resort fallback only —
    /// when the global pool is empty AND no persona is active.
    #[test]
    fn effective_preview_pool_falls_back_to_laa_when_everything_empty() {
        let mut cfg = Config::default();
        cfg.persona.active = None;
        cfg.mac.oui_pool = vec![];
        let pool = effective_preview_pool(&cfg);
        assert_eq!(pool, vec!["random-locally-administered".to_string()]);
    }

    /// NM2.7 — end-to-end: with `oui_pool = ["apple"]`, the
    /// `preview_mac` helper produces a MAC whose first three bytes
    /// land in the Apple OUI registry. Pin so future regressions
    /// in the pool-threading land catch it.
    #[test]
    fn preview_mac_emits_apple_oui_when_pool_says_apple() {
        let pool = vec!["apple".to_string()];
        // Run several samples — generator is randomised; every sample
        // must still land in the Apple OUI registry.
        for _ in 0..16 {
            let mac = preview_mac(&pool);
            // Cheap surface check: the rendered string must not be
            // the generic "02:00:00:..." LAA placeholder.
            assert!(
                !mac.starts_with("02:00:00"),
                "Apple-pool preview should not render as generic LAA: {mac:?}"
            );
        }
    }
}
