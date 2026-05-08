// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus diff` data model + comparison logic.
//!
//! Three independent comparisons:
//!
//!   1. Config vs `Config::default()` — flatten both to dotted-key TOML and
//!      report keys whose rendered values differ.
//!   2. Managed-file drift — for each known Proteus-managed `/etc/` path that
//!      exists, parse the `# sha256:...` header and recompute the body's
//!      sha. Mismatches surface as an edit-detection / tamper-hint signal:
//!      something (a manual edit, another tool's writer) changed the file.
//!      Issue #234: this is *not* an integrity guarantee — both header and
//!      body live in the same root-owned file, so anything with write
//!      access can rewrite the header to match a tampered body. The check
//!      catches honest drift; it is not a defence against an active
//!      adversary who already has root.
//!   3. State summary — light projection of `state.json` for the human + JSON
//!      report (originals presence, managed connections, pinned interfaces,
//!      most recent rotation).
//!
//! Pure read-only. Never writes; never asks for root.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;
use crate::crypto::sha256;
use crate::state::State;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub config_drift: Vec<ConfigDrift>,
    pub managed_file_drift: Vec<FileDrift>,
    pub state_summary: StateSummary,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ConfigDrift {
    pub key: String,
    pub current: String,
    pub default: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileDrift {
    pub path: String,
    /// SHA-256 hex from the file's `# sha256:` header, or `None` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_sha: Option<String>,
    /// SHA-256 hex of the body actually on disk.
    pub actual_sha: String,
    pub drift: bool,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
pub struct StateSummary {
    pub originals_cached: bool,
    pub managed_connections: Vec<String>,
    pub pinned_interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rotation_at: Option<String>,
}

// --- Build ----------------------------------------------------------------

pub fn build_report(config: &Config, state: Option<&State>, etc_root: &Path) -> Report {
    Report {
        schema_version: SCHEMA_VERSION,
        config_drift: compute_config_drift(config),
        managed_file_drift: compute_managed_file_drift_with_state(etc_root, state),
        state_summary: summarize_state(state),
    }
}

// --- Config drift ---------------------------------------------------------

const UNSET: &str = "(unset)";

/// Compare `current` against `Config::default()` by serializing both to TOML
/// and walking the parsed tree. We use TOML rather than serde_json so the
/// keys read the same as `proteus show-config` output, e.g.
/// `mac.rotation_interval = "1h"` → key `mac.rotation_interval`.
///
/// `Option<T>` fields with serde-skip-serializing-on-`None` only appear in
/// the flat map when `Some`; the absent side is reported as `(unset)`.
pub fn compute_config_drift(current: &Config) -> Vec<ConfigDrift> {
    let cur_flat = flatten_config(current).unwrap_or_default();
    let def_flat = flatten_config(&Config::default()).unwrap_or_default();
    let all_keys: std::collections::BTreeSet<&String> =
        cur_flat.keys().chain(def_flat.keys()).collect();
    let mut out = Vec::new();
    for key in all_keys {
        let cur_val = cur_flat.get(key).map(String::as_str).unwrap_or(UNSET);
        let def_val = def_flat.get(key).map(String::as_str).unwrap_or(UNSET);
        if cur_val != def_val {
            out.push(ConfigDrift {
                key: key.clone(),
                current: cur_val.to_string(),
                default: def_val.to_string(),
            });
        }
    }
    out
}

fn flatten_config(cfg: &Config) -> Option<std::collections::BTreeMap<String, String>> {
    let s = toml::to_string(cfg).ok()?;
    let v: toml::Value = toml::from_str(&s).ok()?;
    let mut out = std::collections::BTreeMap::new();
    flatten_value("", &v, &mut out);
    Some(out)
}

fn flatten_value(
    prefix: &str,
    v: &toml::Value,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    match v {
        toml::Value::Table(t) => {
            for (k, child) in t {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_value(&next, child, out);
            }
        }
        // Arrays render as one-line; we never recurse into them. The config
        // schema only uses arrays for OUI pools and probe endpoints, where
        // "the whole list differs" is the right granularity.
        leaf => {
            out.insert(prefix.to_string(), render_leaf(leaf));
        }
    }
}

fn render_leaf(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(render_leaf).collect();
            format!("[{}]", parts.join(", "))
        }
        toml::Value::Table(_) => "(table)".to_string(),
    }
}

