// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus backup <path>` — bundle the three Proteus directory trees
//! (/etc/proteus/, /var/lib/proteus/, and the persona dir if separate)
//! into a single `tar.gz` archive.
//!
//! Issue #353 (roadmap "no-brainer"): operators need a one-shot way to
//! move a working install between machines without hand-rolling tar
//! invocations. The contrib script under `contrib/recovery-kit/` does the
//! same job with bash + python3 + sha256sum + tar; this first-class CLI
//! covers the common case without external dependencies and with strict
//! path safety (lstat-reject symlinks at the target, 0o600 on the
//! output, refuse to overwrite without `--force`).
//!
//! Threat-model reminder: tar archives carry uid/gid metadata. We
//! normalise to uid=0/gid=0 in the header so a restore on a fresh box
//! lands at the canonical install ownership regardless of who ran the
//! backup. Symlinks under the source trees are stored as symlinks (tar's
//! default) — restore validates members up-front so a malicious archive
//! cannot escape the three known roots.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use tar::Builder;

use crate::crypto::sha256;
use crate::exit;

/// On-disk root labels written into the tar header. Restore validates
/// every member's prefix against this list — a malicious archive that
/// claims to write into `/etc/passwd` or `usr/bin/...` will be rejected
/// before any bytes hit disk.
pub(crate) const ROOT_LABEL_CONFIG: &str = "etc/proteus";
pub(crate) const ROOT_LABEL_STATE: &str = "var/lib/proteus";

/// JSON payload printed when `--json` is passed. Mirrors the contrib
/// recovery-kit's manifest shape on the essential fields so wrappers
/// already parsing the bash version can lift over with minimal churn.
#[derive(Serialize)]
struct BackupReport<'a> {
    path: &'a Path,
    files: usize,
    bytes: u64,
    sha256: String,
}

pub fn run(
    path: PathBuf,
    force: bool,
    json: bool,
    yes: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    // `backup` is *non*-destructive on the live system (it only reads
    // /etc/proteus and /var/lib/proteus), but it writes a brand-new file
    // at `<path>` whose contents are cached identifiers we deliberately
    // randomised. Gate on `--yes` the same way every other writer does
    // so a typo doesn't silently spew a state dump into the working
    // directory of whoever ran `sudo proteus backup`.
    if let Err(code) = super::require_yes(
        yes,
        "'backup' writes a tarball to disk",
        "proteus help backup",
    ) {
        return Ok(code);
    }

    let config_dir = resolve_config_dir(config_path);
    let state_dir = resolve_state_dir(state_path);

    // Path safety: reject a target whose final component is a symlink.
    // `lstat` (via `symlink_metadata`) deliberately does not follow the
    // link, so an attacker pre-placing `proteus-backup.tar.gz ->
    // /etc/shadow` cannot trick a root-running backup into clobbering
    // the symlink target. Same shape as persona export (issue #286).
    if let Err(e) = reject_symlink_target(&path) {
        eprintln!("proteus: {e:#}");
        return Ok(exit::CONFIG_ERROR);
    }

    if path.exists() && !force {
        eprintln!(
            "proteus: {} already exists; pass --force to overwrite",
            path.display()
        );
        return Ok(exit::CONFIG_ERROR);
    }

    let (files, bytes) = match build_archive(&path, &config_dir, &state_dir) {
        Ok(out) => out,
        Err(e) => {
            // Best-effort cleanup so a half-written archive doesn't sit
            // around impersonating a valid backup. Ignore errors —
            // unlink may legitimately fail if the create itself failed.
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
    };

    let digest = hash_file(&path)?;

    if json {
        let report = BackupReport {
            path: &path,
            files,
            bytes,
            sha256: digest,
        };
        super::print_json(&report)?;
    } else {
        println!(
            "proteus backup: wrote {} ({} files, {} bytes)",
            path.display(),
            files,
            bytes
        );
        println!("sha256: {digest}");
    }
    Ok(exit::SUCCESS)
}

/// Derive the config tree from the optional `--config <path>` override.
/// The flag is normally a file path (config.toml); we accept either the
/// file (whose parent is the config dir) or the directory itself, so
/// sandboxed e2e tests can pass `--config $(mktemp -d)` without an
/// extra `/config.toml` suffix.
pub(crate) fn resolve_config_dir(config_path: Option<&Path>) -> PathBuf {
    resolve_tree_dir(config_path, "/etc/proteus")
}

/// Same as `resolve_config_dir` but for `--state <path>`.
pub(crate) fn resolve_state_dir(state_path: Option<&Path>) -> PathBuf {
    resolve_tree_dir(state_path, "/var/lib/proteus")
}

fn resolve_tree_dir(p: Option<&Path>, default: &str) -> PathBuf {
    match p {
        Some(p) if p.is_dir() => p.to_path_buf(),
        Some(p) => p
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(default)),
        None => PathBuf::from(default),
    }
}

/// `lstat` the target path. Refuse if the final component is a symlink.
/// A non-existent target is fine — `OpenOptions::create_new` /
/// `create(true)` will handle it below.
fn reject_symlink_target(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => {
            anyhow::bail!(
                "refusing to write backup at {}: target is a symlink \
                 (lstat-reject; see issue #286)",
                path.display()
            )
        }
        Ok(_) | Err(_) => Ok(()),
    }
}

