// SPDX-License-Identifier: GPL-3.0-or-later

//! `InitSystem` — the abstraction that lets Proteus emit scheduling /
//! resume / boot hooks in whatever shape the host's init system speaks.
//!
//! Roadmap Milestone 5 (`docs/ROADMAP.md`): the trait + four impls land
//! here. `dist/install.sh` and the existing `src/timer/` pipeline will
//! grow consumers in follow-ups; this PR ships the abstraction layer
//! so the per-distro packaging work has something concrete to call.
//!
//! Why a synchronous trait (vs the boxed-future `NetworkBackend` next
//! door): every method here is pure-render — it produces unit/script
//! text and a target path; nothing touches the kernel, dbus, or the
//! filesystem. Callers (`dist/install.sh`, `proteus apply`'s timer
//! reconciler) decide whether to commit the artifact, dry-run it, or
//! diff it against what's on disk.
//!
//! Selection mirrors `crate::backend::select`: walk Systemd → OpenRC →
//! Runit → SysVinit and pick the first whose `detect()` returns true.
//! Default to Systemd so `proteus doctor` and any uninitialised path
//! still has something to render.

pub mod openrc;
pub mod posix_fallback;
pub mod runit;
pub mod select;
pub mod systemd;
pub mod sysvinit;

use std::path::PathBuf;

use anyhow::Result;

pub use select::{available_systems, detect};

/// What `schedule_periodic` / `hook_resume` / `hook_boot` produce: a
/// path to write to, the textual body, and the file mode an installer
/// should chmod the result to. Nothing here is committed to disk —
/// the caller decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitArtifact {
    /// Where this content should land on a real install. Relative to
    /// `/` — installers prepend their own staging prefix.
    pub path: PathBuf,
    /// Unit / script body. Always ends with `\n`.
    pub content: String,
    /// File mode. `0o644` for unit files, `0o755` for executables.
    pub mode: u32,
}

/// The trait every init-system impl satisfies. `name()` is the stable
/// token used in logs and `proteus doctor`'s init matrix; `detect()`
/// is a cheap path probe for "is this init the active one".
pub trait InitSystem: Send + Sync {
    /// Stable token for logs / doctor / `[init] driver = "..."`. One
    /// of `"systemd"`, `"openrc"`, `"runit"`, `"sysvinit"`.
    fn name(&self) -> &'static str;

    /// True iff this init system is the active one on the current
    /// host. Cheap, never-mutating path probe — never panics. Used by
    /// [`detect`] for the auto path.
    fn detect(&self) -> bool;

    /// Schedule a periodic mutating check (rotation, lease renew, ...).
    /// `name` is a short token that goes into the unit/script filename
    /// (`proteus-<name>`); `interval_seconds` is how often the check
    /// fires; `exec` is the absolute command line to run.
    ///
    /// systemd impls return the timer artifact; the matching `.service`
    /// is bundled into the same content blob so a single write covers
    /// both. OpenRC/Runit/SysVinit return whatever the bucket layout
    /// of that init system uses.
    fn schedule_periodic(
        &self,
        name: &str,
        interval_seconds: u64,
        exec: &str,
    ) -> Result<InitArtifact>;

    /// Hook resume-from-suspend so identifiers rotate after a wake.
    /// `name` becomes part of the artifact filename; `exec` is the
    /// command the hook should invoke.
    fn hook_resume(&self, name: &str, exec: &str) -> Result<InitArtifact>;

    /// Hook boot so identifiers rotate before any user-traffic process
    /// opens a socket. `name` becomes part of the artifact filename;
    /// `exec` is the command the hook should invoke.
    fn hook_boot(&self, name: &str, exec: &str) -> Result<InitArtifact>;
}

/// Reject names that would let the caller break out of the artifact's
/// path (e.g. `..`, embedded slashes) or smuggle newlines into a unit
/// file. Used by every impl before formatting paths or unit bodies.
///
/// `proteus apply` runs as root and most consumers of this module are
/// likewise root-only, so the threat is limited; the check exists so
/// a config-file typo can't silently produce a broken `.service` or a
/// path that escapes `/etc/systemd/system/`.
pub(crate) fn validate_artifact_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("init artifact name is empty");
    }
    if name.len() > 64 {
        anyhow::bail!("init artifact name '{name}' is too long (>64 chars)");
    }
    for ch in name.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
        if !ok {
            anyhow::bail!(
                "init artifact name '{name}' contains forbidden character {ch:?}; \
                 use [A-Za-z0-9_-] only"
            );
        }
    }
    Ok(())
}

/// Reject exec lines containing characters that would break unit/script
/// rendering. The threat surface is small (`exec` is hardcoded by
/// install scripts today) but a typo like a stray newline silently
/// produces a malformed unit.
pub(crate) fn validate_exec(exec: &str) -> Result<()> {
    if exec.trim().is_empty() {
        anyhow::bail!("init artifact exec is empty");
    }
    if exec.contains(['\n', '\r', '\0']) {
        anyhow::bail!("init artifact exec contains a forbidden character (\\n, \\r, NUL)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_artifact_name_accepts_alnum_dash_underscore() {
        assert!(validate_artifact_name("rotate").is_ok());
        assert!(validate_artifact_name("rotate-1").is_ok());
        assert!(validate_artifact_name("rotate_check").is_ok());
        assert!(validate_artifact_name("Rotate2").is_ok());
    }

    #[test]
    fn validate_artifact_name_rejects_path_traversal_and_empty() {
        assert!(validate_artifact_name("").is_err());
        assert!(validate_artifact_name("..").is_err());
        assert!(validate_artifact_name("a/b").is_err());
        assert!(validate_artifact_name("a b").is_err());
        assert!(validate_artifact_name("a.b").is_err());
        assert!(validate_artifact_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn validate_exec_rejects_newlines_and_empty() {
        assert!(validate_exec("/usr/local/bin/proteus rotate --yes").is_ok());
        assert!(validate_exec("").is_err());
        assert!(validate_exec("   ").is_err());
        assert!(validate_exec("a\nb").is_err());
        assert!(validate_exec("a\rb").is_err());
        assert!(validate_exec("a\0b").is_err());
    }

    #[test]
    fn artifact_clone_round_trip() {
        let a = InitArtifact {
            path: PathBuf::from("/etc/systemd/system/proteus-rotate.timer"),
            content: "[Timer]\n".into(),
            mode: 0o644,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
