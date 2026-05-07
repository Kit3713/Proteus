// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod bluetooth_cmd;
pub mod completions;
pub mod config_cmd;
pub mod current;
pub mod dhcp;
pub mod diff;
pub mod dns;
pub mod doctor;
pub mod dry_run;
pub mod enterprise_wifi;
pub mod events;
pub mod hostname;
pub mod ipv6;
pub mod kill;
pub mod nft;
pub mod ntp;
pub mod original;
pub mod persona;
pub mod pin;
pub mod portal;
pub mod probe;
pub mod reset;
pub mod resolved;
pub mod revert;
pub mod rf;
pub mod rotate;
pub mod session;
pub mod show_config;
pub mod show_defaults;
pub mod ssid;
pub mod stack;
pub mod status;
pub mod stub;
pub mod timer;
pub mod uninstall;
pub mod unpin;
pub mod watch;
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
use crate::exit;
use crate::state_lock::{self, LockError, StateLockGuard};

/// Acquire the state lock for a mutating command. On contention, returns
/// `Err(exit_code)` so callers can `return Ok(code)` directly without
/// duplicating the error-printing boilerplate. Any other error is bubbled
/// via `anyhow`.
///
/// Issue #126: every mutating command entry point calls this so two
/// concurrent `proteus` processes serialize on `<state-dir>/.lock`.
///
/// Issue #211: lock contention now exits with the dedicated `LOCK_BUSY` (75)
/// code so wrappers can distinguish a retryable contention from an
/// unrecoverable config error (65). The two-and-a-half-line bash retry
/// pattern wrapping `proteus apply` becomes legible:
///
/// ```sh
/// while ! proteus apply --yes; do
///     case $? in 75) sleep 2 ;; *) exit $? ;; esac
/// done
/// ```
pub(crate) fn acquire_state_lock_or_print(
    override_state_path: Option<&Path>,
) -> std::result::Result<StateLockGuard, u8> {
    let path = state_path(override_state_path);
    match state_lock::acquire_for_state_path(&path) {
        Ok(g) => Ok(g),
        Err(LockError::Busy { path }) => {
            eprintln!(
                "proteus: another proteus run holds the state lock at {}; retry shortly",
                path.display()
            );
            Err(exit::LOCK_BUSY)
        }
        Err(e) => {
            eprintln!("proteus: failed to acquire state lock: {e:#}");
            Err(exit::GENERIC_ERROR)
        }
    }
}

pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<()> {
    // Roadmap Milestone 6 — `--format yaml` redirects through the
    // JSON-to-YAML emitter below. Set by `cli::dispatch` when the
    // user passes `--format yaml`. Default (Json) is the existing
    // pretty-printed JSON path.
    if YAML_OUTPUT.load(std::sync::atomic::Ordering::Relaxed) {
        let yaml = serde_to_yaml(value)?;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        use std::io::Write;
        handle.write_all(yaml.as_bytes())?;
        if !yaml.ends_with('\n') {
            handle.write_all(b"\n")?;
        }
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, value)?;
    println!();
    Ok(())
}

/// Roadmap Milestone 6: process-wide YAML-output toggle. Set once by
/// `cli::dispatch` when `--format yaml` is on the command line; every
/// `print_json` call after that emits YAML instead of JSON.
///
/// Atomic rather than thread-local because all readers run on the main
/// thread and the toggle is set exactly once before dispatch enters
/// the per-subcommand match arms. An atomic bool is a single-byte read
/// per `print_json` call — strictly cheaper than a thread-local check.
pub(crate) static YAML_OUTPUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Serialize `value` through `serde_json::Value` and convert into a
/// minimal YAML block-style document. The conversion intentionally
/// covers only the JSON shape Proteus's reports already use:
/// objects → mappings, arrays → sequences, primitives → scalars.
/// No anchors, tags, flow style, or multi-document streams.
fn serde_to_yaml<T: Serialize>(value: &T) -> Result<String> {
    let v: serde_json::Value = serde_json::to_value(value)?;
    let mut out = String::with_capacity(256);
    write_yaml_value(&mut out, &v, 0, true);
    Ok(out)
}

