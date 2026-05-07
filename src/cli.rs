// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use crate::commands;
use crate::exit;
use crate::logging;

/// Default cap on `wiki search` result rows.
const WIKI_SEARCH_DEFAULT_LIMIT: usize = 10;

#[derive(Parser, Debug)]
#[command(
    name = "proteus",
    version,
    about = "Erase the network identifiers your Linux laptop hands out on every join.",
    long_about = None,
    propagate_version = true,
    disable_help_subcommand = true,
)]
pub struct Cli {
    /// Increase log verbosity (repeat for more: -v, -vv).
    #[arg(short = 'v', long = "verbose", global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Decrease log verbosity (repeat for less: -q, -qq).
    #[arg(short = 'q', long = "quiet", global = true, action = clap::ArgAction::Count)]
    pub quiet: u8,

    /// Override config path (default: /etc/proteus/config.toml).
    #[arg(long = "config", global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Override state path (default: /var/lib/proteus/state.json).
    #[arg(long = "state", global = true, value_name = "PATH")]
    pub state: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show overall system + per-feature status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List current MAC addresses (per interface).
    Current {
        #[arg(long)]
        json: bool,
        /// Limit to a single interface.
        #[arg(long)]
        iface: Option<String>,
    },
    /// Show the cached original MACs and hostname.
    Original {
        #[arg(long)]
        json: bool,
    },
    /// Print the active config file (or note that defaults are in use).
    ShowConfig {
        #[arg(long)]
        json: bool,
    },
    /// Print the built-in default config.
    ShowDefaults {
        #[arg(long)]
        json: bool,
    },
    /// Apply Proteus config to the system.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Revert Proteus changes to the cached originals.
    Revert {
        #[arg(long)]
        yes: bool,
    },
    /// Rotate MAC for one or all managed interfaces.
    Rotate {
        #[arg(long)]
        iface: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Pin an interface or NM connection to a specific MAC.
    Pin {
        /// Interface name or NM connection profile.
        target: String,
        /// Specific MAC to pin (defaults to current cloned MAC).
        #[arg(long)]
        mac: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Remove a pin previously set with `pin`.
    Unpin {
        /// Interface name or NM connection profile.
        target: String,
    },
    /// Show diff between config, defaults, and live state.
    Diff {
        #[arg(long)]
        json: bool,
    },
    /// Preview what a mutating command would do.
    DryRun {
        /// The command (and args) to preview.
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Reset config to built-in defaults (sacred originals untouched).
    Reset {
        #[arg(long)]
        yes: bool,
        /// Print what would happen without writing.
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    /// Remove Proteus from the system.
    Uninstall {
        /// Also remove /etc/proteus and /var/lib/proteus.
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Bluetooth alias / discoverable / BLE RPA management.
    Bluetooth {
        #[command(subcommand)]
        action: BluetoothAction,
    },
    /// Hostname (kernel/pretty/transient) management via systemd hostnamed.
    Hostname {
        #[command(subcommand)]
        action: HostnameAction,
    },
    /// IPv6 stable-privacy + temp addresses + DUID rotation.
    Ipv6 {
        #[command(subcommand)]
        action: Ipv6Action,
    },
    /// 802.1X enterprise Wi-Fi anonymous outer identity (opt-in).
    EnterpriseWifi {
        #[command(subcommand)]
        action: EnterpriseWifiAction,
    },
    /// Stack-fingerprint sysctl drop-in (TCP/ICMP/NDP hardening).
    Stack {
        #[command(subcommand)]
        action: StackAction,
    },
    /// DNS ECS-strip drop-in on systemd-resolved (one knob, hard guard).
    Dns {
        #[command(subcommand)]
        action: DnsAction,
    },
    /// Browse the embedded wiki (or search it with `wiki search <query>`).
    #[command(args_conflicts_with_subcommands = true)]
    Wiki {
        #[command(subcommand)]
        action: Option<WikiAction>,
        /// Page name (e.g. `intro`); omit to list pages.
        page: Option<String>,
    },
    /// Show help for a feature (alias for `wiki <feature>` with friendly fallback).
    Help {
        /// Feature or wiki page name.
        feature: Option<String>,
    },
    /// Manage Proteus systemd timers (status, enable, set cadence, etc.).
    Timer {
        #[command(subcommand)]
        action: TimerAction,
    },
    /// Manage Proteus configuration without hand-editing config.toml.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Run a battery of self-diagnostic checks (read-only).
    Doctor {
        /// Emit JSON instead of human-readable output.
        #[arg(long)]
        json: bool,
        /// Skip the slower checks (DBus probes, filesystem walks).
        #[arg(long)]
        quick: bool,
    },
    /// Run a manual probe round against the configured endpoints.
    Probe {
        /// Machine-readable JSON output.
        #[arg(long)]
        json: bool,
        /// Single endpoint, fast.
        #[arg(long)]
        quick: bool,
    },
    /// Emergency network kill switch — bring all interfaces down + radios off.
    ///
    /// `proteus kill --yes` cuts every managed interface and disables Wi-Fi,
    /// WWAN, and Bluetooth radios. `proteus kill status` reports the current
    /// state. Use `proteus resume --yes` to restore. See `proteus wiki kill-switch`.
    Kill {
        #[command(subcommand)]
        action: Option<KillAction>,
        /// Confirm the destructive action when omitting the subcommand.
        #[arg(long, global = true)]
        yes: bool,
    },
    /// Restore network connectivity after `proteus kill`.
    Resume {
        #[arg(long)]
        yes: bool,
    },
    /// Manage the Proteus nftables table (ICMP info-drops + optional discovery blocks).
    Nft {
        #[command(subcommand)]
        action: NftAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum KillAction {
    /// Show whether the kill switch is currently active.
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum NftAction {
    /// Show whether our nft table is installed plus the rendered ruleset.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Install or refresh the Proteus nft table (idempotent).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the Proteus nft table.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum TimerAction {
    /// Show all proteus-* timers, their state, and current cadence.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List the timer types Proteus defines.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Enable + start a timer (e.g. `rotate`, `check`).
    Enable(TimerNameArgs),
    /// Disable + stop a timer.
    Disable(TimerNameArgs),
    /// Change a timer's cadence (writes a drop-in).
    Set {
        /// Timer name (`rotate`, `check`, ...).
        name: String,
        /// Cadence: `30s`, `5m`, `2h`, `1d`, or `hourly` / `daily`.
        #[arg(long)]
        interval: String,
    },
    /// Reset a timer's cadence back to its default (removes the drop-in).
    Reset(TimerNameArgs),
    /// Tail recent journald logs for a timer's unit.
    Logs {
        /// Timer name (`rotate`, `check`, ...).
        name: String,
        /// How many lines to tail.
        #[arg(long, default_value_t = 50)]
        lines: u32,
    },
}

#[derive(Args, Debug)]
pub struct TimerNameArgs {
    /// Timer name (`rotate`, `check`, `resume`, `boot`).
    pub name: String,
}

#[derive(Subcommand, Debug)]
pub enum HostnameAction {
    /// Show current kernel/pretty/transient + Proteus mode + cached originals.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Pick a new hostname per `[hostname] mode` and apply it.
    Rotate {
        #[arg(long)]
        yes: bool,
    },
    /// Pin to a specific hostname (validated against RFC 1123).
    Pin {
        /// Hostname to apply. Must be lowercase [a-z0-9-], no leading/trailing hyphen.
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Restore the cached original hostname.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum Ipv6Action {
    /// Show current per-iface IPv6 settings + privacy mode.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply stable-privacy + temp + DUID per config.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore the cached pre-Proteus IPv6 sysctl values.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum EnterpriseWifiAction {
    /// Show 802-1x.anonymous-identity for every 802.1X connection NM knows.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Set 802-1x.anonymous-identity = anonymous@<realm> on a connection.
    Enable {
        /// NM connection profile id (the human-friendly name).
        #[arg(long)]
        connection: String,
        #[arg(long)]
        yes: bool,
    },
    /// Clear 802-1x.anonymous-identity on a connection.
    Disable {
        /// NM connection profile id (the human-friendly name).
        #[arg(long)]
        connection: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum WikiAction {
    /// Full-text search across the embedded wiki.
    Search {
        /// One or more query terms (space-separated; case-insensitive).
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        json: bool,
        /// Cap on result rows shown (default 10).
        #[arg(long, default_value_t = WIKI_SEARCH_DEFAULT_LIMIT)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum BluetoothAction {
    /// List adapters with current alias, discoverable state, and RPA status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply alias / discoverable / BLE RPA policy to all adapters.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore original adapter aliases from cache.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the active config (alias for `proteus show-config`).
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Print a single config value (e.g. `mac.enabled`).
    Get {
        /// Dotted key, e.g. `mac.rotation_interval`.
        key: String,
        #[arg(long)]
        json: bool,
    },
    /// Set a single config value. Requires root + --yes.
    Set {
        /// Dotted key, e.g. `mac.rotation_interval`.
        key: String,
        /// New value (string, integer, bool — coerced to the existing type).
        value: String,
        #[arg(long)]
        yes: bool,
    },
    /// Enable a component (shorthand for `set <component>.enabled true`).
    Enable {
        /// Section name, e.g. `mac`, `hostname`.
        component: String,
        #[arg(long)]
        yes: bool,
    },
    /// Disable a component, optionally recording a reason as a comment.
    Disable {
        /// Section name, e.g. `dns`.
        component: String,
        /// Free-form reason; written above the section as a `# Proteus: disabled` comment.
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Open $EDITOR on /etc/proteus/config.toml; validate on save.
    Edit,
    /// Parse the current config; report errors with file context.
    Validate {
        #[arg(long)]
        json: bool,
    },
    /// Reset a section (or the whole file) to built-in defaults. Requires --yes.
    Reset {
        /// Optional section to reset; omit to reset everything.
        section: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// List every supported config key with its type and default.
    Keys {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum StackAction {
    /// Show current sysctl values + the drop-in we'd apply.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Write the drop-in to /etc/sysctl.d/95-proteus.conf and reload.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the drop-in and reload defaults.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DnsAction {
    /// Show what is applied or what we deferred to and why.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the ECS-strip drop-in (no-op if hard guard trips).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the ECS-strip drop-in.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    logging::init(cli.verbose, cli.quiet, cli.no_color);

    let code = match cli.command {
        Command::Status { json } => {
            commands::status::run(json, cli.state.as_deref(), cli.config.as_deref())
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
        Command::Rotate { iface, yes } => commands::rotate::run(
            iface.as_deref(),
            yes,
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
    };

    ExitCode::from(code.unwrap_or(exit::GENERIC_ERROR))
}

fn dispatch_config(action: ConfigAction, config: Option<&std::path::Path>) -> anyhow::Result<u8> {
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
    }
}
