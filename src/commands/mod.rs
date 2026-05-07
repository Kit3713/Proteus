// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod bluetooth_cmd;
pub mod config_cmd;
pub mod current;
pub mod dhcp;
pub mod diff;
pub mod dns;
pub mod doctor;
pub mod dry_run;
pub mod enterprise_wifi;
pub mod hostname;
pub mod ipv6;
pub mod kill;
pub mod nft;
pub mod original;
pub mod pin;
pub mod portal;
pub mod probe;
pub mod reset;
pub mod revert;
pub mod rf;
pub mod rotate;
pub mod session;
pub mod show_config;
pub mod show_defaults;
pub mod stack;
pub mod status;
pub mod stub;
pub mod timer;
pub mod uninstall;
pub mod unpin;
pub mod wiki_cmd;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
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

/// `--yes` confirmation gate shared by every mutating subcommand.
///
/// Returns `Ok(())` when the user passed `--yes`; otherwise prints a
/// uniform "this is mutating, pass --yes" line to stderr (with the caller's
/// `description` text and `wiki_hint` pointer) and yields the
/// `CONFIRMATION_REQUIRED` exit code via `Err`. The caller wires it in as
/// `if let Err(code) = require_yes(...) { return Ok(code); }`.
///
/// `description` should briefly explain *why* the command is destructive
/// (e.g. `"is mutating (writes state.json)"`) so the operator sees what
/// they're confirming. `wiki_hint` is the trailing pointer the operator
/// can read for context (e.g. `"proteus help pin"`).
pub(crate) fn require_yes(yes: bool, description: &str, wiki_hint: &str) -> Result<(), u8> {
    if yes {
        return Ok(());
    }
    eprintln!("proteus: {description}; pass --yes to confirm (see `{wiki_hint}`)");
    Err(crate::exit::CONFIRMATION_REQUIRED)
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

pub(crate) fn unix_to_ymdhms(mut t: u64) -> (u32, u32, u32, u32, u32, u32) {
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
///
/// The destination always lands at mode `0o600` (owner read/write only). State
/// and config files cache identifiers we just spent effort to randomise — the
/// original MAC, original hostname, per-NM-connection identifiers — so they
/// must not be world-readable on multi-user systems (issue #116).
///
/// Defends against TOCTOU/symlink attacks (issues #125, #150) by:
/// - Naming the temp file with a random suffix (`<name>.proteus-<rand>.tmp`)
///   so an attacker cannot pre-place a symlink at the temp path.
/// - Opening the temp file with `O_CREAT | O_EXCL` so an existing file at
///   that exact path is a hard error, never followed.
/// - Wrapping the temp path in an RAII guard that removes it on drop, so
///   error returns never leak `.tmp` litter on disk.
/// - Calling `sync_all` on the parent directory after rename so the
///   directory entry survives a crash, not just the file contents.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let tmp = tmp_path_for(path)?;
    let guard = TmpFile(tmp.clone());
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true) // O_CREAT | O_EXCL — never follow an existing path
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    // Rename is durable only after the parent directory entry is fsynced.
    let dir = File::open(parent)
        .with_context(|| format!("opening parent dir {} for fsync", parent.display()))?;
    dir.sync_all()
        .with_context(|| format!("fsync parent dir {}", parent.display()))?;
    // Rename succeeded; the temp path is now the destination, so don't let
    // the guard remove the freshly-renamed file on drop.
    std::mem::forget(guard);
    Ok(())
}

/// Build a per-call temp path with a random suffix. Random bytes come from
/// `getrandom` (already a dep) so two concurrent writes against the same
/// target cannot collide and a non-root attacker cannot guess the name.
fn tmp_path_for(path: &Path) -> Result<PathBuf> {
    let mut rand = [0u8; 8];
    getrandom::getrandom(&mut rand).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    let suffix: String = rand.iter().map(|b| format!("{b:02x}")).collect();
    let base = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    Ok(path.with_file_name(format!("{base}.proteus-{suffix}.tmp")))
}

/// RAII guard that removes a path on drop. Used by `write_atomic` so an
/// error mid-write doesn't leave a `.tmp` orphan; on success the caller
/// `mem::forget`s the guard.
struct TmpFile(PathBuf);

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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

