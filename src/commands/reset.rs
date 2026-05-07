// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::io::Write;
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
    write_atomic(&path, minimal.as_bytes())?;

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
            assert_eq!(
                resolved.profile,
                p,
                "resolved profile mismatch for {:?}",
                p
            );
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
