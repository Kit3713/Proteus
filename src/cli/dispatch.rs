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
    match cli.command {
        Command::Status { json } => {
            commands::status::run(json, cli.state.as_deref(), cli.config.as_deref())
        }
        Command::Session { json } => {
            commands::session::run(json, cli.state.as_deref(), cli.config.as_deref())
        }
        Command::Current { json, iface } => {
            commands::current::run(json, iface.as_deref(), cli.state.as_deref())
        }
        Command::Original { json } => commands::original::run(json, cli.state.as_deref()),
        Command::ShowConfig { json } => commands::show_config::run(json, cli.config.as_deref()),
        Command::ShowDefaults { json } => commands::show_defaults::run(json),
        Command::Apply { yes } => {
            commands::apply::run(yes, cli.state.as_deref(), cli.config.as_deref())
        }
        Command::Revert { yes } => commands::revert::run(yes),
        Command::Rotate {
            iface,
            yes,
            explain,
        } => commands::rotate::run(
            iface.as_deref(),
            yes,
            explain,
            cli.state.as_deref(),
            cli.config.as_deref(),
        ),
        Command::Pin { target, mac, yes } => {
            commands::pin::run(&target, mac.as_deref(), yes, cli.state.as_deref())
        }
        Command::Unpin { target } => commands::unpin::run(&target, cli.state.as_deref()),
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
            BluetoothAction::Apply { .. } => {
                commands::bluetooth_cmd::apply(cli.state.as_deref(), cli.config.as_deref())
            }
            BluetoothAction::Revert { .. } => commands::bluetooth_cmd::revert(cli.state.as_deref()),
        },
        Command::Hostname { action } => match action {
            HostnameAction::Status { json } => {
                commands::hostname::status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Rotate { .. } => {
                commands::hostname::rotate(cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Pin { name, .. } => {
                commands::hostname::pin(&name, cli.state.as_deref(), cli.config.as_deref())
            }
            HostnameAction::Revert { .. } => commands::hostname::revert(cli.state.as_deref()),
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
        Command::Dns { action } => match action {
            DnsAction::Status { json } => commands::dns::status(json, cli.config.as_deref()),
            DnsAction::Apply { .. } => commands::dns::apply(cli.config.as_deref()),
            DnsAction::Revert { .. } => commands::dns::revert(),
        },
        Command::Resolved { action } => match action {
            ResolvedAction::Status { json } => {
                commands::resolved::status(json, cli.config.as_deref())
            }
            ResolvedAction::Apply { .. } => commands::resolved::apply(cli.config.as_deref()),
            ResolvedAction::Revert { .. } => commands::resolved::revert(),
        },
        Command::Ntp { action } => match action {
            NtpAction::Status { json } => commands::ntp::status(json, cli.config.as_deref()),
            NtpAction::Apply { .. } => commands::ntp::apply(cli.config.as_deref()),
            NtpAction::Revert { .. } => commands::ntp::revert(),
        },
        Command::Dhcp { action } => match action {
            DhcpAction::Status { json } => commands::dhcp::status(json),
            DhcpAction::Apply { .. } => {
                commands::dhcp::apply(cli.state.as_deref(), cli.config.as_deref())
            }
            DhcpAction::Revert { .. } => commands::dhcp::revert(cli.state.as_deref()),
            DhcpAction::Renew { iface, yes } => {
                commands::dhcp::renew(iface.as_deref(), yes, cli.state.as_deref())
            }
        },
        Command::Timer { action } => match action {
            TimerAction::Status { json } => commands::timer::run_status(json),
            TimerAction::List { json } => commands::timer::run_list(json),
            TimerAction::Enable(a) => commands::timer::run_enable(&a.name),
            TimerAction::Disable(a) => commands::timer::run_disable(&a.name),
            TimerAction::Set { name, interval } => commands::timer::run_set(&name, &interval),
            TimerAction::Reset(a) => commands::timer::run_reset(&a.name),
            TimerAction::Logs { name, lines } => commands::timer::run_logs(&name, lines),
        },
        Command::Config { action } => dispatch_config(action, cli.config.as_deref()),
        Command::Portal { action } => match action {
            PortalAction::Status { json } => {
                commands::portal::run_status(json, cli.state.as_deref(), cli.config.as_deref())
            }
            PortalAction::List { json } => commands::portal::run_list(json, cli.state.as_deref()),
            PortalAction::Mark { ssid, .. } => {
                commands::portal::run_mark(&ssid, cli.state.as_deref())
            }
            PortalAction::Unmark { ssid, .. } => {
                commands::portal::run_unmark(&ssid, cli.state.as_deref())
            }
            PortalAction::Open { .. } => {
                commands::portal::run_open(cli.state.as_deref(), cli.config.as_deref())
            }
        },
        Command::Wiki { action, page } => match action {
            Some(WikiAction::Search { query, json, limit }) => {
                commands::wiki_cmd::run_search(&query, json, limit)
            }
            None => commands::wiki_cmd::run(page.as_deref(), cli.no_color),
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
        Command::Resume { yes } => commands::kill::resume_run(yes, cli.state.as_deref()),
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
        Command::Persona { action } => commands::persona::run(action, cli.config.as_deref()),
        // Roadmap Milestone 3: per-SSID profile policies. Resolver +
        // CLI surface land here; the NM-connection-up dispatcher
        // integration is the follow-up.
        Command::Ssid { action } => commands::ssid::run(action, cli.config.as_deref()),
        // Roadmap Milestone 6: print bundled shell completions.
        Command::Completions { shell } => commands::completions::run(&shell),
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