// --- Managed-file drift ---------------------------------------------------

/// Known Proteus-managed paths under `/etc/` that a `proteus diff` should
/// inspect. Paths missing on disk are silently skipped (not "drift" — just
/// "not yet applied"). Paths present without the managed header are flagged
/// so manual installs don't silently take over Proteus's slot.
fn known_managed_paths(etc_root: &Path) -> Vec<PathBuf> {
    let join = |suffix: &str| etc_root.join(suffix.trim_start_matches('/'));
    let mut out = vec![
        join("/etc/sysctl.d/95-proteus.conf"),
        join("/etc/sysctl.d/96-proteus-ipv6.conf"),
        join("/etc/systemd/timesyncd.conf.d/10-proteus.conf"),
        join("/etc/NetworkManager/dispatcher.d/01-proteus"),
    ];
    // Glob-style: enumerate /etc/systemd/resolved.conf.d/10-proteus-*.conf
    // and /etc/systemd/system/proteus-*.{timer,service} plus any .d/*.conf
    // drop-ins under those.
    if let Ok(entries) = std::fs::read_dir(etc_root.join("etc/systemd/resolved.conf.d")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("10-proteus-") && s.ends_with(".conf") {
                out.push(entry.path());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(etc_root.join("etc/systemd/system")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy().into_owned();
            let path = entry.path();
            if (s.starts_with("proteus-") && (s.ends_with(".timer") || s.ends_with(".service")))
                && path.is_file()
            {
                out.push(path.clone());
            }
            // Recurse one level into proteus-*.{timer,service}.d for drop-ins.
            if s.starts_with("proteus-") && s.ends_with(".d") && path.is_dir() {
                if let Ok(sub) = std::fs::read_dir(&path) {
                    for s_entry in sub.flatten() {
                        let n = s_entry.file_name().to_string_lossy().into_owned();
                        if n.ends_with(".conf") {
                            out.push(s_entry.path());
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Roadmap N12.10: cap any single managed file we read for `proteus diff` at
/// 64 MiB. Proteus drop-ins are routinely <2 KiB; a multi-megabyte file is a
/// strong signal that the path was clobbered (or symlinked) into something we
/// shouldn't fully buffer. `read_capped` reads up to `MAX_DIFF_FILE_BYTES + 1`
/// so we can detect the over-cap case without allocating an unbounded body.
pub const MAX_DIFF_FILE_BYTES: u64 = 64 * 1024 * 1024;

fn read_capped(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    // Read up to `cap + 1` bytes; if the +1 byte materializes the file is
    // larger than our budget and we surface FileTooLarge.
    let mut limited = f.take(cap.saturating_add(1));
    let mut buf = Vec::new();
    limited.read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            format!(
                "{} exceeds the {}-byte diff cap; refusing to fully buffer",
                path.display(),
                cap
            ),
        ));
    }
    Ok(buf)
}

pub fn compute_managed_file_drift(etc_root: &Path) -> Vec<FileDrift> {
    compute_managed_file_drift_with_state(etc_root, None)
}

/// Roadmap NMOD.3: cross-reference `state.json`'s applied-originals markers
/// against the filesystem. The previous `compute_managed_file_drift`
/// silently skipped paths that were missing from disk — which means a
/// Proteus-managed file the operator deleted (manually or via a packaged
/// uninstall that didn't go through `proteus`) wouldn't surface in
/// `proteus diff` at all. Now, when `state` indicates we've applied that
/// surface (sysctls touched, IPv6 originals captured, NM dispatcher writers
/// run, timesyncd timesync'd), the absence of the corresponding file is
/// reported as a missing-drift entry.
pub fn compute_managed_file_drift_with_state(
    etc_root: &Path,
    state: Option<&State>,
) -> Vec<FileDrift> {
    let mut out = Vec::new();
    let expected_present: std::collections::BTreeSet<PathBuf> =
        expected_managed_paths(etc_root, state);
    let known = known_managed_paths(etc_root);
    let mut visited: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for path in known {
        visited.insert(path.clone());
        let bytes = match read_capped(&path, MAX_DIFF_FILE_BYTES) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // NMOD.3: only surface this as drift if state.json said we
                // ever applied this surface. Otherwise it's "not yet
                // applied", which is the original silently-skip behaviour.
                if expected_present.contains(&path) {
                    out.push(FileDrift {
                        path: path.display().to_string(),
                        expected_sha: None,
                        actual_sha: String::new(),
                        drift: true,
                        reason: "missing on disk (state.json says proteus applied this)"
                            .to_string(),
                    });
                }
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::FileTooLarge => {
                // N12.10: surface the cap as drift so the operator sees
                // exactly which file blew the budget instead of a silent skip.
                out.push(FileDrift {
                    path: path.display().to_string(),
                    expected_sha: None,
                    actual_sha: String::new(),
                    drift: true,
                    reason: format!("file exceeds {}-byte diff cap: {e}", MAX_DIFF_FILE_BYTES),
                });
                continue;
            }
            Err(e) => {
                tracing::warn!("skip {}: {e}", path.display());
                continue;
            }
        };
        out.push(check_file(&path, &bytes));
    }
    // NMOD.3 second half: any expected-by-state path that wasn't in the
    // known glob (e.g. a resolved drop-in whose name no longer matches the
    // current naming convention) is reported missing too. Avoids hiding
    // stragglers behind the fact that the directory walk found nothing.
    for path in &expected_present {
        if visited.contains(path) {
            continue;
        }
        if path.exists() {
            // Path is present but didn't surface from the glob walk — fall
            // through and check it directly.
            match read_capped(path, MAX_DIFF_FILE_BYTES) {
                Ok(b) => out.push(check_file(path, &b)),
                Err(e) => tracing::warn!("nmod.3 fallback skip {}: {e}", path.display()),
            }
        } else {
            out.push(FileDrift {
                path: path.display().to_string(),
                expected_sha: None,
                actual_sha: String::new(),
                drift: true,
                reason: "missing on disk (state.json says proteus applied this)".to_string(),
            });
        }
    }
    out
}

/// Build the set of `/etc/` paths that `state.json` implies should currently
/// exist. The mapping is keyed on the `Originals` markers `proteus apply`
/// writes when it first touches each surface — every entry here corresponds
/// to a real on-disk drop-in that `apply` lays down once the marker is set.
fn expected_managed_paths(
    etc_root: &Path,
    state: Option<&State>,
) -> std::collections::BTreeSet<PathBuf> {
    let mut out = std::collections::BTreeSet::new();
    let Some(state) = state else { return out };
    let join = |suffix: &str| etc_root.join(suffix.trim_start_matches('/'));
    if !state.originals.sysctls.is_empty() {
        out.insert(join("/etc/sysctl.d/95-proteus.conf"));
    }
    if !state.originals.ipv6.is_empty() {
        out.insert(join("/etc/sysctl.d/96-proteus-ipv6.conf"));
    }
    if !state.originals.connections.is_empty() {
        // `apply` lays the dispatcher down the first time any connection is
        // managed. If state has connection originals, we expect the script
        // to be present too.
        out.insert(join("/etc/NetworkManager/dispatcher.d/01-proteus"));
    }
    out
}

/// Public for tests: classify one file's contents into a `FileDrift`.
pub fn check_file(path: &Path, bytes: &[u8]) -> FileDrift {
    let text = String::from_utf8_lossy(bytes);
    match parse_managed_header(&text) {
        Some(parsed) => {
            let actual = sha256::hex_digest(parsed.body.as_bytes());
            if actual == parsed.expected_sha {
                FileDrift {
                    path: path.display().to_string(),
                    expected_sha: Some(parsed.expected_sha),
                    actual_sha: actual,
                    drift: false,
                    reason: "sha matches header".to_string(),
                }
            } else {
                FileDrift {
                    path: path.display().to_string(),
                    expected_sha: Some(parsed.expected_sha),
                    actual_sha: actual,
                    drift: true,
                    reason: "manual edit detected".to_string(),
                }
            }
        }
        None => {
            // File exists but no managed header. Flag — could be a stale
            // hand-installed copy from before Proteus, or third-party.
            let actual = sha256::hex_digest(bytes);
            FileDrift {
                path: path.display().to_string(),
                expected_sha: None,
                actual_sha: actual,
                drift: true,
                reason: "manually-installed (not managed by proteus)".to_string(),
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedHeader {
    pub expected_sha: String,
    pub body: String,
}

/// Parse `# sha256:<hex>` from a Proteus-managed file's header.
///
/// The header format documented in `wiki/internals.md`:
///
/// ```text
/// # managed by proteus v0.1.0
/// # do not edit; manage via /etc/proteus/config.toml or `proteus apply`
/// # sha256:abc123...  (body sha)
/// <body...>
/// ```
///
/// We accept a more relaxed match: any leading run of comment lines (`#` or
/// `;` for sysctl-style) is the header; the first comment line containing
/// `sha256:<hex64>` provides the expected hash; everything after the last
/// header line is the body. The "managed by proteus" tag must appear in the
/// header so we don't accidentally claim arbitrary commented files as ours.
pub fn parse_managed_header(text: &str) -> Option<ParsedHeader> {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut body_start: Option<usize> = None;
    let mut byte_offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            header_lines.push(line.trim_end_matches('\n').trim_end_matches('\r'));
            byte_offset += line.len();
        } else if line.trim().is_empty() && body_start.is_none() {
            // Tolerate blank lines inside the header — some drop-ins have them.
            byte_offset += line.len();
        } else {
            body_start = Some(byte_offset);
            break;
        }
    }
    let body_start = body_start.unwrap_or(byte_offset);

    let mut expected: Option<String> = None;
    let mut managed_marker = false;
    for line in &header_lines {
        if line.contains("managed by proteus") {
            managed_marker = true;
        }
        if let Some(idx) = line.find("sha256:") {
            let rest = &line[idx + "sha256:".len()..];
            let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() == 64 {
                expected = Some(hex.to_ascii_lowercase());
            }
        }
    }
    if !managed_marker {
        return None;
    }
    let expected = expected?;
    Some(ParsedHeader {
        expected_sha: expected,
        body: text[body_start..].to_string(),
    })
}

// --- State summary --------------------------------------------------------

pub fn summarize_state(state: Option<&State>) -> StateSummary {
    let Some(state) = state else {
        return StateSummary::default();
    };
    let originals_cached = !state.original_macs.is_empty()
        || state.originals.hostname.is_some()
        || !state.originals.bluetooth_aliases.is_empty();
    let managed_connections: Vec<String> = state.managed.connections.keys().cloned().collect();
    let pinned_interfaces: Vec<String> = state
        .managed
        .interfaces
        .iter()
        .filter_map(|(name, rec)| rec.pinned.as_ref().map(|_| name.clone()))
        .collect();
    let last_rotation_at: Option<String> = state
        .managed
        .interfaces
        .values()
        .filter_map(|r| r.last_rotated.clone())
        .max();
    StateSummary {
        originals_cached,
        managed_connections,
        pinned_interfaces,
        last_rotation_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_drift_detects_changed_value() {
        let mut cfg = Config::default();
        cfg.mac.rotation_interval = "1h".to_string();
        let drifts = compute_config_drift(&cfg);
        let mac_drift = drifts
            .iter()
            .find(|d| d.key == "mac.rotation_interval")
            .expect("expected rotation_interval drift");
        assert_eq!(mac_drift.current, "1h");
        assert_eq!(mac_drift.default, "2h");
        // Untouched keys must not appear.
        assert!(
            drifts
                .iter()
                .all(|d| d.key != "dns.strip_edns_client_subnet")
        );
    }

    #[test]
    fn config_drift_empty_when_unchanged() {
        let cfg = Config::default();
        assert!(compute_config_drift(&cfg).is_empty());
    }

    #[test]
    fn parse_managed_header_extracts_sha_and_body() {
        let text = "# managed by proteus v0.1.0\n\
                    # do not edit; manage via /etc/proteus/config.toml\n\
                    # sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n\
                    body line 1\n\
                    body line 2\n";
        let parsed = parse_managed_header(text).expect("header should parse");
        assert_eq!(
            parsed.expected_sha,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(parsed.body, "body line 1\nbody line 2\n");
    }

    #[test]
    fn parse_managed_header_returns_none_without_marker() {
        // Generic comment file with a sha line — must not be claimed as ours.
        let text = "# random script\n# sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nbody\n";
        assert!(parse_managed_header(text).is_none());
    }

    #[test]
    fn parse_managed_header_returns_none_without_sha() {
        let text = "# managed by proteus v0.1.0\n# do not edit\nbody\n";
        assert!(parse_managed_header(text).is_none());
    }

    #[test]
    fn check_file_no_drift_when_sha_matches() {
        let body = "kernel.something = 1\n";
        let sha = sha256::hex_digest(body.as_bytes());
        let text = format!("# managed by proteus v0.1.0\n# do not edit\n# sha256:{sha}\n{body}");
        let drift = check_file(Path::new("/etc/sysctl.d/95-proteus.conf"), text.as_bytes());
        assert!(!drift.drift, "expected no drift, got: {drift:?}");
        assert_eq!(drift.expected_sha.as_deref(), Some(sha.as_str()));
    }

    #[test]
    fn check_file_drift_on_body_edit() {
        let body = "kernel.something = 1\n";
        let sha = sha256::hex_digest(body.as_bytes());
        // Body has been tampered with after the header was written.
        let tampered = "kernel.something = 2\n";
        let text =
            format!("# managed by proteus v0.1.0\n# do not edit\n# sha256:{sha}\n{tampered}");
        let drift = check_file(Path::new("/etc/sysctl.d/95-proteus.conf"), text.as_bytes());
        assert!(drift.drift);
        assert_eq!(drift.reason, "manual edit detected");
    }

    #[test]
    fn check_file_unmanaged_when_header_missing() {
        let text = b"# random local edit\nfoo=bar\n";
        let drift = check_file(Path::new("/etc/sysctl.d/95-proteus.conf"), text);
        assert!(drift.drift);
        assert!(drift.expected_sha.is_none());
        assert_eq!(drift.reason, "manually-installed (not managed by proteus)");
    }

    /// Roadmap N12.10: a managed-path file larger than the 64 MiB diff cap
    /// must surface as drift with a clear reason rather than the previous
    /// unbounded `std::fs::read` that would happily allocate gigabytes for
    /// a clobbered drop-in. We use a tiny cap here to keep the test fast;
    /// the production constant is `MAX_DIFF_FILE_BYTES` (64 MiB).
    #[test]
    fn read_capped_rejects_oversize_file() {
        let tmp = tempdir_path();
        let path = tmp.join("big.conf");
        // 8 KiB of data, cap at 1 KiB — must surface FileTooLarge.
        std::fs::write(&path, vec![b'x'; 8 * 1024]).unwrap();
        let err = read_capped(&path, 1024).expect_err("should reject oversize");
        assert_eq!(err.kind(), std::io::ErrorKind::FileTooLarge);
        // A small file under the cap reads cleanly.
        let small = tmp.join("small.conf");
        std::fs::write(&small, b"hello").unwrap();
        let bytes = read_capped(&small, 1024).unwrap();
        assert_eq!(bytes, b"hello");
        // Boundary: file exactly at the cap reads cleanly.
        let exact = tmp.join("exact.conf");
        std::fs::write(&exact, vec![b'y'; 1024]).unwrap();
        let bytes = read_capped(&exact, 1024).unwrap();
        assert_eq!(bytes.len(), 1024);
        // Boundary: file one byte over the cap fails.
        let over = tmp.join("over.conf");
        std::fs::write(&over, vec![b'z'; 1025]).unwrap();
        let err = read_capped(&over, 1024).expect_err("should reject 1-over");
        assert_eq!(err.kind(), std::io::ErrorKind::FileTooLarge);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn known_paths_walks_glob_directories() {
        // Build a fake /etc/ tree under tmp.
        let tmp = tempdir_path();
        let resolved_d = tmp.join("etc/systemd/resolved.conf.d");
        std::fs::create_dir_all(&resolved_d).unwrap();
        std::fs::write(resolved_d.join("10-proteus-no-ecs.conf"), b"x").unwrap();
        std::fs::write(resolved_d.join("99-other-tool.conf"), b"x").unwrap();

        let sys_d = tmp.join("etc/systemd/system/proteus-rotate.timer.d");
        std::fs::create_dir_all(&sys_d).unwrap();
        std::fs::write(sys_d.join("override.conf"), b"x").unwrap();

        let paths = known_managed_paths(&tmp);
        // Roadmap P3: `Path::file_name()` returns `None` for paths that end
        // in `..` or a trailing separator. Skip those rather than panic so
        // a future caller passing such a path doesn't take the test down.
        let names: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert!(
            names.contains(&"10-proteus-no-ecs.conf".to_string()),
            "missing resolved drop-in: {names:?}"
        );
        assert!(
            names.contains(&"override.conf".to_string()),
            "missing systemd drop-in: {names:?}"
        );
        assert!(
            !names.contains(&"99-other-tool.conf".to_string()),
            "non-proteus drop-in must not be claimed: {names:?}"
        );
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Roadmap NMOD.3: when state.json indicates a surface was applied
    /// but the corresponding on-disk file was deleted out from under
    /// proteus, the drift report must surface a "missing" entry.
    #[test]
    fn drift_reports_missing_file_when_state_says_applied() {
        let tmp = tempdir_path();
        // No on-disk files at all.
        let mut state = State::default();
        state
            .originals
            .sysctls
            .insert("net.ipv4.ip_forward".to_string(), "1".to_string());
        let drifts = compute_managed_file_drift_with_state(&tmp, Some(&state));
        // The expected sysctl drop-in is missing — it must show up as drift.
        assert!(
            drifts.iter().any(|d| d.path.ends_with("95-proteus.conf")
                && d.drift
                && d.reason.contains("missing on disk")),
            "expected missing-sysctl drift, got {drifts:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Without a state hint, a missing managed file is still silently
    /// skipped — that's the documented "not yet applied" path.
    #[test]
    fn drift_silently_skips_missing_file_without_state_hint() {
        let tmp = tempdir_path();
        let drifts = compute_managed_file_drift_with_state(&tmp, None);
        assert!(
            drifts.is_empty(),
            "without state hint, missing files should be silently skipped, got {drifts:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn state_summary_reports_originals_and_pins() {
        use crate::state::{InterfaceRecord, ManagedState};

        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".to_string(), "aa:bb:cc:dd:ee:ff".to_string());
        let mut managed = ManagedState::default();
        managed.interfaces.insert(
            "wlan0".to_string(),
            InterfaceRecord {
                current_mac: Some("11:22:33:44:55:66".to_string()),
                pinned: Some("11:22:33:44:55:66".to_string()),
                last_rotated: Some("2026-05-07T12:34:56Z".to_string()),
                rotation_count: 1,
            },
        );
        state.managed = managed;

        let summary = summarize_state(Some(&state));
        assert!(summary.originals_cached);
        assert_eq!(summary.pinned_interfaces, vec!["wlan0".to_string()]);
        assert_eq!(
            summary.last_rotation_at.as_deref(),
            Some("2026-05-07T12:34:56Z")
        );
    }

    fn tempdir_path() -> PathBuf {
        // Tiny tmpdir helper to avoid pulling in the tempfile crate.
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        p.push(format!("proteus-diff-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
