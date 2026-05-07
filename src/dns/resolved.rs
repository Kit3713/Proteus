// SPDX-License-Identifier: GPL-3.0-or-later

//! `systemd-resolved` mDNS + LLMNR drop-in (Milestone 4a).
//!
//! Sibling to `dns/apply.rs` (which owns the ECS-strip drop-in). This module
//! handles a separate file under the same `resolved.conf.d/` dir so the two
//! knobs can be reverted independently — the operator may want to keep
//! ECS-strip on while restoring mDNS/LLMNR for a printer-discovery session.
//!
//! The detect-and-defer policy is identical to `dns::detect_defer`: any
//! foreign drop-in / non-resolved `/etc/resolv.conf` / running third-party
//! resolver causes Proteus to bow out. The `dns::Paths` and
//! `dns::detect_defer_system` helpers are reused verbatim.

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::Paths;
use super::apply::sha256_hex;
use crate::commands;
use crate::config::ResolvedConfig;
use crate::version;

/// Single Proteus-owned filename in `resolved.conf.d/`. Numbered higher than
/// the ECS-strip drop-in (`10-proteus-no-ecs.conf`) but with the same
/// `10-proteus-` prefix so the existing detect-and-defer guard recognises it.
pub const PROTEUS_RESOLVED_DROPIN_NAME: &str = "10-proteus-mdns-llmnr.conf";

/// Body of the drop-in **without** the management header. Only the lines
/// that the user's chosen knobs require are emitted, so a one-knob config
/// produces a one-line body.
pub fn render_body(cfg: &ResolvedConfig) -> String {
    let mut out = String::from("[Resolve]\n");
    if cfg.mdns_off {
        out.push_str("MulticastDNS=no\n");
    }
    if cfg.llmnr_off {
        out.push_str("LLMNR=no\n");
    }
    out
}

/// Whether the resolved knob has anything to do at all. When both bools are
/// off the apply path treats the feature as `idle` and never writes a file.
pub fn is_active(cfg: &ResolvedConfig) -> bool {
    cfg.mdns_off || cfg.llmnr_off
}

/// Full file contents: managed-file header + `# sha256:<body>` + body. SHA
/// covers the body only so a future `proteus diff` can recompute it from the
/// rendered body without parsing the header.
pub fn render_dropin(cfg: &ResolvedConfig) -> String {
    let body = render_body(cfg);
    let sha = sha256_hex(body.as_bytes());
    format!(
        "# managed by proteus v{version}\n# do not edit; manage via /etc/proteus/config.toml or `proteus resolved apply`\n# sha256:{sha}\n{body}",
        version = version::VERSION,
    )
}

/// Path to the drop-in given a `Paths` layout. `Paths::system_default()`
/// resolves to `/etc/systemd/resolved.conf.d/10-proteus-mdns-llmnr.conf`.
pub fn dropin_path(paths: &Paths) -> PathBuf {
    paths
        .resolved_dropin_dir()
        .join(PROTEUS_RESOLVED_DROPIN_NAME)
}

/// Atomically write the drop-in. Creates `resolved.conf.d/` if missing.
pub fn write_dropin(paths: &Paths, cfg: &ResolvedConfig) -> Result<PathBuf> {
    let dir = paths.resolved_dropin_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating drop-in dir {}", dir.display()))?;
    let path = dropin_path(paths);
    let body = render_dropin(cfg);
    commands::write_atomic(&path, body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Remove the drop-in if present. Idempotent.
pub fn remove_dropin(paths: &Paths) -> Result<bool> {
    let path = dropin_path(paths);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow::Error::new(e).context(format!("removing {}", path.display()))),
    }
}

