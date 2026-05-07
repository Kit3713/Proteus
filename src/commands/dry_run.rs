// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus dry-run <command> [args...]` — preview a mutator's effects.
//!
//! The dry-run command parses its trailing args as if they were a top-level
//! subcommand, then dispatches to a per-module preview function that returns
//! a [`Plan`]. The plan is rendered as text or JSON. Nothing mutates: no DBus
//! calls, no file writes, no `state.json` updates.
//!
//! Phase G covers the "easy" mutators that already exist in the binary:
//! - `rotate` — preview a MAC rotation per managed interface
//! - `pin <target>` — preview pinning the named target to its current MAC
//! - `apply` — concatenated preview of every enabled component
//! - `revert` — preview the originals each module would restore
//! - `reset` — preview the config backup + defaults rewrite
//! - `uninstall [--purge]` — preview the units/files removal
//!
//! Modules that haven't landed yet (DHCP/DNS/IPv6/stack/nft) return a
//! `not yet implemented` note rather than a hard error so wrappers don't
//! have to special-case them.
//!
//! Unknown inner commands exit `64` (NOT_IMPLEMENTED) with a one-line
//! pointer at `proteus help`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;

use crate::config::Config;
use crate::dry_run::{Plan, PlanStep, StepKind, render};
use crate::exit;
use crate::state::State;

/// Inner arg parser. `dry-run`'s clap definition swallows the inner command
/// as a free-form `Vec<String>` so the dry-run module can re-parse it
/// independently. That keeps the global flags out of the inner shape and
/// lets us route to per-module previewers without coupling clap structures.
#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true)]
struct InnerArgs {
    /// Emit machine-readable JSON instead of human text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Inner,
}

#[derive(clap::Subcommand, Debug)]
enum Inner {
    /// Rotate MAC for one or all managed interfaces.
    Rotate {
        #[arg(long)]
        iface: Option<String>,
    },
    /// Apply Proteus config to the system.
    Apply,
    /// Revert Proteus changes to the cached originals.
    Revert,
    /// Reset config to built-in defaults.
    Reset,
    /// Remove Proteus from the system.
    Uninstall {
        #[arg(long)]
        purge: bool,
    },
    /// Pin an interface or NM connection to a specific MAC.
    Pin {
        target: String,
        #[arg(long)]
        mac: Option<String>,
    },
    /// Hostname rotate (preview only).
    Hostname,
    /// Bluetooth apply (preview only).
    Bluetooth,
}

/// Public entry point invoked from `cli::run`.
pub fn run(
    raw_args: &[String],
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if raw_args.is_empty() {
        eprintln!(
            "proteus dry-run: missing <command>. Try `proteus dry-run rotate` or see `proteus help`"
        );
        return Ok(exit::NOT_IMPLEMENTED);
    }

    // Map any clap parse error to the documented `64` exit so dry-run mirrors
    // the rest of the stub conventions.
    let parsed = match InnerArgs::try_parse_from(raw_args.iter().map(String::as_str)) {
        Ok(p) => p,
        Err(e) => {
            let head = raw_args.first().map(String::as_str).unwrap_or("?");
            eprintln!(
                "proteus dry-run: not implemented for '{head}': {}",
                e.kind()
            );
            eprintln!("see `proteus help` for the supported commands");
            return Ok(exit::NOT_IMPLEMENTED);
        }
    };

    let plan = build_plan(parsed.cmd, state_path, config_path);
    render(&plan, parsed.json)?;
    Ok(exit::SUCCESS)
}

fn build_plan(inner: Inner, state_path: Option<&Path>, config_path: Option<&Path>) -> Plan {
    let cfg_path = super::config_path(config_path);
    let st_path = super::state_path(state_path);
    let config = Config::default_or_loaded(&cfg_path).unwrap_or_default();
    let state = State::load_or_default(&st_path).unwrap_or_default();

    match inner {
        Inner::Rotate { iface } => crate::mac::plan_rotate(&config, &state, iface.as_deref()),
        Inner::Apply => plan_apply(&config, &state),
        Inner::Revert => plan_revert(&state),
        Inner::Reset => plan_reset(&cfg_path),
        Inner::Uninstall { purge } => plan_uninstall(purge),
        Inner::Pin { target, mac } => crate::mac::plan_pin(&state, &target, mac.as_deref()),
        Inner::Hostname => crate::hostname::plan_rotate(&config),
        Inner::Bluetooth => crate::bluetooth::plan_apply(&config),
    }
}