fn write_yaml_value(out: &mut String, v: &serde_json::Value, indent: usize, top_level: bool) {
    match v {
        serde_json::Value::Null => out.push_str("null\n"),
        serde_json::Value::Bool(b) => {
            out.push_str(if *b { "true" } else { "false" });
            out.push('\n');
        }
        serde_json::Value::Number(n) => {
            out.push_str(&n.to_string());
            out.push('\n');
        }
        serde_json::Value::String(s) => {
            out.push_str(&yaml_scalar(s));
            out.push('\n');
        }
        serde_json::Value::Array(arr) => write_yaml_array(out, arr, indent, top_level),
        serde_json::Value::Object(map) => write_yaml_object(out, map, indent, top_level),
    }
}

fn write_yaml_array(
    out: &mut String,
    arr: &[serde_json::Value],
    indent: usize,
    top_level: bool,
) {
    if arr.is_empty() {
        out.push_str("[]\n");
        return;
    }
    if !top_level {
        out.push('\n');
    }
    for item in arr {
        push_indent(out, indent);
        out.push_str("- ");
        match item {
            serde_json::Value::Object(map) if !map.is_empty() => {
                let mut first = true;
                for (k, v) in map {
                    if !first {
                        push_indent(out, indent + 1);
                    }
                    first = false;
                    out.push_str(&yaml_key(k));
                    out.push(':');
                    if matches!(v, serde_json::Value::Object(m) if !m.is_empty())
                        || matches!(v, serde_json::Value::Array(a) if !a.is_empty())
                    {
                        write_yaml_value(out, v, indent + 2, false);
                    } else {
                        out.push(' ');
                        write_yaml_value(out, v, indent + 1, false);
                    }
                }
            }
            serde_json::Value::Array(inner) if !inner.is_empty() => {
                write_yaml_array(out, inner, indent + 1, false);
            }
            _ => write_yaml_value(out, item, indent + 1, false),
        }
    }
}

fn write_yaml_object(
    out: &mut String,
    map: &serde_json::Map<String, serde_json::Value>,
    indent: usize,
    top_level: bool,
) {
    if map.is_empty() {
        out.push_str("{}\n");
        return;
    }
    if !top_level {
        out.push('\n');
    }
    for (k, v) in map {
        push_indent(out, indent);
        out.push_str(&yaml_key(k));
        out.push(':');
        if matches!(v, serde_json::Value::Object(m) if !m.is_empty())
            || matches!(v, serde_json::Value::Array(a) if !a.is_empty())
        {
            write_yaml_value(out, v, indent + 1, false);
        } else {
            out.push(' ');
            write_yaml_value(out, v, indent, false);
        }
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// Quote keys that would otherwise be ambiguous (contain `:`, `#`,
/// start with a sigil, or look like a YAML keyword). Plain identifiers
/// pass through unquoted.
fn yaml_key(s: &str) -> String {
    if needs_quoting(s) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Quote string scalars that could be misread as numbers, booleans, or
/// YAML control characters. Plain text passes through unquoted to
/// keep the rendered output readable.
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if needs_quoting(s) || looks_like_other_scalar(s) {
        return format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    }
    s.to_string()
}

fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let first = s.chars().next().unwrap();
    if matches!(first, '!' | '&' | '*' | '@' | '`' | '%' | '|' | '>' | '?' | '#' | '-' | '[' | ']' | '{' | '}' | ',' | '"' | '\'') {
        return true;
    }
    s.contains(':') || s.contains('#') || s.contains('\n') || s.starts_with(' ') || s.ends_with(' ')
}

fn looks_like_other_scalar(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "~"
    ) || s.parse::<f64>().is_ok()
}

