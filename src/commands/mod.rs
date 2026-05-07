// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod bluetooth_cmd;
pub mod config_cmd;
pub mod current;
pub mod doctor;
pub mod hostname;
pub mod original;
pub mod pin;
pub mod probe;
pub mod reset;
pub mod revert;
pub mod rotate;
pub mod show_config;
pub mod show_defaults;
pub mod status;
pub mod stub;
pub mod timer;
pub mod uninstall;
pub mod unpin;
pub mod wiki_cmd;

use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_STATE_PATH: &str = "/var/lib/proteus/state.json";
pub(crate) const DEFAULT_CONFIG_PATH: &str = "/etc/proteus/config.toml";

pub(crate) fn state_path(override_path: Option<&Path>) -> PathBuf {
    override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_PATH))
}

pub(crate) fn config_path(override_path: Option<&Path>) -> PathBuf {
    override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

/// Read the effective UID from procfs (Linux-only, avoids pulling in libc).
pub(crate) fn read_uid() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
}

pub(crate) fn require_root() -> anyhow::Result<()> {
    match read_uid() {
        Some(0) => Ok(()),
        Some(other) => anyhow::bail!(
            "this command must be run as root (current uid {other}); try `sudo proteus ...`"
        ),
        None => anyhow::bail!("could not determine effective uid from /proc/self/status"),
    }
}

pub(crate) fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Hand-rolled UTC ISO-8601 to keep zero deps.
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn unix_to_ymdhms(mut t: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = (t % 60) as u32;
    t /= 60;
    let mi = (t % 60) as u32;
    t /= 60;
    let h = (t % 24) as u32;
    t /= 24;
    let mut days = t as i64;
    // Howard Hinnant's civil_from_days algorithm (public domain).
    days += 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64 + era * 400) as u32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, h, mi, s)
}

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;

pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, value)?;
    println!();
    Ok(())
}

/// Atomic file write: temp file + sync + rename. Used by both state and
/// config writers so we share one durability story.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let tmp = path.with_extension("tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

pub(crate) fn render_config(cfg: &Config, json: bool) -> Result<()> {
    if json {
        print_json(cfg)
    } else {
        let rendered = toml::to_string_pretty(cfg).context("serializing config to TOML")?;
        print!("{rendered}");
        Ok(())
    }
}