#[cfg(test)]
mod write_atomic_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::thread;

    /// Small RAII tempdir kept here so these tests don't reach across modules.
    /// Removed on drop; collision-resistant via getrandom suffix.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut buf = [0u8; 8];
            getrandom::getrandom(&mut buf).unwrap();
            let suffix: String = buf.iter().map(|b| format!("{b:02x}")).collect();
            let path = std::env::temp_dir().join(format!("proteus-write-atomic-{tag}-{suffix}"));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Returns true iff `dir` contains any file whose name ends in `.tmp`.
    fn any_tmp_leaks(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
    }

    /// Issue #116: state.json caches the original MAC, original hostname, and
    /// per-NM-connection identifiers — exactly the values we just tried to
    /// hide from the network. Anything written through `write_atomic` must
    /// land at `0o600` so an unprivileged user on the same machine can't
    /// read those originals straight off disk.
    #[test]
    fn write_atomic_writes_0600_mode() {
        let tmp = TempDir::new("mode");
        let path = tmp.0.join("state.json");
        write_atomic(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0o600, got 0o{mode:o}");
        // Re-writing the same path must keep the strict mode — the new tmp
        // file is created fresh each call and renamed over the destination.
        write_atomic(&path, b"{\"x\":1}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0o600 on overwrite, got 0o{mode:o}");
    }

    #[test]
    fn writes_contents_and_no_tmp_leak_on_success() {
        let tmp = TempDir::new("ok");
        let path = tmp.0.join("payload");
        write_atomic(&path, b"hello\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello\n");
        assert!(
            !any_tmp_leaks(&tmp.0),
            "unexpected .tmp leak under {}",
            tmp.0.display()
        );
    }

    #[test]
    fn temp_filename_contains_random_suffix() {
        let tmp = TempDir::new("rand");
        let path = tmp.0.join("payload");
        let a = tmp_path_for(&path).unwrap();
        let b = tmp_path_for(&path).unwrap();
        let a_name = a.file_name().unwrap().to_string_lossy().into_owned();
        let b_name = b.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            a_name.starts_with("payload.proteus-") && a_name.ends_with(".tmp"),
            "unexpected temp name: {a_name}"
        );
        assert!(
            b_name.starts_with("payload.proteus-") && b_name.ends_with(".tmp"),
            "unexpected temp name: {b_name}"
        );
        // Different random suffixes per call (overwhelmingly likely).
        assert_ne!(
            a_name, b_name,
            "tmp_path_for returned identical names: {a_name} == {b_name}"
        );
    }

    #[test]
    fn parallel_writes_against_same_target_all_succeed() {
        let tmp = TempDir::new("parallel");
        let path = Arc::new(tmp.0.join("shared"));
        let n_threads = 8;
        let handles: Vec<_> = (0..n_threads)
            .map(|i| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    let body = format!("writer-{i}\n");
                    write_atomic(&path, body.as_bytes())
                })
            })
            .collect();
        for h in handles {
            h.join()
                .unwrap()
                .expect("write_atomic must succeed under contention");
        }
        // File is present, contents are one of the writers' bodies.
        let final_bytes = std::fs::read(&*path).unwrap();
        assert!(
            final_bytes.starts_with(b"writer-"),
            "unexpected final contents: {:?}",
            String::from_utf8_lossy(&final_bytes)
        );
        assert!(
            !any_tmp_leaks(&tmp.0),
            "parallel writes leaked .tmp under {}",
            tmp.0.display()
        );
    }

    #[test]
    fn drop_cleans_up_temp_file_when_left_behind() {
        // Simulates the "open succeeded but rename never happened" path:
        // we create the tmp file, hand it to the guard, then drop the guard
        // and assert the file is gone.
        let tmp = TempDir::new("guard");
        let path = tmp.0.join("payload");
        let temp = tmp_path_for(&path).unwrap();
        std::fs::write(&temp, b"orphan").unwrap();
        assert!(temp.exists(), "precondition: tmp file exists");
        {
            let guard = TmpFile(temp.clone());
            drop(guard);
        }
        assert!(
            !temp.exists(),
            "TmpFile drop should have removed {}",
            temp.display()
        );
    }

    #[test]
    fn refuses_to_follow_pre_existing_tmp_symlink() {
        // The TOCTOU defense: if an attacker pre-creates a file at the exact
        // tmp path, O_CREAT|O_EXCL must reject it. Random suffix makes this
        // essentially unguessable in practice; here we force the collision
        // by hand to exercise the EXCL branch.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("excl");
        let path = tmp.0.join("payload");
        let preexisting = tmp_path_for(&path).unwrap();
        let target = tmp.0.join("target");
        std::fs::write(&target, b"attacker").unwrap();
        symlink(&target, &preexisting).unwrap();

        // Drive the EXCL open directly so the test does not depend on
        // catching the exact random suffix `write_atomic` will pick.
        let err = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&preexisting)
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        // Symlink target must not have been clobbered.
        assert_eq!(std::fs::read(&target).unwrap(), b"attacker");
    }
}