/// Apply: walk components in the same order as the real orchestrator and
/// concatenate each one's preview. Disabled components and modules that
/// haven't landed get a one-line note so the preview matches what apply
/// would emit at the summary line.
fn plan_apply(config: &Config, state: &State) -> Plan {
    let mut plan = Plan::new("apply");
    if config.mac.enabled {
        plan.extend(crate::mac::plan_rotate(config, state, None));
    } else {
        plan.note("mac: disabled in config (mac.enabled = false)");
    }
    if config.hostname.enabled {
        plan.extend(crate::hostname::plan_rotate(config));
    } else {
        plan.note("hostname: disabled in config (hostname.enabled = false)");
    }
    if config.bluetooth.enabled {
        plan.extend(crate::bluetooth::plan_apply(config));
    } else {
        plan.note("bluetooth: disabled in config (bluetooth.enabled = false)");
    }
    for name in ["dhcp", "dns", "stack", "nft"] {
        plan.note(format!("{name}: not yet implemented"));
    }
    plan
}

/// Revert: preview restoring originals for the modules that have apply paths
/// today (mac, hostname, bluetooth). Future modules light up automatically as
/// they land.
fn plan_revert(state: &State) -> Plan {
    let mut plan = Plan::new("revert");

    if state.original_macs.is_empty() {
        plan.note("mac: no original MACs cached, nothing to restore");
    } else {
        for (iface, mac) in &state.original_macs {
            plan.push(PlanStep {
                kind: StepKind::Restore,
                message: format!("would restore {iface} to {mac}"),
                detail: None,
            });
        }
    }

    match state.originals.hostname.as_ref() {
        Some(triple) => {
            plan.push(PlanStep {
                kind: StepKind::Restore,
                message: format!(
                    "would restore hostname (kernel={}, pretty={}, transient={})",
                    triple.kernel.as_deref().unwrap_or("(unset)"),
                    triple.pretty.as_deref().unwrap_or("(unset)"),
                    triple.transient.as_deref().unwrap_or("(unset)"),
                ),
                detail: None,
            });
        }
        None => {
            plan.note("hostname: no original cached, nothing to restore");
        }
    }

    if state.originals.bluetooth_aliases.is_empty() {
        plan.note("bluetooth: no original aliases cached, nothing to restore");
    } else {
        for (hci, alias) in &state.originals.bluetooth_aliases {
            plan.push(PlanStep {
                kind: StepKind::Restore,
                message: format!("would restore bluetooth adapter {hci} alias to '{alias}'"),
                detail: None,
            });
        }
    }

    plan.note("dhcp/dns/stack/nft: not yet implemented");
    plan
}

/// Reset: preview the config backup + defaults rewrite. Mirrors the real
/// `reset --dry-run` paths so the message text is consistent.
fn plan_reset(config_path: &Path) -> Plan {
    let mut plan = Plan::new("reset");
    let backup = super::reset::backup_path_for(config_path, "<timestamp>");

    if config_path.exists() {
        plan.push(PlanStep {
            kind: StepKind::FileWrite,
            message: format!(
                "would back up {} to {}",
                config_path.display(),
                backup.display()
            ),
            detail: None,
        });
    } else {
        plan.note(format!(
            "no existing config at {}; nothing to back up",
            config_path.display()
        ));
    }
    plan.push(PlanStep {
        kind: StepKind::FileWrite,
        message: format!("would write fresh defaults to {}", config_path.display()),
        detail: Some("state.json untouched (originals are sacred)".into()),
    });
    plan.note("would NOT call `proteus apply` automatically");
    plan
}

