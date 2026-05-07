// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::commands;
use crate::exit;
use crate::logging;

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
    /// Reset config to defaults and re-apply.
    Reset {
        #[arg(long)]
        yes: bool,
    },
    /// Remove Proteus from the system.
    Uninstall {
        /// Also remove /etc/proteus and /var/lib/proteus.
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Browse the embedded wiki.
    Wiki {
        /// Page name (e.g. `intro`); omit to list pages.
        page: Option<String>,
    },
    /// Show help for a feature (alias for `wiki <feature>` with friendly fallback).
    Help {
        /// Feature or wiki page name.
        feature: Option<String>,
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
        Command::Apply { .. } => {
            commands::stub::not_implemented("apply", 'B', "proteus wiki concepts")
        }
        Command::Revert { .. } => commands::stub::not_implemented(
            "revert",
            'G',
            "revert is critical and lands in phase G",
        ),
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
        Command::Diff { .. } => {
            commands::stub::not_implemented("diff", 'G', "proteus wiki concepts")
        }
        Command::DryRun { .. } => {
            commands::stub::not_implemented("dry-run", 'G', "proteus wiki concepts")
        }
        Command::Reset { .. } => {
            commands::stub::not_implemented("reset", 'G', "proteus wiki concepts")
        }
        Command::Uninstall { .. } => {
            commands::stub::not_implemented("uninstall", 'G', "proteus wiki uninstall")
        }
        Command::Wiki { page } => commands::wiki_cmd::run(page.as_deref()),
        Command::Help { feature } => commands::wiki_cmd::run_help(feature.as_deref()),
    };

    ExitCode::from(code.unwrap_or(exit::GENERIC_ERROR))
}
