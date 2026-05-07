// SPDX-License-Identifier: GPL-3.0-or-later

use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt};

pub fn init(verbose: u8, quiet: u8, no_color: bool) {
    let default_level = level_from_counts(verbose, quiet);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level.to_string().to_lowercase()));

    // JOURNAL_STREAM is set by systemd when the process is launched as a unit.
    // Prefer journald there so timer-driven runs are auto-correlated.
    if std::env::var_os("JOURNAL_STREAM").is_some()
        && let Ok(layer) = tracing_journald::layer()
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init();
        return;
    }

    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(!no_color)
        .without_time()
        .with_target(false)
        .try_init();
}

fn level_from_counts(verbose: u8, quiet: u8) -> Level {
    let v = i32::from(verbose) - i32::from(quiet);
    match v {
        i32::MIN..=-2 => Level::ERROR,
        -1 => Level::WARN,
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    }
}
