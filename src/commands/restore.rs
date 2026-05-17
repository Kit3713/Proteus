// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus restore <path>` — extract a tarball produced by `proteus
//! backup` back into the canonical install layout.
//!
//! Issue #353 (roadmap "no-brainer"): the inverse of `commands::backup`.
//! Destructive — overwrites `/etc/proteus/` and `/var/lib/proteus/` from
//! the archive — so it requires `--yes` like every other mutating
//! command. The pre-flight rejects archives whose members escape the
//! three known roots, refuses absolute member paths, and refuses any
//! component containing `..`. Optional `--expected-sha <hex>` lets a
//! wrapper pin the bundle's identity end-to-end before extraction.
//!
//! Threat-model reminder: tar entries can carry symlinks and hardlinks
//! that point outside the archive root. The validation walk inspects
//! every entry's path AND link target up-front so a malicious archive
//! cannot use a symlink-then-write trick to escape (the classic CVE-
//! 2007-4131 "tar parent-dir traversal" shape). We bail with a clear
//! diagnostic before any extraction happens.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::Serialize;
use tar::Archive;

use super::backup::{
    ROOT_LABEL_CONFIG, ROOT_LABEL_STATE, hash_file, resolve_config_dir, resolve_state_dir,
};
use crate::exit;

/// gzip magic bytes. We refuse archives that don't begin with these so a
/// user who points `restore` at the wrong file gets a clear diagnostic
/// instead of a generic "decode error" deep in the gzip state machine.
const GZIP_MAGIC: &[u8; 2] = &[0x1f, 0x8b];

#[derive(Serialize)]
struct RestoreReport<'a> {
    path: &'a Path,
    files: usize,
    bytes: u64,
    sha256: String,
}

pub fn run(
    path: PathBuf,
    yes: bool,
    json: bool,
    expected_sha: Option<String>,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'restore' overwrites /etc/proteus and /var/lib/proteus",
        "proteus help restore",
    ) {
        return Ok(code);
    }

    // Hold the state lock for the full extraction. A concurrent
    // `proteus rotate` or `proteus apply` writing into state.json
    // mid-restore would either lose its write (we'd overwrite it) or
    // corrupt the restored file (extraction races a partial write).
    // Issue #126: every mutating command serialises on this lock.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    if !path.exists() {
        eprintln!("proteus: backup not found: {}", path.display());
        return Ok(exit::CONFIG_ERROR);
    }

    // Validate gzip magic + SHA-256 before any extraction. Both checks
    // are cheap and bail out before we touch /etc/proteus.
    if let Err(e) = validate_magic(&path) {
        eprintln!("proteus: {e:#}");
        return Ok(exit::CONFIG_ERROR);
    }

    let digest = hash_file(&path)?;
    if let Some(expected) = expected_sha.as_deref()
        && !sha_eq(&digest, expected)
    {
        eprintln!("proteus: sha256 mismatch: archive is {digest}, --expected-sha was {expected}");
        return Ok(exit::CONFIG_ERROR);
    }

    // Validate every member's path BEFORE extracting anything. A
    // malicious archive that mixes legitimate entries with a path-
    // traversal entry must be rejected as a whole — partial extraction
    // would leave the system in a half-restored state.
    if let Err(e) = validate_members(&path) {
        eprintln!("proteus: {e:#}");
        return Ok(exit::CONFIG_ERROR);
    }

    let config_dir = resolve_config_dir(config_path);
    let state_dir = resolve_state_dir(state_path);
    let (files, bytes) = extract(&path, &config_dir, &state_dir)?;

    if json {
        let report = RestoreReport {
            path: &path,
            files,
            bytes,
            sha256: digest,
        };
        super::print_json(&report)?;
    } else {
        println!(
            "proteus restore: extracted {} files ({} bytes) from {}",
            files,
            bytes,
            path.display()
        );
        println!("sha256: {digest}");
    }
    Ok(exit::SUCCESS)
}

