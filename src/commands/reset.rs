// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::exit;

/// Reset config to built-in defaults.
///
/// Mutating; requires root and `--yes`. The cached original MACs and hostname
/// in `state.json` are sacred and untouched. `apply` is not invoked
/// automatically — the user decides when to re-apply.
pub fn run(yes: bool, dry_run: bool, config_override: Option<&Path>) -> Result<u8> {
    if !yes && !dry_run {
        eprintln!(
            "proteus: reset is destructive; pass --yes to confirm or --dry-run to preview. \
             See: proteus wiki concepts"
        );
        return Ok(exit::NOT_IMPLEMENTED);
    }

    if !dry_run {
        if let Err(e) = super::require_root() {
            eprintln!("proteus: {e}");
            return Ok(exit::PERMISSION_ERROR);
        }
    }

    let path = super::config_path(config_override);
    let backup_path = backup_path_for(&path, &super::now_iso8601());

    if dry_run {
        let exists = path.exists();
        println!("proteus reset --dry-run:");
        if exists {
            println!(
                "  would back up {} to {}",
                path.display(),
                backup_path.display()
            );
        } else {
            println!(
                "  no existing config at {}; nothing to back up",
                path.display()
            );
        }
        println!(
            "  would write fresh defaults to {} (state.json untouched)",
            path.display()
        );
        println!("  would NOT call `proteus apply` — re-apply manually when ready");
        return Ok(exit::SUCCESS);
    }

    let defaults = toml::to_string_pretty(&Config::default()).context("serializing defaults")?;
    let backed_up = backup_existing(&path, &backup_path)?;
    write_atomic(&path, defaults.as_bytes())?;

    if backed_up {
        println!(
            "config reset; previous config saved to {}",
            backup_path.display()
        );
    } else {
        println!(
            "config reset; no previous config at {} (wrote fresh defaults)",
            path.display()
        );
    }
    Ok(exit::SUCCESS)
}

/// Copy `path` to `backup_path` if it exists. Returns whether a backup was made.
/// Avoids a TOCTOU pre-check by reacting to NotFound from `fs::copy`.
fn backup_existing(path: &Path, backup_path: &Path) -> Result<bool> {
    match fs::copy(path, backup_path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e)
            .with_context(|| format!("backing up {} to {}", path.display(), backup_path.display())),
    }
}

/// `/etc/proteus/config.toml` -> `/etc/proteus/config.toml.bak.<timestamp>`.
///
/// The timestamp is the ISO-8601 form (with `:` replaced by `-` to keep the
/// filename portable across shells and filesystems). `pub(crate)` so the
/// dry-run preview names the same path the real reset would produce.
pub(crate) fn backup_path_for(config: &Path, timestamp: &str) -> PathBuf {
    let safe = timestamp.replace(':', "-");
    let mut s = config.as_os_str().to_os_string();
    s.push(format!(".bak.{safe}"));
    PathBuf::from(s)
}

/// Atomic write: temp file in the same directory, then rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("config path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_path_appends_bak_with_safe_timestamp() {
        let path = Path::new("/etc/proteus/config.toml");
        let backup = backup_path_for(path, "2026-05-06T12:34:56Z");
        assert_eq!(
            backup,
            PathBuf::from("/etc/proteus/config.toml.bak.2026-05-06T12-34-56Z")
        );
    }

    #[test]
    fn defaults_round_trip_via_toml() {
        // The `reset` operation writes Config::default() as TOML; we must be
        // able to read it back into the same shape and key defaults must
        // survive the trip.
        let cfg = Config::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.probes.quorum_n, cfg.probes.quorum_n);
        assert_eq!(back.probes.quorum_total, cfg.probes.quorum_total);
        assert_eq!(
            back.dns.strip_edns_client_subnet,
            cfg.dns.strip_edns_client_subnet
        );
        assert_eq!(back.mac.rotation_interval, cfg.mac.rotation_interval);
        assert_eq!(back.mac.oui_pool, cfg.mac.oui_pool);
    }
}
