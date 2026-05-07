// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus dry-run` plan model.
//!
//! Each mutating command can produce a `Plan`: a structured, side-effect-free
//! description of what it would do. Plans are rendered to the user as either
//! human text or `--json`. Per-module preview functions live alongside the
//! module they describe (e.g. `mac::plan_rotate`); this file owns the shared
//! types and the dispatch glue.
//!
//! Phase G: rotate / pin / apply / revert / reset / uninstall are wired.
//! Modules that haven't landed yet (DHCP, DNS, IPv6, stack, nft) report
//! `not yet implemented` so a future enable lights up the dry-run automatically
//! when the matching apply path lands.
//!
//! No `Plan` step ever writes to disk, calls DBus, or shells out. The whole
//! module is read-only. That invariant is what `proteus dry-run` exists to
//! provide.
//!
//! See `proteus wiki concepts` and the phase-G section of `docs/PLAN.md`.

use serde::{Deserialize, Serialize};

/// Granular preview entry. The shape is intentionally narrow — a kind, a
/// human description, and an optional reason — so the JSON contract is
/// stable across phases as new mutators land.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    /// What family of effect this step describes. `serde` renders these as
    /// kebab-case strings so a wrapper can branch on them defensively.
    pub kind: StepKind,
    /// One human-readable line describing the effect. Stable enough for a
    /// user to read; not stable enough to scrape — branch on `kind` instead.
    pub message: String,
    /// Optional secondary line: a reason a step is skipped, the source path
    /// of a backup, etc. Omitted from JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Closed enum of effect kinds. New variants land alongside new modules; old
/// variants never change meaning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StepKind {
    /// Generate or assign a MAC.
    MacRotate,
    /// Pin a MAC to an interface or NM connection.
    MacPin,
    /// Set the system hostname (kernel/pretty/transient).
    HostnameSet,
    /// Restore an original cached value.
    Restore,
    /// Adjust a Bluetooth adapter property (alias / discoverable / RPA).
    BluetoothAdjust,
    /// Write a managed file under `/etc/`.
    FileWrite,
    /// Remove a managed file or drop-in.
    FileRemove,
    /// Update a field in `state.json`.
    StateUpdate,
    /// Make a DBus call. `message` names the destination + method.
    DbusCall,
    /// Run a systemd / nft / sysctl-style command.
    Command,
    /// A note: either "skipped: <reason>" or "not yet implemented".
    Note,
}

/// A complete plan for one inner command. Empty means there is nothing to do —
/// the dry-run printer surfaces that explicitly.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Plan {
    /// Inner command name, e.g. `rotate`, `apply`, `revert`.
    pub command: String,
    /// One step per planned effect, in execution order.
    pub steps: Vec<PlanStep>,
}

impl Plan {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            steps: Vec::new(),
        }
    }

    pub fn push(&mut self, step: PlanStep) -> &mut Self {
        self.steps.push(step);
        self
    }

    pub fn note(&mut self, message: impl Into<String>) -> &mut Self {
        self.push(PlanStep {
            kind: StepKind::Note,
            message: message.into(),
            detail: None,
        })
    }

    pub fn extend(&mut self, other: Plan) {
        self.steps.extend(other.steps);
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Render a plan to stdout in the requested format.
///
/// Human form is one line per step plus a trailing summary; JSON is the
/// `Plan` struct serialized verbatim. Both forms always end with a newline
/// so wrappers can split on `\n`.
pub fn render(plan: &Plan, json: bool) -> anyhow::Result<()> {
    if json {
        crate::commands::print_json(plan)?;
    } else {
        print_human(plan);
    }
    Ok(())
}

fn print_human(plan: &Plan) {
    println!("dry-run: {} (preview only — no changes made)", plan.command);
    if plan.steps.is_empty() {
        println!("  (nothing to do)");
        return;
    }
    for s in &plan.steps {
        let tag = step_tag(s.kind);
        println!("  [{tag}] {}", s.message);
        if let Some(d) = &s.detail {
            println!("        {d}");
        }
    }
}

fn step_tag(k: StepKind) -> &'static str {
    match k {
        StepKind::MacRotate => "mac",
        StepKind::MacPin => "pin",
        StepKind::HostnameSet => "hostname",
        StepKind::Restore => "restore",
        StepKind::BluetoothAdjust => "bluetooth",
        StepKind::FileWrite => "write",
        StepKind::FileRemove => "remove",
        StepKind::StateUpdate => "state",
        StepKind::DbusCall => "dbus",
        StepKind::Command => "cmd",
        StepKind::Note => "note",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_plan_round_trips_via_json() {
        let plan = Plan::new("rotate");
        let s = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&s).unwrap();
        assert_eq!(plan, back);
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_steps_serialize_with_kebab_case_kinds() {
        let mut plan = Plan::new("rotate");
        plan.push(PlanStep {
            kind: StepKind::MacRotate,
            message: "would assign aa:bb:cc:dd:ee:ff to wlan0".into(),
            detail: None,
        });
        plan.push(PlanStep {
            kind: StepKind::StateUpdate,
            message: "would update state.json".into(),
            detail: Some("managed.interfaces.wlan0".into()),
        });
        let s = serde_json::to_string(&plan).unwrap();
        assert!(s.contains("\"kind\":\"mac-rotate\""));
        assert!(s.contains("\"kind\":\"state-update\""));
        // detail field is only present on the second step.
        assert!(s.contains("\"detail\":\"managed.interfaces.wlan0\""));
    }

    #[test]
    fn extend_concatenates_step_lists() {
        let mut a = Plan::new("apply");
        a.note("a");
        let mut b = Plan::new("apply");
        b.note("b");
        a.extend(b);
        assert_eq!(a.steps.len(), 2);
        assert_eq!(a.steps[0].message, "a");
        assert_eq!(a.steps[1].message, "b");
    }
}