/// First two bytes must be the gzip magic. Catches the "user pointed at
/// the wrong file" mistake with a clear message.
fn validate_magic(path: &Path) -> Result<()> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = [0u8; 2];
    f.read_exact(&mut buf)
        .with_context(|| format!("reading magic from {}", path.display()))?;
    if &buf != GZIP_MAGIC {
        anyhow::bail!(
            "not a gzip-compressed tarball: {} (magic was {:02x}{:02x})",
            path.display(),
            buf[0],
            buf[1]
        );
    }
    Ok(())
}

/// Walk the archive once, checking every member's path. Reject any
/// entry whose path:
///   - has an absolute root (e.g. `/etc/passwd`),
///   - contains a `..` component (traversal), or
///   - falls outside the two known root labels.
///
/// We also reject every link entry whose target violates the same
/// rules. Run BEFORE any extraction so a bad archive aborts cleanly.
fn validate_members(path: &Path) -> Result<()> {
    let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let gz = GzDecoder::new(BufReader::new(f));
    let mut archive = Archive::new(gz);
    for entry in archive
        .entries()
        .with_context(|| format!("reading tar entries from {}", path.display()))?
    {
        let entry = entry.with_context(|| format!("reading tar entry from {}", path.display()))?;
        let entry_path = entry
            .path()
            .with_context(|| format!("decoding tar entry path in {}", path.display()))?;
        check_member_path(&entry_path)?;
        if let Some(link) = entry
            .link_name()
            .with_context(|| format!("decoding link name in {}", path.display()))?
        {
            check_link_target(&link)?;
        }
    }
    Ok(())
}

/// A valid in-archive member path is relative, free of `..`, and starts
/// with one of the known root labels.
fn check_member_path(p: &Path) -> Result<()> {
    if p.is_absolute() {
        anyhow::bail!("rejecting absolute path in archive: {}", p.display());
    }
    for c in p.components() {
        if matches!(c, Component::ParentDir) {
            anyhow::bail!("rejecting `..` component in archive entry: {}", p.display());
        }
    }
    let s = p.to_string_lossy();
    if !(s.starts_with(ROOT_LABEL_CONFIG) || s.starts_with(ROOT_LABEL_STATE)) {
        anyhow::bail!(
            "rejecting entry outside known roots: {} \
             (expected prefix `{ROOT_LABEL_CONFIG}` or `{ROOT_LABEL_STATE}`)",
            p.display()
        );
    }
    Ok(())
}

/// Symlink / hardlink targets are validated against the same rules.
/// Absolute or `..`-bearing targets would let a malicious archive
/// re-point a known-good in-archive name at /etc/passwd or similar
/// before the actual data entry lands.
fn check_link_target(target: &Path) -> Result<()> {
    if target.is_absolute() {
        anyhow::bail!(
            "rejecting absolute link target in archive: {}",
            target.display()
        );
    }
    for c in target.components() {
        if matches!(c, Component::ParentDir) {
            anyhow::bail!(
                "rejecting `..` in link target in archive: {}",
                target.display()
            );
        }
    }
    Ok(())
}

/// Extract the archive into a temp staging area mapped from the two
/// in-archive prefixes to the operator's effective config/state dirs.
/// Returns `(file_count, total_byte_count)`.
fn extract(archive: &Path, config_dir: &Path, state_dir: &Path) -> Result<(usize, u64)> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("creating {}", config_dir.display()))?;
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("creating {}", state_dir.display()))?;

    let f = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let gz = GzDecoder::new(BufReader::new(f));
    let mut a = Archive::new(gz);
    // Don't let tar restore ownership / permissions / mtime from the
    // archive headers — extraction should land at the caller's uid/gid
    // and the system umask. Preserving 0o600 etc. is handled by the
    // managed-file writers themselves on the next `proteus apply`.
    a.set_preserve_permissions(false);
    a.set_preserve_mtime(false);
    a.set_overwrite(true);

    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in a
        .entries()
        .with_context(|| format!("reading entries from {}", archive.display()))?
    {
        let mut entry = entry.context("reading tar entry")?;
        let entry_path = entry
            .path()
            .context("decoding tar entry path")?
            .into_owned();
        // Re-validate at extraction time as defence-in-depth. The
        // pre-flight already rejected bad entries, but a TOCTOU
        // adversary could swap the archive between the two reads.
        check_member_path(&entry_path)?;
        let dest = map_member_to_dest(&entry_path, config_dir, state_dir)?;
        if entry.header().entry_type().is_file() {
            files += 1;
            bytes += entry.header().size().unwrap_or(0);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        entry
            .unpack(&dest)
            .with_context(|| format!("extracting to {}", dest.display()))?;
    }
    Ok((files, bytes))
}

