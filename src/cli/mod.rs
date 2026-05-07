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
    if let Err(code) = validate_config_override(cli.config.as_deref(), &cli.command) {
        return ExitCode::from(code);
    }
    let code = match dispatch::dispatch(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("proteus: {e:#}");
            exit::GENERIC_ERROR
        }
    };
    ExitCode::from(code)
}


/// When the user passes `--config <path>` explicitly we treat a missing file
/// as a hard error: silently falling back to defaults masks typos and is the
/// kind of trap that costs people half an hour. The implicit default path
/// (no `--config` flag) keeps the existing "fall back to defaults" behavior
/// because a fresh install hasn't written that file yet.
///
/// Commands whose job is to create or rewrite the config file (e.g.
/// `proteus reset`, `proteus config edit`) are exempt — those need to be
/// able to point at a path that does not yet exist.
fn validate_config_override(
    override_path: Option<&std::path::Path>,
    command: &Command,
) -> Result<(), u8> {
    let Some(p) = override_path else {
        return Ok(());
    };
    if command_writes_config(command) || p.exists() {
        return Ok(());
    }
    eprintln!("proteus: config not found at {}", p.display());
    Err(exit::CONFIG_ERROR)
}

/// True for subcommands that may legitimately create the config file from
/// scratch. The list is intentionally narrow: only commands that exist
/// specifically to write the config file qualify. Read commands and
/// commands that consume the config (apply, rotate, etc.) still fail when
/// `--config <path>` points at a missing file.
fn command_writes_config(cmd: &Command) -> bool {
    use Command::*;
    matches!(
        cmd,
        Reset { .. }
            | Config {
                action: ConfigAction::Edit
                    | ConfigAction::Reset { .. }
                    | ConfigAction::Set { .. }
                    | ConfigAction::SetProfile { .. }
                    | ConfigAction::Enable { .. }
                    | ConfigAction::Disable { .. },
            }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, anyhow};
    use std::path::Path;

    fn dummy_read_command() -> Command {
        Command::Status { json: false }
    }

    /// Mirrors the `eprintln!("proteus: {e:#}")` path in `run()`:
    /// `anyhow::Error` formatted with `:#` must include both the top-level
    /// message and the entire source chain. Issue #112.
    #[test]
    fn anyhow_alternate_format_walks_source_chain() {
        let err: anyhow::Error = Err::<(), _>(anyhow!("inner cause: file not found"))
            .context("middle context: failed to read config")
            .context("top: config load failed")
            .unwrap_err();

        let rendered = format!("{err:#}");

        assert!(rendered.contains("top: config load failed"));
        assert!(rendered.contains("middle context: failed to read config"));
        assert!(rendered.contains("inner cause: file not found"));
    }

    #[test]
    fn no_override_is_ok_even_when_default_path_missing() {
        // The implicit default path may not exist on a fresh install; the
        // pre-flight check must not fail in that case.
        assert!(validate_config_override(None, &dummy_read_command()).is_ok());
    }

    #[test]
    fn missing_explicit_override_returns_config_error_for_read_command() {
        let bogus = Path::new("/nonexistent/proteus/config-does-not-exist.toml");
        assert_eq!(
            validate_config_override(Some(bogus), &dummy_read_command()),
            Err(exit::CONFIG_ERROR)
        );
    }

    #[test]
    fn missing_explicit_override_is_ok_for_reset_which_will_create_the_file() {
        let bogus = Path::new("/nonexistent/proteus/about-to-be-created.toml");
        let cmd = Command::Reset {
            yes: true,
            dry_run: true,
        };
        assert!(validate_config_override(Some(bogus), &cmd).is_ok());
    }

    #[test]
    fn missing_explicit_override_is_ok_for_config_edit() {
        let bogus = Path::new("/nonexistent/proteus/about-to-be-created.toml");
        let cmd = Command::Config {
            action: ConfigAction::Edit,
        };
        assert!(validate_config_override(Some(bogus), &cmd).is_ok());
    }

    #[test]
    fn existing_explicit_override_passes() {
        // Write a real (empty) file so the existence check is exercised
        // without depending on system paths like /proc.
        let cfg = std::env::temp_dir().join(format!(
            "proteus-cli-validate-{}-{}.toml",
            std::process::id(),
            line!()
        ));
        std::fs::write(&cfg, "").unwrap();
        assert!(validate_config_override(Some(&cfg), &dummy_read_command()).is_ok());
        let _ = std::fs::remove_file(&cfg);
    }
}