/// True iff the drop-in is on disk right now.
pub fn dropin_present(paths: &Paths) -> bool {
    dropin_path(paths).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(mdns: bool, llmnr: bool) -> ResolvedConfig {
        ResolvedConfig {
            mdns_off: mdns,
            llmnr_off: llmnr,
        }
    }

    #[test]
    fn render_body_with_both_off_emits_both_lines() {
        let body = render_body(&cfg(true, true));
        assert!(body.contains("[Resolve]"));
        assert!(body.contains("MulticastDNS=no"));
        assert!(body.contains("LLMNR=no"));
    }

    #[test]
    fn render_body_only_mdns_omits_llmnr() {
        let body = render_body(&cfg(true, false));
        assert!(body.contains("MulticastDNS=no"));
        assert!(!body.contains("LLMNR=no"));
    }

    #[test]
    fn render_body_only_llmnr_omits_mdns() {
        let body = render_body(&cfg(false, true));
        assert!(!body.contains("MulticastDNS=no"));
        assert!(body.contains("LLMNR=no"));
    }

    #[test]
    fn render_body_both_off_emits_section_only() {
        // Empty knob set means we should not stamp a stray DNS=, etc.
        let body = render_body(&cfg(false, false));
        assert_eq!(body, "[Resolve]\n");
    }

    #[test]
    fn is_active_reflects_at_least_one_knob_on() {
        assert!(!is_active(&cfg(false, false)));
        assert!(is_active(&cfg(true, false)));
        assert!(is_active(&cfg(false, true)));
        assert!(is_active(&cfg(true, true)));
    }

    #[test]
    fn render_dropin_includes_sha_of_body() {
        let c = cfg(true, true);
        let body = render_body(&c);
        let expected = sha256_hex(body.as_bytes());
        let full = render_dropin(&c);
        assert!(full.contains(&format!("sha256:{expected}")));
        assert!(full.contains("# managed by proteus v"));
        assert!(full.ends_with(&body));
    }

    #[test]
    fn dropin_path_uses_proteus_filename() {
        let paths = Paths::default();
        let p = dropin_path(&paths);
        assert!(p.ends_with(PROTEUS_RESOLVED_DROPIN_NAME));
    }

    #[test]
    fn dropin_filename_uses_managed_prefix() {
        // The detect-and-defer guard skips files starting with `10-proteus-`,
        // so changing the name without updating the prefix would cause apply
        // to immediately defer to "its own" drop-in.
        assert!(PROTEUS_RESOLVED_DROPIN_NAME.starts_with(super::super::PROTEUS_DROPIN_PREFIX));
    }

    #[test]
    fn write_then_remove_round_trips() {
        let root = crate::testing::TempRoot::new("resolved");
        let paths = Paths::rooted_at(&root.path);
        let c = cfg(true, true);

        // Pre-condition: no file.
        assert!(!dropin_present(&paths));

        // Write.
        let written = write_dropin(&paths, &c).expect("write");
        assert!(written.ends_with(PROTEUS_RESOLVED_DROPIN_NAME));
        assert!(dropin_present(&paths));

        // Contents include the managed-file header.
        let bytes = std::fs::read_to_string(&written).expect("read");
        assert!(bytes.contains("# managed by proteus"));
        assert!(bytes.contains("MulticastDNS=no"));

        // Idempotent remove: first call returns true, second false.
        assert!(remove_dropin(&paths).expect("remove"));
        assert!(!dropin_present(&paths));
        assert!(!remove_dropin(&paths).expect("remove-again"));
    }

    #[test]
    fn render_dropin_is_byte_stable_for_same_inputs() {
        let c = cfg(true, true);
        let a = render_dropin(&c);
        let b = render_dropin(&c);
        assert_eq!(a, b);
    }

    #[test]
    fn render_dropin_changes_when_knobs_change() {
        let a = render_dropin(&cfg(true, true));
        let b = render_dropin(&cfg(true, false));
        assert_ne!(
            a, b,
            "different knob sets must produce different drop-in contents"
        );
    }

    #[test]
    fn write_creates_parent_dir_if_missing() {
        // resolved.conf.d/ is sometimes absent on a freshly-installed system.
        let root = crate::testing::TempRoot::new("resolved-no-parent");
        let paths = Paths::rooted_at(&root.path);
        let dir = paths.resolved_dropin_dir();
        assert!(!dir.exists(), "precondition: dir should not exist yet");
        write_dropin(&paths, &cfg(true, true)).expect("write");
        assert!(dir.is_dir(), "parent dir should have been created");
    }
}
