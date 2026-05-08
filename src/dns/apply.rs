// SPDX-License-Identifier: GPL-3.0-or-later

//! Drop-in writer for the `EDNSClientSubnet=no` knob.
//!
//! All of the policy lives in `super`; this module only knows how to
//! render, write, and remove the single drop-in file. Pure functions
//! where possible so unit tests can verify formatting without touching
//! the real `/etc`.

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{PROTEUS_DROPIN_NAME, Paths};
use crate::commands;
use crate::crypto::sha256;
use crate::version;

/// Body of the drop-in **without** the sha256 marker. Kept separate so the
/// hash can be computed over this exact text and stamped on top.
pub fn render_body() -> String {
    "[Resolve]\nEDNSClientSubnet=no\n".to_string()
}

/// Full drop-in contents including the management headers. The sha256
/// marker covers the body returned by `render_body` so the SHA stays
/// stable regardless of which proteus version stamped the header.
pub fn render_dropin() -> String {
    let body = render_body();
    let sha = sha256::hex_digest(body.as_bytes());
    format!(
        "# managed by proteus v{version}\n# do not edit; manage via /etc/proteus/config.toml or `proteus dns apply`\n# sha256:{sha}\n{body}",
        version = version::VERSION,
    )
}

/// Returns the path Proteus's drop-in lives at given a `Paths` layout.
pub fn dropin_path(paths: &Paths) -> PathBuf {
    paths.resolved_dropin_dir().join(PROTEUS_DROPIN_NAME)
}

/// Write the drop-in atomically. Creates the parent directory if it does
/// not already exist (the typical Fedora install ships with the dir
/// missing until something writes to it).
pub fn write_dropin(paths: &Paths) -> Result<PathBuf> {
    let dir = paths.resolved_dropin_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating drop-in dir {}", dir.display()))?;
    let path = dropin_path(paths);
    let body = render_dropin();
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

/// Returns true if Proteus's drop-in exists right now.
pub fn dropin_present(paths: &Paths) -> bool {
    dropin_path(paths).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_body_is_minimal_and_stable() {
        let body = render_body();
        assert!(body.contains("[Resolve]"));
        assert!(body.contains("EDNSClientSubnet=no"));
        // We never want this drop-in to ship anything but the one knob.
        assert!(!body.contains("DNSOverTLS"));
        assert!(!body.contains("DNSSEC"));
        assert!(!body.contains("DNS="));
    }

    #[test]
    fn render_dropin_includes_sha_of_body() {
        let body = render_body();
        let expected = sha256::hex_digest(body.as_bytes());
        let full = render_dropin();
        assert!(full.contains(&format!("sha256:{expected}")));
        assert!(full.contains("[Resolve]"));
        assert!(full.contains("EDNSClientSubnet=no"));
    }

    #[test]
    fn dropin_path_uses_proteus_filename() {
        let paths = Paths::default();
        let p = dropin_path(&paths);
        assert!(p.ends_with("10-proteus-no-ecs.conf"));
    }
}
