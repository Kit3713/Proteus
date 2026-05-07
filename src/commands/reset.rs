// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::exit;
use crate::profile::Profile;

/// Reset config to a minimal "profile only" file.
///
/// Mutating; requires root and `--yes`. The cached original MACs and hostname
/// in `state.json` are sacred and untouched. `apply` is not invoked
/// automatically — the user decides when to re-apply.
///
/// The written file contains only `profile = "<name>"`. The active profile
/// is preserved from the existing config when present, otherwise it falls
/// back to the built-in default. Resolution at load time fills in every
/// other knob from the profile baseline. This is the inverse of the bug in
/// issue #131 where reset wrote the resolved (frozen) Config back to disk
/// and turned every default into an explicit override.
pub fn run(yes: bool, dry_run: bool, config_override: Option<&Path>) -> Result<u8> {
    // `--dry-run` is a free preview, so it implicitly satisfies the
    // confirmation gate. The shared helper handles the message + exit code
    // for the unconfirmed real-run path.
    if !dry_run
        && let Err(code) = super::require_yes(
            yes,
            "reset is destructive (or pass --dry-run to preview)",
            "proteus wiki concepts",
        )
    {
        return Ok(code);
    }

    if !dry_run {
        if let Err(e) = super::require_root() {
            eprintln!("proteus: {e}");
            return Ok(exit::PERMISSION_ERROR);
        }
    }

    let path = super::config_path(config_override);
    let backup_path = backup_path_for(&path, &super::now_iso8601());
    let profile = active_profile(&path);

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
            "  would write minimal profile-only config to {} (profile = \"{}\")",
            path.display(),
            profile.name()
        );
        println!("  would NOT call `proteus apply` — re-apply manually when ready");
        return Ok(exit::SUCCESS);
    }

    let minimal = render_minimal(profile);
    let backed_up = backup_existing(&path, &backup_path)?;
    super::write_atomic(&path, minimal.as_bytes())?;

    if backed_up {
        println!(
            "config reset to profile = \"{}\"; previous config saved to {}",
            profile.name(),
            backup_path.display()
        );
    } else {
        println!(
            "config reset to profile = \"{}\"; no previous config at {}",
            profile.name(),
            path.display()
        );
    }
    Ok(exit::SUCCESS)
}

/// Read the currently-active profile from `path`. If the file is absent or
/// unparseable we fall back to the built-in default — reset's job is to
/// produce a clean, valid file even when the previous one was broken.
fn active_profile(path: &Path) -> Profile {
    Config::default_or_loaded(path)
        .map(|c| c.profile)
        .unwrap_or_default()
}

/// Render the minimal on-disk form: just `profile = "<name>"`. Resolution
/// at load time fills in every other knob from the profile baseline, which
/// is the same override-only-if-present model used everywhere else.
fn render_minimal(profile: Profile) -> String {
    format!("profile = \"{}\"\n", profile.name())
}

/// Maximum number of `.bak.<ts>` files to retain per config file. Older
/// backups are pruned at reset time (issue #161). Five is enough to recover
/// from a recent mistake without letting cached identifiers (pinned MAC,
/// pinned alias, pinned hostname) accumulate indefinitely on disk.
const MAX_BACKUPS: usize = 5;

/// Copy `path` to `backup_path` if it exists, then prune older sibling
/// backups beyond `MAX_BACKUPS`. Returns whether a backup was made.
/// Avoids a TOCTOU pre-check by reacting to NotFound from `fs::copy`.
fn backup_existing(path: &Path, backup_path: &Path) -> Result<bool> {
    let made = match fs::copy(path, backup_path) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            return Err(e).with_context(|| {
                format!("backing up {} to {}", path.display(), backup_path.display())
            });
        }
    };
    if made {
        prune_old_backups(path);
    }
    Ok(made)
}

/// Issue #161: prune `.bak.<ts>` siblings beyond `MAX_BACKUPS`. ISO-8601
/// timestamps sort lexicographically by chronology, so the oldest are the
/// first when the list is sorted. Best-effort: failures here don't fail
/// the reset — the user already got a clean config write.
fn prune_old_backups(config_path: &Path) {
    let Some(parent) = config_path.parent() else {
        return;
    };
    let Some(name) = config_path.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let prefix = format!("{name}.bak.");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let mut backups: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(&prefix))
        })
        .collect();
    backups.sort();
    let to_remove = backups.len().saturating_sub(MAX_BACKUPS);
    for path in backups.into_iter().take(to_remove) {
        let _ = fs::remove_file(&path);
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
    fn render_minimal_writes_only_profile_line() {
        // Issue #131: reset must not bloat the file with every default
        // explicitly set. The on-disk form is exactly one assignment.
        let s = render_minimal(Profile::Med);
        assert_eq!(s, "profile = \"med\"\n");
    }

    #[test]
    fn render_minimal_round_trips_through_resolver() {
        // After reset the file must still resolve to the profile baseline
        // when read back. Without per-knob overrides, that is exactly the
        // built-in baseline for the chosen profile.
        for p in [
            Profile::Off,
            Profile::Min,
            Profile::Low,
            Profile::Med,
            Profile::High,
            Profile::Agr,
        ] {
            let s = render_minimal(p);
            let raw: crate::config::RawConfig =
                toml::from_str(&s).expect("minimal profile-only TOML parses");
            assert!(
                !raw.has_overrides(),
                "render_minimal should produce zero per-knob overrides for {:?}",
                p
            );
            let resolved = raw.resolve();
            assert_eq!(resolved.profile, p, "resolved profile mismatch for {:?}", p);
        }
    }

    #[test]
    fn active_profile_falls_back_to_default_when_path_missing() {
        let nowhere = Path::new("/nonexistent/proteus/no-such-config.toml");
        assert_eq!(active_profile(nowhere), Profile::default());
    }

    #[test]
    fn active_profile_reads_existing_profile_from_file() {
        // Per-test unique path so parallel cargo runs don't collide.
        let cfg = std::env::temp_dir().join(format!(
            "proteus-reset-active-profile-{}-{}.toml",
            std::process::id(),
            line!()
        ));
        fs::write(&cfg, "profile = \"high\"\n[mac]\nenabled = false\n").unwrap();
        assert_eq!(active_profile(&cfg), Profile::High);
        let _ = fs::remove_file(&cfg);
    }
}