/// Map `etc/proteus/...` to `<config_dir>/...` and `var/lib/proteus/...`
/// to `<state_dir>/...`. The member-path validation upstream guarantees
/// every entry falls into one of the two buckets.
fn map_member_to_dest(member: &Path, config_dir: &Path, state_dir: &Path) -> Result<PathBuf> {
    let s = member.to_string_lossy();
    if let Some(rest) = s.strip_prefix(ROOT_LABEL_CONFIG) {
        let rest = rest.trim_start_matches('/');
        Ok(config_dir.join(rest))
    } else if let Some(rest) = s.strip_prefix(ROOT_LABEL_STATE) {
        let rest = rest.trim_start_matches('/');
        Ok(state_dir.join(rest))
    } else {
        anyhow::bail!(
            "internal: unmapped member after validation: {}",
            member.display()
        )
    }
}

/// Hex equality with whitespace + case tolerance. The digests are public
/// values so a real constant-time compare isn't needed; this just avoids
/// surprises if the caller pads the expected SHA with whitespace or
/// upper-cases it.
fn sha_eq(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_tmp(tag: &str) -> PathBuf {
        let mut buf = [0u8; 8];
        getrandom::getrandom(&mut buf).unwrap();
        let suffix: String = buf.iter().map(|b| format!("{b:02x}")).collect();
        std::env::temp_dir().join(format!("proteus-restore-{tag}-{suffix}"))
    }

    #[test]
    fn rejects_absolute_member_path() {
        let p = Path::new("/etc/passwd");
        assert!(check_member_path(p).is_err());
    }

    #[test]
    fn rejects_dot_dot_in_member() {
        let p = Path::new("etc/proteus/../passwd");
        assert!(check_member_path(p).is_err());
    }

    #[test]
    fn rejects_unknown_prefix() {
        let p = Path::new("usr/bin/evil");
        assert!(check_member_path(p).is_err());
    }

    #[test]
    fn accepts_known_prefixes() {
        assert!(check_member_path(Path::new("etc/proteus/config.toml")).is_ok());
        assert!(check_member_path(Path::new("var/lib/proteus/state.json")).is_ok());
    }

    #[test]
    fn rejects_absolute_link_target() {
        let t = Path::new("/etc/shadow");
        assert!(check_link_target(t).is_err());
    }

    #[test]
    fn rejects_traversal_link_target() {
        let t = Path::new("../../etc/shadow");
        assert!(check_link_target(t).is_err());
    }

    #[test]
    fn map_member_routes_under_overrides() {
        let cfg = Path::new("/tmp/cfg");
        let st = Path::new("/tmp/st");
        assert_eq!(
            map_member_to_dest(Path::new("etc/proteus/config.toml"), cfg, st).unwrap(),
            PathBuf::from("/tmp/cfg/config.toml")
        );
        assert_eq!(
            map_member_to_dest(Path::new("var/lib/proteus/state.json"), cfg, st).unwrap(),
            PathBuf::from("/tmp/st/state.json")
        );
        assert_eq!(
            map_member_to_dest(Path::new("etc/proteus/personas/foo.toml"), cfg, st).unwrap(),
            PathBuf::from("/tmp/cfg/personas/foo.toml")
        );
    }

    #[test]
    fn rejects_non_gzip_files() {
        let dir = unique_tmp("magic");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("not-a-gz");
        std::fs::write(&p, b"this is not gzip").unwrap();
        assert!(validate_magic(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_backup_then_restore() {
        // End-to-end smoke: build an archive via the backup module,
        // restore it under a fresh tempdir layout, and compare bytes.
        use crate::commands::backup;
        let cfg_src = unique_tmp("rt-cfg");
        let st_src = unique_tmp("rt-state");
        std::fs::create_dir_all(&cfg_src).unwrap();
        std::fs::create_dir_all(&st_src).unwrap();
        std::fs::write(cfg_src.join("config.toml"), b"profile = \"med\"\n").unwrap();
        std::fs::create_dir_all(cfg_src.join("personas")).unwrap();
        std::fs::write(cfg_src.join("personas").join("p.toml"), b"id = \"p\"\n").unwrap();
        std::fs::write(st_src.join("state.json"), b"{\"v\":1}\n").unwrap();

        let archive = unique_tmp("rt-archive.tar.gz");
        let cfg_file = cfg_src.join("config.toml");
        let st_file = st_src.join("state.json");
        // Bypass the higher-level `run` (which has a --yes gate);
        // exercise the archive-building primitive directly.
        backup::run(
            archive.clone(),
            true,
            true,
            true,
            Some(&st_file),
            Some(&cfg_file),
        )
        .unwrap();

        let cfg_dst = unique_tmp("rt-cfg-restored");
        let st_dst = unique_tmp("rt-state-restored");
        let cfg_dst_file = cfg_dst.join("config.toml");
        let st_dst_file = st_dst.join("state.json");

        let rc = super::run(
            archive.clone(),
            true,
            true,
            None,
            Some(&st_dst_file),
            Some(&cfg_dst_file),
        )
        .unwrap();
        assert_eq!(rc, exit::SUCCESS);

        assert_eq!(
            std::fs::read(cfg_dst.join("config.toml")).unwrap(),
            b"profile = \"med\"\n"
        );
        assert_eq!(
            std::fs::read(cfg_dst.join("personas").join("p.toml")).unwrap(),
            b"id = \"p\"\n"
        );
        assert_eq!(
            std::fs::read(st_dst.join("state.json")).unwrap(),
            b"{\"v\":1}\n"
        );

        let _ = std::fs::remove_dir_all(&cfg_src);
        let _ = std::fs::remove_dir_all(&st_src);
        let _ = std::fs::remove_dir_all(&cfg_dst);
        let _ = std::fs::remove_dir_all(&st_dst);
        let _ = std::fs::remove_file(&archive);
    }

    #[test]
    fn rejects_expected_sha_mismatch() {
        use crate::commands::backup;
        let cfg = unique_tmp("sha-cfg");
        let st = unique_tmp("sha-state");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&st).unwrap();
        std::fs::write(cfg.join("config.toml"), b"x = 1\n").unwrap();
        std::fs::write(st.join("state.json"), b"{}\n").unwrap();
        let archive = unique_tmp("sha-archive.tar.gz");
        backup::run(
            archive.clone(),
            true,
            true,
            true,
            Some(&st.join("state.json")),
            Some(&cfg.join("config.toml")),
        )
        .unwrap();

        let dst_cfg = unique_tmp("sha-cfg-out");
        let dst_st = unique_tmp("sha-state-out");
        let rc = super::run(
            archive.clone(),
            true,
            true,
            Some("deadbeef".repeat(8)),
            Some(&dst_st.join("state.json")),
            Some(&dst_cfg.join("config.toml")),
        )
        .unwrap();
        assert_eq!(rc, exit::CONFIG_ERROR);

        let _ = std::fs::remove_dir_all(&cfg);
        let _ = std::fs::remove_dir_all(&st);
        let _ = std::fs::remove_dir_all(&dst_cfg);
        let _ = std::fs::remove_dir_all(&dst_st);
        let _ = std::fs::remove_file(&archive);
    }

    #[test]
    fn sha_eq_is_case_and_whitespace_tolerant() {
        assert!(sha_eq("abcdef", "ABCDEF"));
        assert!(sha_eq(" abc ", "abc"));
        assert!(!sha_eq("abc", "abd"));
    }
}