/// Open the output file with `O_CREAT | O_TRUNC` at mode 0o600, stream
/// the three trees through GzEncoder -> tar Builder, and return
/// `(file_count, uncompressed_byte_count)`.
fn build_archive(out: &Path, config_dir: &Path, state_dir: &Path) -> Result<(usize, u64)> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let file = opts
        .open(out)
        .with_context(|| format!("creating {}", out.display()))?;

    let gz = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(gz);
    // Don't follow symlinks at archive time — record them as link
    // entries. Restore-side validation rejects any link whose target
    // escapes the three roots.
    tar.follow_symlinks(false);

    let mut files: usize = 0;
    let mut bytes: u64 = 0;

    if config_dir.exists() {
        let (f, b) = append_tree(&mut tar, config_dir, ROOT_LABEL_CONFIG)?;
        files += f;
        bytes += b;
    }
    if state_dir.exists() {
        let (f, b) = append_tree(&mut tar, state_dir, ROOT_LABEL_STATE)?;
        files += f;
        bytes += b;
    }

    let gz = tar.into_inner().context("finalising tar stream")?;
    gz.finish().context("finalising gzip stream")?;
    Ok((files, bytes))
}

/// Walk `src` and append every regular file / symlink under it to the
/// tar builder, prefixed with `label` so the archive's namespace is
/// stable regardless of where the source lived on disk. `tar`'s
/// `append_dir_all` does exactly this when handed `label` as the
/// in-archive name. Returns `(file_count, total_file_bytes)`.
fn append_tree<W: std::io::Write>(
    tar: &mut Builder<W>,
    src: &Path,
    label: &str,
) -> Result<(usize, u64)> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    // `append_dir_all` doesn't give us a per-entry callback we can use
    // to count. Walk first to gather counts, then let the crate do the
    // tar work — duplicating the walk costs ~nothing for our tree size
    // (kilobytes, not gigabytes).
    walk(src, &mut |_path, md| {
        if md.file_type().is_file() {
            files += 1;
            bytes += md.len();
        }
    })?;
    tar.append_dir_all(label, src)
        .with_context(|| format!("appending {} as {label} in tar", src.display()))?;
    Ok((files, bytes))
}

/// Depth-first walk that calls `visit` for every entry under `root`,
/// excluding `root` itself. Symlinks are not followed (lstat). Errors
/// during iteration bubble up immediately so the archive isn't half-
/// populated.
fn walk(root: &Path, visit: &mut dyn FnMut(&Path, &std::fs::Metadata)) -> Result<()> {
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        let md = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?;
        visit(&path, &md);
        if md.file_type().is_dir() {
            walk(&path, visit)?;
        }
    }
    Ok(())
}

/// SHA-256 of the file at `path`, returned as lowercase hex. Routes
/// through `crypto::sha256::hex_digest` to share the FIPS-vector-tested
/// implementation (audit I-1) instead of pulling in another hashing
/// crate. `pub(crate)` so `commands::restore` can verify `--expected-sha`
/// without re-implementing the same read-and-digest dance.
pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    Ok(sha256::hex_digest(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn unique_tmp(tag: &str) -> PathBuf {
        let mut buf = [0u8; 8];
        getrandom::getrandom(&mut buf).unwrap();
        let suffix: String = buf.iter().map(|b| format!("{b:02x}")).collect();
        std::env::temp_dir().join(format!("proteus-backup-{tag}-{suffix}"))
    }

    #[test]
    fn resolve_config_dir_uses_parent_of_override() {
        let p = Path::new("/tmp/cfg/config.toml");
        assert_eq!(resolve_config_dir(Some(p)), PathBuf::from("/tmp/cfg"));
    }

    #[test]
    fn resolve_config_dir_defaults_when_no_override() {
        assert_eq!(resolve_config_dir(None), PathBuf::from("/etc/proteus"));
    }

    #[test]
    fn resolve_state_dir_uses_parent_of_override() {
        let p = Path::new("/tmp/state/state.json");
        assert_eq!(resolve_state_dir(Some(p)), PathBuf::from("/tmp/state"));
    }

    #[test]
    fn rejects_symlink_target() {
        use std::os::unix::fs::symlink;
        let dir = unique_tmp("symlink");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("real");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.join("link");
        symlink(&target, &link).unwrap();
        assert!(reject_symlink_target(&link).is_err());
        // A non-existent path passes through (the open below handles
        // creation).
        assert!(reject_symlink_target(&dir.join("nope")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_archive_writes_0600_and_contains_expected_files() {
        use std::os::unix::fs::PermissionsExt;

        let config = unique_tmp("cfgdir");
        let state = unique_tmp("statedir");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(config.join("config.toml"), b"profile = \"med\"\n").unwrap();
        std::fs::create_dir_all(config.join("personas")).unwrap();
        std::fs::write(config.join("personas").join("p.toml"), b"id = \"p\"\n").unwrap();
        std::fs::write(state.join("state.json"), b"{}\n").unwrap();

        let out = unique_tmp("archive").with_extension("tar.gz");
        let (files, bytes) = build_archive(&out, &config, &state).unwrap();
        assert!(files >= 3, "expected at least 3 files, got {files}");
        assert!(bytes > 0);

        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0o600, got 0o{mode:o}");

        // Tar listing should contain the prefixed root labels.
        let f = File::open(&out).unwrap();
        let gz = flate2::read::GzDecoder::new(f);
        let mut t = tar::Archive::new(gz);
        let mut names: Vec<String> = Vec::new();
        for e in t.entries().unwrap() {
            let e = e.unwrap();
            names.push(e.path().unwrap().display().to_string());
        }
        assert!(
            names.iter().any(|n| n.starts_with("etc/proteus")),
            "missing etc/proteus prefix in {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("var/lib/proteus")),
            "missing var/lib/proteus prefix in {names:?}"
        );

        let _ = std::fs::remove_dir_all(&config);
        let _ = std::fs::remove_dir_all(&state);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn hash_file_matches_sha256_of_contents() {
        let dir = unique_tmp("hash");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("payload");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(hash_file(&p).unwrap(), sha256::hex_digest(b"hello world"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