#[cfg(test)]
mod yaml_emitter_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitives_emit_their_yaml_form() {
        assert_eq!(serde_to_yaml(&json!(null)).unwrap(), "null\n");
        assert_eq!(serde_to_yaml(&json!(true)).unwrap(), "true\n");
        assert_eq!(serde_to_yaml(&json!(42)).unwrap(), "42\n");
        // Strings that look like other scalars get quoted.
        assert_eq!(serde_to_yaml(&json!("true")).unwrap(), "\"true\"\n");
        assert_eq!(serde_to_yaml(&json!("42")).unwrap(), "\"42\"\n");
        // Plain text does not.
        assert_eq!(serde_to_yaml(&json!("hello")).unwrap(), "hello\n");
    }

    #[test]
    fn flat_object_emits_block_mapping() {
        let v = json!({ "a": 1, "b": "two" });
        let y = serde_to_yaml(&v).unwrap();
        assert!(y.contains("a: 1"), "{y}");
        assert!(y.contains("b: two"), "{y}");
    }

    #[test]
    fn nested_object_indents_children() {
        let v = json!({ "outer": { "inner": 7 } });
        let y = serde_to_yaml(&v).unwrap();
        assert!(y.contains("outer:\n"), "{y}");
        assert!(y.contains("  inner: 7"), "{y}");
    }

    #[test]
    fn array_of_scalars_uses_dash_lines() {
        let v = json!(["a", "b", "c"]);
        let y = serde_to_yaml(&v).unwrap();
        assert_eq!(y, "- a\n- b\n- c\n");
    }

    #[test]
    fn array_of_objects_inlines_first_key_after_dash() {
        let v = json!([{ "k": "v1" }, { "k": "v2" }]);
        let y = serde_to_yaml(&v).unwrap();
        assert!(y.contains("- k: v1"));
        assert!(y.contains("- k: v2"));
    }

    #[test]
    fn empty_collections_use_flow_style() {
        assert_eq!(serde_to_yaml(&json!({})).unwrap(), "{}\n");
        assert_eq!(serde_to_yaml(&json!([])).unwrap(), "[]\n");
    }

    #[test]
    fn keys_with_colons_get_quoted() {
        let v = json!({ "k:with:colons": "v" });
        let y = serde_to_yaml(&v).unwrap();
        assert!(y.contains("\"k:with:colons\":"), "{y}");
    }
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
///
/// Issue #206-H: a path without a file-name component is a programmer
/// error — `write_atomic` only ever gets called with concrete leaf paths.
/// Bail loudly rather than silently default to a generic `"file"` stem
/// that would shadow whatever Proteus thought it was writing.
fn tmp_path_for(path: &Path) -> Result<PathBuf> {
    let mut rand = [0u8; 8];
    getrandom::getrandom(&mut rand).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    let suffix: String = rand.iter().map(|b| format!("{b:02x}")).collect();
    let base = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "write_atomic target has no file-name component: {}",
                path.display()
            )
        })?;
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
    use std::fs::OpenOptions;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::io::AsRawFd;
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

    #[test]
    fn acquire_state_lock_or_print_returns_lock_busy_when_busy() {
        // Issue #126 / #211: a second mutating-command entry point must bail
        // with LOCK_BUSY (75) rather than blocking when another process holds
        // the lock. Simulate the "another process" by taking the kernel flock
        // through a separate fd, then call the helper.
        //
        // Serialize with the shared test mutex so we don't race with the
        // lock-module's own tests touching the process-wide HELD flag.
        let _serial = state_lock::test_serial_guard();

        let dir = std::env::temp_dir().join(format!("proteus-helper-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");
        let lock = dir.join(".lock");

        let foreign = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .unwrap();
        let rc = unsafe { libc::flock(foreign.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "test setup: foreign flock should succeed");

        let result = acquire_state_lock_or_print(Some(&state));
        assert_eq!(
            result.err(),
            Some(exit::LOCK_BUSY),
            "busy lock must surface LOCK_BUSY (75)"
        );

        unsafe {
            libc::flock(foreign.as_raw_fd(), libc::LOCK_UN);
        }
        drop(foreign);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
