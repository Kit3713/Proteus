// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use anyhow::Result;

use crate::display::display_safe;
use crate::exit;
use crate::state::State;

/// Pin scopes recognised by `unpin --scope`. Kept here (not exposed) so
/// the CLI surface is "interface vs NM connection" and the operator
/// doesn't need to know the underlying state-map names.
const SCOPE_IFACE: &str = "iface";
const SCOPE_NM_CONNECTION: &str = "nm-connection";

/// Issue #391 / N12.1: `unpin` clears the persisted pin so a subsequent
/// rotation drops the operator-chosen MAC. That's a mutating change just
/// like `pin`, so we gate on `--yes` for parity with the rest of the
/// confirmation contract — wrapper scripts that depend on the gate were
/// silently no-ops before.
///
/// Issue #392: `--all` removes every pin; `--scope <type>` restricts the
/// bulk-clear to one scope (`iface` or `nm-connection`). Clap enforces
/// "exactly one of target/--all/--scope" at parse time, so by the time we
/// reach the handler the inputs are already validated.
pub fn run(
    target: Option<&str>,
    all: bool,
    scope: Option<&str>,
    yes: bool,
    state_path: Option<&Path>,
) -> Result<u8> {
    // Validate `--scope <type>` up front so a typo rejects with
    // CONFIG_ERROR before we touch root, the lock, or state.json.
    let scope_filter = match scope {
        Some(s) => match parse_scope(s) {
            Some(f) => Some(f),
            None => {
                eprintln!(
                    "proteus: invalid --scope '{}'; expected '{SCOPE_IFACE}' or '{SCOPE_NM_CONNECTION}'",
                    display_safe(s)
                );
                return Ok(exit::CONFIG_ERROR);
            }
        },
        None => None,
    };

    if let Err(code) = super::require_yes(
        yes,
        "'unpin' is mutating (clears the operator-chosen pin)",
        "proteus help pin",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if all || scope.is_some() {
        let (removed_ifaces, removed_conns) = clear_pins(&mut state, scope_filter);
        let total = removed_ifaces + removed_conns;
        if total == 0 {
            println!("no pins to remove");
        } else {
            state.save(&state_path)?;
            println!(
                "unpinned {total} pin(s): {removed_ifaces} interface(s), {removed_conns} connection(s)"
            );
        }
        return Ok(exit::SUCCESS);
    }

    // Single-target path (back-compat). Clap guarantees `target` is
    // `Some` here because we hit neither `--all` nor `--scope`.
    let Some(target) = target else {
        // Defensive: should be unreachable via clap. Keep a clear
        // error rather than panicking if the parse contract ever drifts.
        eprintln!("proteus: missing target (and neither --all nor --scope set)");
        return Ok(exit::CONFIG_ERROR);
    };

    let mut changed = false;
    if let Some(rec) = state.managed.interfaces.get_mut(target)
        && rec.pinned.is_some()
    {
        rec.pinned = None;
        changed = true;
    }
    if let Some(rec) = state.managed.connections.get_mut(target)
        && rec.pinned.is_some()
    {
        rec.pinned = None;
        changed = true;
    }

    if !changed {
        eprintln!("proteus: no pin found for '{}'", display_safe(target));
        return Ok(exit::GENERIC_ERROR);
    }

    state.save(&state_path)?;
    println!("unpinned {}", display_safe(target));
    Ok(exit::SUCCESS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeFilter {
    Iface,
    NmConnection,
}

fn parse_scope(s: &str) -> Option<ScopeFilter> {
    match s {
        SCOPE_IFACE => Some(ScopeFilter::Iface),
        SCOPE_NM_CONNECTION => Some(ScopeFilter::NmConnection),
        _ => None,
    }
}

/// Clear pins across both managed maps, honouring `scope`.
///
/// Returns `(interfaces_cleared, connections_cleared)`. Records with no
/// pin set are skipped so we don't churn the state file for a no-op
/// rewrite. `--all` (i.e. `scope = None`) clears both maps; otherwise
/// only the requested scope is touched.
fn clear_pins(state: &mut State, scope: Option<ScopeFilter>) -> (usize, usize) {
    let mut ifaces = 0;
    let mut conns = 0;
    if matches!(scope, None | Some(ScopeFilter::Iface)) {
        for rec in state.managed.interfaces.values_mut() {
            if rec.pinned.is_some() {
                rec.pinned = None;
                ifaces += 1;
            }
        }
    }
    if matches!(scope, None | Some(ScopeFilter::NmConnection)) {
        for rec in state.managed.connections.values_mut() {
            if rec.pinned.is_some() {
                rec.pinned = None;
                conns += 1;
            }
        }
    }
    (ifaces, conns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ConnectionRecord, InterfaceRecord, State};

    fn fresh_state_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("proteus-unpin-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_state(state_path: &Path) {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                pinned: Some("02:aa:bb:cc:dd:01".into()),
                ..Default::default()
            },
        );
        s.managed.interfaces.insert(
            "eth0".into(),
            InterfaceRecord {
                pinned: Some("02:aa:bb:cc:dd:02".into()),
                ..Default::default()
            },
        );
        s.managed.connections.insert(
            "home-wifi".into(),
            ConnectionRecord {
                pinned: Some("02:aa:bb:cc:dd:03".into()),
                ..Default::default()
            },
        );
        s.save(state_path).unwrap();
    }

    /// `--all --yes` clears every pin in both maps.
    #[test]
    fn all_yes_clears_every_pin() {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                pinned: Some("02:00:00:00:00:01".into()),
                ..Default::default()
            },
        );
        s.managed.connections.insert(
            "home".into(),
            ConnectionRecord {
                pinned: Some("02:00:00:00:00:02".into()),
                ..Default::default()
            },
        );

        let (ifaces, conns) = clear_pins(&mut s, None);
        assert_eq!(ifaces, 1);
        assert_eq!(conns, 1);
        assert!(s.managed.interfaces["wlan0"].pinned.is_none());
        assert!(s.managed.connections["home"].pinned.is_none());
    }

    /// `--scope nm-connection --yes` clears only NM-connection pins;
    /// interface pins are left intact.
    #[test]
    fn scope_nm_connection_only_clears_connections() {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                pinned: Some("02:00:00:00:00:01".into()),
                ..Default::default()
            },
        );
        s.managed.connections.insert(
            "home".into(),
            ConnectionRecord {
                pinned: Some("02:00:00:00:00:02".into()),
                ..Default::default()
            },
        );

        let (ifaces, conns) = clear_pins(&mut s, Some(ScopeFilter::NmConnection));
        assert_eq!(ifaces, 0);
        assert_eq!(conns, 1);
        assert!(
            s.managed.interfaces["wlan0"].pinned.is_some(),
            "interface pin must survive --scope nm-connection"
        );
        assert!(s.managed.connections["home"].pinned.is_none());
    }

    /// `--scope iface --yes` clears only interface pins.
    #[test]
    fn scope_iface_only_clears_interfaces() {
        let mut s = State::default();
        s.managed.interfaces.insert(
            "wlan0".into(),
            InterfaceRecord {
                pinned: Some("02:00:00:00:00:01".into()),
                ..Default::default()
            },
        );
        s.managed.connections.insert(
            "home".into(),
            ConnectionRecord {
                pinned: Some("02:00:00:00:00:02".into()),
                ..Default::default()
            },
        );

        let (ifaces, conns) = clear_pins(&mut s, Some(ScopeFilter::Iface));
        assert_eq!(ifaces, 1);
        assert_eq!(conns, 0);
        assert!(s.managed.interfaces["wlan0"].pinned.is_none());
        assert!(s.managed.connections["home"].pinned.is_some());
    }

    /// Records with no pin set are skipped — `clear_pins` does NOT
    /// claim it cleared a slot that was already empty.
    #[test]
    fn clear_pins_skips_records_with_no_pin() {
        let mut s = State::default();
        s.managed
            .interfaces
            .insert("wlan0".into(), InterfaceRecord::default());
        let (ifaces, conns) = clear_pins(&mut s, None);
        assert_eq!(ifaces, 0);
        assert_eq!(conns, 0);
    }

    /// `--all` without `--yes` returns CONFIRMATION_REQUIRED and never
    /// touches the state file.
    #[test]
    fn all_without_yes_returns_confirmation_required() {
        let dir = fresh_state_dir("all-noyes");
        let state_path = dir.join("state.json");
        seed_state(&state_path);
        let before = std::fs::read(&state_path).unwrap();

        let code = run(None, true, None, false, Some(&state_path)).unwrap();
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
        let after = std::fs::read(&state_path).unwrap();
        assert_eq!(before, after, "--all without --yes must not rewrite state");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--scope` without `--yes` is gated the same way.
    #[test]
    fn scope_without_yes_returns_confirmation_required() {
        let dir = fresh_state_dir("scope-noyes");
        let state_path = dir.join("state.json");
        seed_state(&state_path);
        let before = std::fs::read(&state_path).unwrap();

        let code = run(None, false, Some("iface"), false, Some(&state_path)).unwrap();
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
        let after = std::fs::read(&state_path).unwrap();
        assert_eq!(
            before, after,
            "--scope without --yes must not rewrite state"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unknown `--scope` value rejects fast with CONFIG_ERROR — the
    /// --yes gate must NOT run first (so a typo with --yes still rejects).
    #[test]
    fn invalid_scope_returns_config_error() {
        let dir = fresh_state_dir("bad-scope");
        let state_path = dir.join("state.json");

        let code = run(None, false, Some("bogus"), true, Some(&state_path)).unwrap();
        assert_eq!(code, exit::CONFIG_ERROR);
        assert!(
            !state_path.exists(),
            "bad --scope must not create state.json"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `parse_scope` accepts the two documented scopes and rejects
    /// everything else.
    #[test]
    fn parse_scope_round_trips_documented_values() {
        assert_eq!(parse_scope("iface"), Some(ScopeFilter::Iface));
        assert_eq!(
            parse_scope("nm-connection"),
            Some(ScopeFilter::NmConnection)
        );
        assert_eq!(parse_scope("interface"), None);
        assert_eq!(parse_scope(""), None);
    }

    // ---- clap parse contract for #392 -----------------------------
    //
    // The top-level `Cli` parser wires `Command::Unpin` so that exactly
    // one of `<target>`, `--all`, or `--scope <type>` is required. We
    // pin that contract here so a future clap drift surfaces in tests
    // rather than in production wrappers.

    use crate::cli::{Cli, Command};
    use clap::Parser;

    #[test]
    fn cli_unpin_with_target_parses() {
        let cli = Cli::try_parse_from(["proteus", "unpin", "wlan0", "--yes"]).expect("parse");
        match cli.command {
            Command::Unpin {
                target,
                all,
                scope,
                yes,
            } => {
                assert_eq!(target.as_deref(), Some("wlan0"));
                assert!(!all);
                assert!(scope.is_none());
                assert!(yes);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cli_unpin_all_parses_without_target() {
        let cli = Cli::try_parse_from(["proteus", "unpin", "--all", "--yes"]).expect("parse");
        match cli.command {
            Command::Unpin {
                target, all, yes, ..
            } => {
                assert!(target.is_none());
                assert!(all);
                assert!(yes);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cli_unpin_scope_parses_without_target() {
        let cli =
            Cli::try_parse_from(["proteus", "unpin", "--scope", "iface", "--yes"]).expect("parse");
        match cli.command {
            Command::Unpin {
                target, scope, yes, ..
            } => {
                assert!(target.is_none());
                assert_eq!(scope.as_deref(), Some("iface"));
                assert!(yes);
            }
            _ => panic!("wrong variant"),
        }
    }

    /// No flags and no target → clap rejects the parse. The roadmap
    /// explicitly calls this out as a test case for #392.
    #[test]
    fn cli_unpin_no_flags_and_no_target_rejects() {
        let r = Cli::try_parse_from(["proteus", "unpin"]);
        assert!(
            r.is_err(),
            "unpin with no target / --all / --scope must fail to parse"
        );
    }

    /// `target` and `--all` together must be rejected by clap
    /// (mutual exclusion).
    #[test]
    fn cli_unpin_target_with_all_is_rejected() {
        let r = Cli::try_parse_from(["proteus", "unpin", "wlan0", "--all", "--yes"]);
        assert!(r.is_err(), "target + --all must conflict");
    }

    /// `target` and `--scope` together must be rejected by clap.
    #[test]
    fn cli_unpin_target_with_scope_is_rejected() {
        let r = Cli::try_parse_from(["proteus", "unpin", "wlan0", "--scope", "iface", "--yes"]);
        assert!(r.is_err(), "target + --scope must conflict");
    }

    /// `--all` and `--scope` together must be rejected by clap.
    #[test]
    fn cli_unpin_all_with_scope_is_rejected() {
        let r = Cli::try_parse_from(["proteus", "unpin", "--all", "--scope", "iface", "--yes"]);
        assert!(r.is_err(), "--all + --scope must conflict");
    }
}
