// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI surface — parsing + entry point.
//!
//! Split into:
//! - `command` — the top-level `Command` enum (one variant per subcommand).
//! - `actions` — per-subcommand action enums (e.g. `BluetoothAction`).
//! - `dispatch` — the big match that maps parsed args to `commands::*`.
//!
//! The split is mechanical: clap derive macros work the same regardless of
//! where types live, as long as they're imported into the module that
//! references them. Re-exports below let `dispatch::dispatch` reach every
//! action enum from a single import.

mod actions;
mod command;
mod dispatch;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::exit;
use crate::logging;

pub use actions::*;
pub use command::Command;

/// Default cap on `wiki search` result rows.
pub(crate) const WIKI_SEARCH_DEFAULT_LIMIT: usize = 10;

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

/// Entry point for the `proteus` binary.
///
/// # Error contract
///
/// Subcommand handlers return `Result<u8>`:
///
/// - `Ok(code)` — the command produced an exit code (zero or non-zero).
///   Commands that intend to surface a user-facing diagnostic do so themselves
///   (typically via `eprintln!`) and return `Ok(non_zero_code)`. We pass that
///   code through unchanged.
/// - `Err(e)` — an unexpected failure bubbled up from a fallible operation
///   (config parse error, DBus unavailable, IO error, …). The dispatcher itself
///   has no diagnostic context to add, so we render the full `anyhow` source
///   chain to stderr here and exit with [`exit::GENERIC_ERROR`].
///
/// The `:#` formatting on `anyhow::Error` walks the source chain so the user
/// sees both the top-level message and the underlying cause(s).
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    logging::init(cli.verbose, cli.quiet, cli.no_color);
    let code = match dispatch::dispatch(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("proteus: {e:#}");
            exit::GENERIC_ERROR
        }
    };
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, anyhow};

    /// Mirrors the `eprintln!("proteus: {e:#}")` path in `run()`:
    /// `anyhow::Error` formatted with `:#` must include both the top-level
    /// message and the entire source chain. If this ever stops being true the
    /// CLI would silently drop diagnostic context (issue #112).
    #[test]
    fn anyhow_alternate_format_walks_source_chain() {
        let err: anyhow::Error = Err::<(), _>(anyhow!("inner cause: file not found"))
            .context("middle context: failed to read config")
            .context("top: config load failed")
            .unwrap_err();

        let rendered = format!("{err:#}");

        assert!(
            rendered.contains("top: config load failed"),
            "rendered error missing top context: {rendered}"
        );
        assert!(
            rendered.contains("middle context: failed to read config"),
            "rendered error missing middle context: {rendered}"
        );
        assert!(
            rendered.contains("inner cause: file not found"),
            "rendered error missing root cause: {rendered}"
        );
    }
}
