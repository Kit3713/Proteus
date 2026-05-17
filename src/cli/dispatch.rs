// SPDX-License-Identifier: GPL-3.0-or-later

//! Maps the parsed `Cli` into per-subcommand functions in `crate::commands::*`.
//!
//! Pure routing — no business logic. Each `commands::*::run` (or kindred
//! function) returns a `Result<u8>` exit code which `cli::run` lifts into
//! `ExitCode`.

use std::path::Path;

use anyhow::Result;

use super::Cli;
use super::actions::*;
use super::command::Command;
use crate::commands;

pub(super) fn dispatch(cli: Cli) -> Result<u8> {
    // Resolve the global --no-color view once so the watch loop's
    // screen-clear logic and the rest of the CLI honour the same flag.
    let no_color = cli.no_color;
    // Roadmap Milestone 6: the global --format flag overrides per-subcommand
    // `--json` flags. `--format yaml` errors out here for all readers
    // since the yaml renderer is reserved for a follow-up; passing
    // `--format json` flips every read command into json mode without the
    // user needing to spell `--json` per subcommand.
    let mut cli = cli;
    if let Some(global_fmt) = cli.format {
        match global_fmt {
            super::OutputFormat::Yaml => {
                eprintln!(
                    "proteus: --format yaml is reserved (no yaml dependency yet); \
                     use --format json or --format table"
                );
                return Ok(crate::exit::CONFIG_ERROR);
            }
            super::OutputFormat::Json => apply_json_to_command(&mut cli.command),
            super::OutputFormat::Table => {} // default — leave commands alone
        }
    }
    match cli.command {
        Command::Status {
            json,
            watch,
            interval,
        } => {
            let state_path = cli.state.clone();
            let config_path = cli.config.clone();
            if watch {
                // Issue #349 / CL1: surface a friendly diagnostic and a
                // CONFIG_ERROR (== 65) exit instead of bubbling the parse
                // error up to a generic-1 exit. Same shape for the two
                // siblings below.
                let delay = match commands::watch::parse_interval(&interval) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("proteus: invalid --interval: {e:#}");
                        return Ok(crate::exit::CONFIG_ERROR);
                    }
                };
                commands::watch::run(delay, no_color, move || {
                    commands::status::run(json, state_path.as_deref(), config_path.as_deref())
                })
            } else {
                commands::status::run(json, state_path.as_deref(), config_path.as_deref())
            }
        }
        Command::Session {
            json,
            watch,
            interval,
        } => {
            let state_path = cli.state.clone();
            let config_path = cli.config.clone();
            if watch {
                let delay = match commands::watch::parse_interval(&interval) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("proteus: invalid --interval: {e:#}");
                        return Ok(crate::exit::CONFIG_ERROR);
                    }
                };
                commands::watch::run(delay, no_color, move || {
                    commands::session::run(json, state_path.as_deref(), config_path.as_deref())
                })
            } else {
                commands::session::run(json, state_path.as_deref(), config_path.as_deref())
            }
        }
        Command::Current {
            json,
            iface,
            watch,
            interval,
        } => {
            let state_path = cli.state.clone();
            let iface_owned = iface.clone();
            if watch {
                let delay = match commands::watch::parse_interval(&interval) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("proteus: invalid --interval: {e:#}");
                        return Ok(crate::exit::CONFIG_ERROR);
                    }
                };
                commands::watch::run(delay, no_color, move || {
                    commands::current::run(json, iface_owned.as_deref(), state_path.as_deref())
                })
            } else {
                commands::current::run(json, iface.as_deref(), cli.state.as_deref())
            }
        }
        Command::Original { json } => commands::original::run(json, cli.state.as_deref()),
        Command::ShowConfig { json } => commands::show_config::run(json, cli.config.as_deref()),
        Command::ShowDefaults { json } => commands::show_defaults::run(json),
        Command::Apply { yes, json } => {
            commands::apply::run(yes, json, cli.state.as_deref(), cli.config.as_deref())
        }
        // Issue #386: thread the global `--state <path>` through revert
        // so the nested per-component reverts honour the operator's
        // override instead of all hardcoding the default state path.
        Command::Revert { yes, json } => commands::revert::run(yes, json, cli.state.as_deref()),
        Command::Rotate {
            iface,
            yes,
            explain,
            json,
        } => commands::rotate::run(
            iface.as_deref(),
            yes,
            explain,
            json,
            cli.state.as_deref(),
            cli.config.as_deref(),
        ),
        Command::RotateIfNeeded {
            iface,
            cooldown,
            ssid,
            yes,
            explain,
        } => commands::rotate::run_if_needed(
            iface.as_deref(),
            cooldown,
            ssid.as_deref(),
            yes,
            explain,
            cli.state.as_deref(),
            cli.config.as_deref(),
        ),
        Command::Pin { target, mac, yes } => {
            commands::pin::run(&target, mac.as_deref(), yes, cli.state.as_deref())
        }
        Command::Unpin {
            target,
            all,
            scope,
            yes,
        } => commands::unpin::run(
            target.as_deref(),
            all,
            scope.as_deref(),
            yes,
            cli.state.as_deref(),
        ),
        Command::Diff { json } => {
            commands::diff::run(json, cli.state.as_deref(), cli.config.as_deref())
        }
        Command::DryRun { command } => {
            commands::dry_run::run(&command, cli.state.as_deref(), cli.config.as_deref())
        }
        Command::Reset { yes, dry_run } => {
            commands::reset::run(yes, dry_run, cli.config.as_deref())
        }
        Command::Uninstall { purge, yes } => commands::uninstall::run(purge, yes),
        Command::Bluetooth { action } => match action {
            BluetoothAction::Status { json } => commands::bluetooth_cmd::status(json),
            BluetoothAction::Apply { yes } => {
                commands::bluetooth_cmd::apply(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            BluetoothAction::Revert { yes } => {
                commands::bluetooth_cmd::revert(yes, cli.state.as_deref())
            }
        },
        Command::Hostname { action } => match action {
            HostnameAction::Status { json } => {
                commands::hostname::status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Rotate { yes } => {
                commands::hostname::rotate(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Pin { name, yes } => {
                commands::hostname::pin(&name, yes, cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Revert { yes } => commands::hostname::revert(yes, cli.state.as_deref()),
        },
        Command::Ipv6 { action } => match action {
            Ipv6Action::Status { json } => {
                commands::ipv6::status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            Ipv6Action::Apply { yes } => {
                commands::ipv6::apply(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            Ipv6Action::Revert { yes } => commands::ipv6::revert(yes, cli.state.as_deref()),
        },
        Command::EnterpriseWifi { action } => match action {
            EnterpriseWifiAction::Status { json } => {
                commands::enterprise_wifi::status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            EnterpriseWifiAction::Enable { connection, yes } => commands::enterprise_wifi::enable(
                &connection,
                yes,
                cli.state.as_deref(),
                cli.config.as_deref(),
            ),
            EnterpriseWifiAction::Disable { connection, yes } => {
                commands::enterprise_wifi::disable(&connection, yes, cli.state.as_deref())
            }
        },
        Command::Stack { action } => match action {
            StackAction::Status { json } => {
                commands::stack::status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            StackAction::Apply { yes } => {
                commands::stack::apply(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            StackAction::Revert { yes } => commands::stack::revert(yes, cli.state.as_deref()),
        },
        // Roadmap #300: read-only state.json summary. Honors --state.
        Command::State { action } => match action {
            StateAction::Info { json } => commands::state_cmd::run_info(json, cli.state.as_deref()),
        },
        Command::Dns { action } => match action {
            DnsAction::Status { json } => commands::dns::status(json, cli.config.as_deref()),
            DnsAction::Apply { yes } => commands::dns::apply(yes, cli.config.as_deref()),
            DnsAction::Revert { yes } => commands::dns::revert(yes),
        },
        Command::Resolved { action } => match action {
            ResolvedAction::Status { json } => {
                commands::resolved::status(json, cli.config.as_deref())
            }
            ResolvedAction::Apply { yes } => commands::resolved::apply(yes, cli.config.as_deref()),
            ResolvedAction::Revert { yes } => commands::resolved::revert(yes),
        },
        Command::Ntp { action } => match action {
            NtpAction::Status { json } => commands::ntp::status(json, cli.config.as_deref()),
            NtpAction::Apply { yes } => commands::ntp::apply(yes, cli.config.as_deref()),
            NtpAction::Revert { yes } => commands::ntp::revert(yes),
        },
        Command::Dhcp { action } => match action {
            DhcpAction::Status { json } => commands::dhcp::status(json),
            // Issue #348/#375/M1/N12.2: --yes was previously dropped by the
            // `..` rest-pattern which let dhcp apply/revert mutate without
            // the operator's explicit confirmation. Plumb the flag through.
            DhcpAction::Apply { yes } => {
                commands::dhcp::apply(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            DhcpAction::Revert { yes } => commands::dhcp::revert(yes, cli.state.as_deref()),
            DhcpAction::Renew { iface, yes } => {
                commands::dhcp::renew(iface.as_deref(), yes, cli.state.as_deref())
            }
        },
        Command::Timer { action } => match action {
            TimerAction::Status { json } => commands::timer::run_status(json),
            TimerAction::List { json } => commands::timer::run_list(json),
            TimerAction::Enable(a) => commands::timer::run_enable(&a.name, a.yes),
            TimerAction::Disable(a) => commands::timer::run_disable(&a.name, a.yes),
            TimerAction::Set {
                name,
                interval,
                yes,
            } => commands::timer::run_set(&name, &interval, yes),
            TimerAction::Reset(a) => commands::timer::run_reset(&a.name, a.yes),
            TimerAction::Logs { name, lines } => commands::timer::run_logs(&name, lines),
        },
        Command::Logs {
            follow,
            lines,
            since,
            json,
        } => commands::logs::run(lines, follow, since.as_deref(), json),
        Command::Config { action } => dispatch_config(action, cli.config.as_deref()),
        Command::Portal { action } => match action {
            PortalAction::Status { json } => {
                commands::portal::run_status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            PortalAction::List { json } => commands::portal::run_list(json, cli.state.as_deref()),
            // Issue #348/N12.3: portal mark/unmark/open are mutators (mark
            // and unmark write state.json, open emits a network probe). The
            // `--yes` flag was being dropped via `..` — plumb it through.
            PortalAction::Mark { ssid, yes } => {
                commands::portal::run_mark(&ssid, yes, cli.state.as_deref())
            }
            PortalAction::Unmark { ssid, yes } => {
                commands::portal::run_unmark(&ssid, yes, cli.state.as_deref())
            }
            PortalAction::Open { yes } => {
                commands::portal::run_open(yes, cli.state.as_deref(), cli.config.as_deref())
            }
        },
        Command::Wiki { action, page, json } => match action {
            Some(WikiAction::Search { query, json, limit }) => {
                // N12.12: clap-bounded to 1..=500 (u64); fits `usize` trivially.
                commands::wiki_cmd::run_search(&query, json, limit as usize)
            }
            // Issue #406: programmatic enumeration of embedded pages.
            Some(WikiAction::List { json }) => commands::wiki_cmd::run_list(json),
            None => commands::wiki_cmd::run(page.as_deref(), json, cli.no_color),
        },
        Command::Help { feature } => commands::wiki_cmd::run_help(feature.as_deref(), cli.no_color),
        Command::Doctor { json, quick } => commands::doctor::run(commands::doctor::Options {
            json,
            quick,
            verbose: cli.verbose > 0,
            no_color: cli.no_color,
            state_path: cli.state.as_deref(),
            config_path: cli.config.as_deref(),
        }),
        Command::Probe { json, quick } => commands::probe::run(json, quick, cli.config.as_deref()),
        Command::Kill { action, yes } => match action {
            Some(KillAction::Status { json }) => {
                commands::kill::kill_status(json, cli.state.as_deref())
            }
            None => commands::kill::kill_run(yes, cli.state.as_deref()),
        },
        Command::Resume { yes, json } => {
            commands::kill::resume_run(yes, json, cli.state.as_deref())
        }
        Command::Nft { action } => match action {
            NftAction::Status { json } => commands::nft::status(json, cli.config.as_deref()),
            NftAction::Apply { yes } => commands::nft::apply(yes, cli.config.as_deref()),
            NftAction::Revert { yes } => commands::nft::revert(yes),
        },
        Command::Rf { action } => match action {
            RfAction::Status { json } => commands::rf::status(json, cli.config.as_deref()),
            RfAction::Apply { yes } => {
                commands::rf::apply(yes, cli.state.as_deref(), cli.config.as_deref())
            }
            RfAction::Revert { yes } => commands::rf::revert(yes, cli.state.as_deref()),
            // Roadmap Milestone 4b: scan-policy + chipset reports.
            RfAction::Scan { json } => commands::rf::scan(json),
            RfAction::Chipset { json } => commands::rf::chipset(json),
        },
        // Roadmap Milestone 2: persona schema + CLI. Apply/rotate
        // integration is the follow-up; this dispatch only flips
        // `[persona] active` and runs the read-side commands.
        Command::Persona { action } => {
            commands::persona::run(action, cli.state.as_deref(), cli.config.as_deref())
        }
        // Roadmap Milestone 3: per-SSID profile policies. Resolver +
        // CLI surface land here; the NM-connection-up dispatcher
        // integration is the follow-up.
        Command::Ssid { action } => commands::ssid::run(action, cli.config.as_deref()),
        // Roadmap Milestone 4c: event-driven rotation framework.
        Command::Events { action } => match action {
            EventsAction::Run {
                force,
                max_triggers,
                once_after_secs,
            } => commands::events::run(
                force,
                max_triggers,
                once_after_secs,
                cli.state.as_deref(),
                cli.config.as_deref(),
            ),
            EventsAction::ListSources { json } => commands::events::list_sources(json),
            EventsAction::Status { json } => commands::events::status(json, cli.state.as_deref()),
            EventsAction::Trigger { name, yes, debug } => {
                commands::events::trigger(&name, yes, debug)
            }
        },
        // Roadmap Milestone 6: print bundled shell completions.
        Command::Completions { shell } => commands::completions::run(&shell),
        // Issue #376: build-provenance reader for CI / GUI wrappers.
        // `about` is a friendly alias for the bare `version` form.
        Command::Version { json } => commands::version::run(json),
        Command::About => commands::version::run(false),
    }
}

/// Roadmap Milestone 6: when the user passes `--format json`, flip
/// the per-command `json` flag for every read command that has one.
/// Mutating commands and commands without a JSON form are left
/// untouched. Adding a new reader only requires extending this match.
fn apply_json_to_command(cmd: &mut Command) {
    // Collapsed-match form: every reader pattern is folded into the outer
    // match arm so clippy's `collapsible_match` lint stays clean. Issue #240:
    // probe / persona / ssid / wiki / config readers are part of the same
    // match. PersonaAction::Validate is intentionally omitted — the variant
    // has no `json` field (it's exit-code-only by design).
    match cmd {
        Command::Status { json, .. }
        | Command::Session { json, .. }
        | Command::Current { json, .. }
        | Command::Original { json }
        | Command::ShowConfig { json }
        | Command::ShowDefaults { json }
        | Command::Diff { json }
        | Command::Doctor { json, .. }
        | Command::Probe { json, .. }
        // Issue #395: rotate is mutating, but its post-mutation summary
        // is read-shaped — let the global `--format json` flip it.
        | Command::Rotate { json, .. }
        | Command::Bluetooth {
            action: BluetoothAction::Status { json },
        }
        | Command::Hostname {
            action: HostnameAction::Status { json },
        }
        | Command::Ipv6 {
            action: Ipv6Action::Status { json },
        }
        | Command::Dhcp {
            action: DhcpAction::Status { json },
        }
        | Command::Dns {
            action: DnsAction::Status { json },
        }
        | Command::Resolved {
            action: ResolvedAction::Status { json },
        }
        | Command::Ntp {
            action: NtpAction::Status { json },
        }
        | Command::Stack {
            action: StackAction::Status { json },
        }
        | Command::Nft {
            action: NftAction::Status { json },
        }
        | Command::EnterpriseWifi {
            action: EnterpriseWifiAction::Status { json },
        }
        | Command::Rf {
            action: RfAction::Status { json } | RfAction::Scan { json } | RfAction::Chipset { json },
        }
        | Command::Portal {
            action: PortalAction::Status { json } | PortalAction::List { json },
        }
        | Command::Timer {
            action: TimerAction::Status { json } | TimerAction::List { json },
        }
        | Command::Events {
            action: EventsAction::ListSources { json } | EventsAction::Status { json },
        }
        | Command::Persona {
            action:
                PersonaAction::List { json, .. }
                | PersonaAction::Show { json, .. }
                | PersonaAction::Current { json }
                | PersonaAction::Random { json, .. }
                | PersonaAction::Search { json, .. },
        }
        | Command::Ssid {
            action: SsidAction::List { json } | SsidAction::Show { json, .. },
        }
        | Command::State {
            action: StateAction::Info { json },
        }
        | Command::Wiki {
            action: Some(WikiAction::Search { json, .. } | WikiAction::List { json }),
            ..
        }
        // CL6: top-level `proteus wiki [page] --json` (no subcommand).
        | Command::Wiki {
            action: None,
            json,
            ..
        }
        | Command::Logs { json, .. }
        | Command::Resume { json, .. }
        | Command::Version { json }
        // Issue #343: apply/revert grew a JSON per-component summary; the
        // global `--format json` flag flips theirs too so wrappers stay
        // consistent with the readers above.
        | Command::Apply { json, .. }
        | Command::Revert { json, .. }
        | Command::Config {
            action:
                ConfigAction::Show { json }
                | ConfigAction::Get { json, .. }
                | ConfigAction::Validate { json }
                | ConfigAction::Keys { json },
        } => {
            *json = true;
        }
        // Subcommands without a `json` flag, or whose readers don't
        // benefit from JSON, are left untouched. Future readers can
        // join the match above.
        _ => {}
    }
}

fn dispatch_config(action: ConfigAction, config: Option<&Path>) -> Result<u8> {
    use commands::config_cmd as c;
    match action {
        ConfigAction::Show { json } => c::show(json, config),
        ConfigAction::Get { key, json } => c::get(&key, json, config),
        ConfigAction::Set { key, value, yes } => c::set(&key, &value, yes, config),
        ConfigAction::Enable { component, yes } => c::enable(&component, yes, config),
        ConfigAction::Disable {
            component,
            reason,
            yes,
        } => c::disable(&component, reason.as_deref(), yes, config),
        ConfigAction::Edit => c::edit(config),
        ConfigAction::Validate { json } => c::validate(json, config),
        ConfigAction::Reset { section, yes } => c::reset(section.as_deref(), yes, config),
        ConfigAction::Keys { json } => c::keys(json),
        ConfigAction::SetProfile { profile, yes } => c::set_profile(&profile, yes, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roadmap Milestone 6: `--format json` flips the per-subcommand
    /// `json` flag at dispatch time. Pin the contract for `Status`
    /// since it's the most-used reader.
    #[test]
    fn apply_json_to_status_command_sets_json_flag() {
        let mut cmd = Command::Status {
            json: false,
            watch: false,
            interval: "2s".into(),
        };
        apply_json_to_command(&mut cmd);
        match cmd {
            Command::Status { json, .. } => assert!(json),
            _ => unreachable!(),
        }
    }

    /// `Doctor` has a `quick` flag too — flipping `json` must not
    /// alter the others.
    #[test]
    fn apply_json_to_doctor_command_only_flips_json() {
        let mut cmd = Command::Doctor {
            json: false,
            quick: false,
        };
        apply_json_to_command(&mut cmd);
        match cmd {
            Command::Doctor { json, quick } => {
                assert!(json);
                assert!(!quick);
            }
            _ => unreachable!(),
        }
    }

    /// Subcommands without a `--json` flag must not panic when the
    /// helper is called against them. `Rotate` is a canonical
    /// no-JSON-flag mutator (it prints to stdout regardless).
    #[test]
    fn apply_json_to_mutating_command_without_json_is_a_noop() {
        let mut cmd = Command::Rotate {
            iface: None,
            yes: true,
            explain: false,
            json: false,
        };
        apply_json_to_command(&mut cmd);
        match cmd {
            Command::Rotate { yes, explain, .. } => {
                assert!(yes);
                assert!(!explain);
            }
            _ => unreachable!(),
        }
    }

    /// Issue #343: `--format json` flips the per-subcommand `json` flag
    /// for the new apply/revert summaries.
    #[test]
    fn apply_json_to_apply_command_sets_json_flag() {
        let mut cmd = Command::Apply {
            yes: true,
            json: false,
        };
        apply_json_to_command(&mut cmd);
        match cmd {
            Command::Apply { yes, json } => {
                assert!(yes);
                assert!(json);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn apply_json_to_revert_command_sets_json_flag() {
        let mut cmd = Command::Revert {
            yes: true,
            json: false,
        };
        apply_json_to_command(&mut cmd);
        match cmd {
            Command::Revert { yes, json } => {
                assert!(yes);
                assert!(json);
            }
            _ => unreachable!(),
        }
    }
}