/// Uninstall: preview the units that would be torn down, the files that
/// would be removed, and (when `--purge`) the directories that would go.
/// The unit and drop-in lists come straight from `commands/uninstall` so
/// the preview can never drift from the real teardown.
fn plan_uninstall(purge: bool) -> Plan {
    let mut plan = Plan::new("uninstall");

    plan.note("would best-effort revert (mac, hostname, bluetooth)");

    for unit in super::uninstall::UNITS {
        plan.push(PlanStep {
            kind: StepKind::Command,
            message: format!("would run `systemctl disable --now {unit}`"),
            detail: None,
        });
    }
    plan.push(PlanStep {
        kind: StepKind::Command,
        message: "would run `systemctl daemon-reload`".into(),
        detail: None,
    });

    for path in super::uninstall::EXTERNAL_DROPINS {
        plan.push(PlanStep {
            kind: StepKind::FileRemove,
            message: format!("would remove {path}"),
            detail: None,
        });
    }
    plan.note(
        "would remove resolved drop-ins matching /etc/systemd/resolved.conf.d/10-proteus-*.conf",
    );

    plan.push(PlanStep {
        kind: StepKind::FileRemove,
        message: "would remove the proteus binary".into(),
        detail: Some("path resolved at runtime; falls back to /usr/local/bin/proteus".into()),
    });

    let config_dir = PathBuf::from("/etc/proteus");
    let state_dir = PathBuf::from("/var/lib/proteus");
    if purge {
        plan.push(PlanStep {
            kind: StepKind::FileRemove,
            message: format!("would remove {} (--purge)", config_dir.display()),
            detail: None,
        });
        plan.push(PlanStep {
            kind: StepKind::FileRemove,
            message: format!("would remove {} (--purge)", state_dir.display()),
            detail: None,
        });
    } else {
        plan.note(format!(
            "would keep {} and {}",
            config_dir.display(),
            state_dir.display()
        ));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> Config {
        let mut cfg = Config::default();
        cfg.mac.enabled = true;
        cfg.hostname.enabled = true;
        cfg.bluetooth.enabled = true;
        cfg
    }

    fn disabled_config() -> Config {
        let mut cfg = Config::default();
        cfg.mac.enabled = false;
        cfg.hostname.enabled = false;
        cfg.bluetooth.enabled = false;
        cfg
    }

    #[test]
    fn empty_args_returns_not_implemented() {
        let code = run(&[], None, None).unwrap();
        assert_eq!(code, exit::NOT_IMPLEMENTED);
    }

    #[test]
    fn unknown_inner_command_returns_not_implemented() {
        let args = vec!["doesnotexist".to_string()];
        let code = run(&args, None, None).unwrap();
        assert_eq!(code, exit::NOT_IMPLEMENTED);
    }

    #[test]
    fn apply_plan_concats_all_components_when_enabled() {
        let cfg = enabled_config();
        let plan = plan_apply(&cfg, &State::default());
        assert_eq!(plan.command, "apply");
        let any_hostname = plan
            .steps
            .iter()
            .any(|s| matches!(s.kind, StepKind::HostnameSet));
        assert!(
            any_hostname,
            "apply preview should include a hostname-set step"
        );
        let any_bluetooth = plan
            .steps
            .iter()
            .any(|s| matches!(s.kind, StepKind::BluetoothAdjust));
        assert!(
            any_bluetooth,
            "apply preview should include a bluetooth step"
        );
        for missing in ["dhcp", "dns", "stack", "nft"] {
            assert!(
                plan.steps
                    .iter()
                    .any(|s| s.message.contains(missing) && s.message.contains("not yet")),
                "expected a not-yet-implemented note for '{missing}'"
            );
        }
    }

    #[test]
    fn apply_plan_notes_each_disabled_component() {
        let cfg = disabled_config();
        let plan = plan_apply(&cfg, &State::default());
        for name in ["mac", "hostname", "bluetooth"] {
            assert!(
                plan.steps
                    .iter()
                    .any(|s| s.message.contains(name) && s.message.contains("disabled")),
                "expected a 'disabled' note for component '{name}'"
            );
        }
    }

    #[test]
    fn revert_plan_lists_cached_originals() {
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state
            .originals
            .bluetooth_aliases
            .insert("hci0".into(), "ThinkPad-Bluetooth".into());
        let plan = plan_revert(&state);
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("wlan0") && s.message.contains("aa:bb:cc:dd:ee:ff"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("hci0") && s.message.contains("ThinkPad-Bluetooth"))
        );
    }

    #[test]
    fn reset_plan_describes_backup_and_defaults() {
        let p = PathBuf::from("/etc/proteus/config.toml");
        let plan = plan_reset(&p);
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("fresh defaults"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("nothing to back up"))
        );
    }

    #[test]
    fn uninstall_plan_includes_unit_teardown_and_purge_branch() {
        let plan = plan_uninstall(true);
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("proteus-rotate.timer"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("--purge") && s.message.contains("/etc/proteus"))
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| s.message.contains("--purge") && s.message.contains("/var/lib/proteus"))
        );

        let plan_keep = plan_uninstall(false);
        assert!(
            plan_keep
                .steps
                .iter()
                .any(|s| s.message.contains("would keep"))
        );
    }
}
