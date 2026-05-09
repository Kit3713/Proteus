// SPDX-License-Identifier: GPL-3.0-or-later

//! N11 — POSIX-fallback init implementation of [`InitSystem`].
//!
//! Detects:
//!
//! - **s6** — Skarnet's process supervision suite. Used by Artix-s6,
//!   Adelie, and AlpineLinux's `s6-rc` packagesets. The canonical
//!   live-tree pointers are `/etc/s6/` (config) and `/run/s6/` or
//!   `/run/service/` (live supervision tree). We accept any of those.
//! - **dinit** — Davidoa's small-footprint init/supervisor. Used by
//!   Chimera Linux. Detected via `/etc/dinit.d/` (services dir) or
//!   `/run/dinitctl` (control socket).
//! - **Anything POSIX-compliant we can't otherwise classify** — when
//!   the host has `/sbin/init` or `/init` but none of the named
//!   detectors fire. The artifact rendering is a generic
//!   shell-script that an operator can hand-wire into the host's
//!   actual scheduler. Consumers (`proteus apply`, `proteus
//!   doctor`) treat the fallback's `name()` token as a clear
//!   "unknown init; manual wiring required" signal.
//!
//! **Why this isn't just another concrete impl.** s6 and dinit each
//! have rich service-definition formats; rendering a fully-correct
//! `s6-rc` source-tree or a dinit `service` file is a meaningful
//! amount of code per init system. The roadmap entry (N11) calls
//! for *detection paths beyond the hardcoded list*, plus graceful
//! degradation. This module satisfies both: detection lights up so
//! `proteus doctor` reports the right init, and the generic shell
//! artifact gives an installer something to write while the
//! per-init renderers are still TODO.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{InitArtifact, InitSystem, validate_artifact_name, validate_exec};

/// Which POSIX-like init we detected (or `Unknown` for the
/// last-resort fallback). Determines the artifact's path and the
/// `name()` token surfaced to operators / `proteus doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosixFlavor {
    /// s6 / s6-rc on Artix, Adelie, Alpine.
    S6,
    /// dinit on Chimera, postmarketOS extras.
    Dinit,
    /// Generic POSIX — we found `/sbin/init` but no further signal.
    Unknown,
}

impl PosixFlavor {
    fn token(self) -> &'static str {
        match self {
            Self::S6 => "s6",
            Self::Dinit => "dinit",
            Self::Unknown => "posix-fallback",
        }
    }
}

pub struct PosixFallback {
    root: PathBuf,
    flavor: PosixFlavor,
}

impl PosixFallback {
    /// Construct a [`PosixFallback`] for `flavor`. Production code
    /// should use [`detect_flavor`] instead, which probes the host
    /// and picks the right flavor automatically.
    pub fn new(flavor: PosixFlavor) -> Self {
        Self {
            root: PathBuf::from("/"),
            flavor,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_root(flavor: PosixFlavor, root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            flavor,
        }
    }

    /// Probe `root` and return whichever specific flavor we can
    /// confirm — never falls through to `Unknown` here, because a
    /// caller that wants the last-resort fallback constructs it
    /// explicitly via `PosixFallback::new(PosixFlavor::Unknown)`.
    pub fn detect_flavor(root: &Path) -> Option<PosixFlavor> {
        if probe_s6(root) {
            return Some(PosixFlavor::S6);
        }
        if probe_dinit(root) {
            return Some(PosixFlavor::Dinit);
        }
        None
    }
}

impl InitSystem for PosixFallback {
    fn name(&self) -> &'static str {
        self.flavor.token()
    }

    fn detect(&self) -> bool {
        match self.flavor {
            PosixFlavor::S6 => probe_s6(&self.root),
            PosixFlavor::Dinit => probe_dinit(&self.root),
            PosixFlavor::Unknown => probe_posix_unknown(&self.root),
        }
    }

    fn schedule_periodic(
        &self,
        name: &str,
        interval_seconds: u64,
        exec: &str,
    ) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        if interval_seconds == 0 {
            anyhow::bail!("schedule_periodic: interval_seconds must be > 0");
        }
        let (dir, body) = match self.flavor {
            PosixFlavor::S6 => (
                Path::new("/etc/s6/sv").to_path_buf(),
                format!(
                    "#!/bin/sh\n\
                     # managed by proteus — periodic {name} every {interval_seconds}s (s6)\n\
                     exec 2>&1\n\
                     while :; do\n\
                     \tsleep {interval_seconds}\n\
                     \t{exec} || true\n\
                     done\n",
                ),
            ),
            PosixFlavor::Dinit => (
                Path::new("/etc/dinit.d").to_path_buf(),
                format!(
                    "# managed by proteus — periodic {name} every {interval_seconds}s (dinit)\n\
                     type = process\n\
                     command = /bin/sh -c 'while :; do sleep {interval_seconds}; {exec} || true; done'\n\
                     restart = true\n",
                ),
            ),
            PosixFlavor::Unknown => (
                Path::new("/usr/local/libexec/proteus").to_path_buf(),
                format!(
                    "#!/bin/sh\n\
                     # managed by proteus — periodic {name} every {interval_seconds}s\n\
                     # No supported init detected. Wire this script into the host's\n\
                     # scheduler manually (cron, anacron, a per-distro mechanism).\n\
                     exec 2>&1\n\
                     while :; do\n\
                     \tsleep {interval_seconds}\n\
                     \t{exec} || true\n\
                     done\n",
                ),
            ),
        };
        // s6 / dinit / posix-unknown all use a service-directory
        // layout; the artifact's `path` is the script that the
        // installer will write. Higher-level mkdir / symlink wiring
        // is out of scope for this module.
        let leaf = match self.flavor {
            PosixFlavor::Dinit => format!("proteus-{name}"),
            _ => format!("proteus-{name}/run"),
        };
        Ok(InitArtifact {
            path: dir.join(leaf),
            content: body,
            // dinit service files are configuration (0o644); s6
            // and posix run-scripts are executables (0o755).
            mode: if matches!(self.flavor, PosixFlavor::Dinit) {
                0o644
            } else {
                0o755
            },
        })
    }

    fn hook_resume(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        // None of the three flavors ship a native suspend/resume
        // target. We drop a script that an installer can call from
        // an elogind sleep.d shim or systemd-logind hook (where
        // applicable). Not wired by us — explicit operator decision.
        let dir = match self.flavor {
            PosixFlavor::S6 => Path::new("/etc/s6/sleep.d"),
            PosixFlavor::Dinit => Path::new("/etc/dinit.d/sleep.d"),
            PosixFlavor::Unknown => Path::new("/etc/proteus/sleep.d"),
        };
        let path = dir.join(format!("90proteus-{name}-resume.sh"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — resume hook for {name} ({})\n\
             # Pair with elogind / logind sleep.d if your distro ships one.\n\
             {exec} || true\n",
            self.flavor.token(),
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o755,
        })
    }

    fn hook_boot(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        let (dir, content) = match self.flavor {
            PosixFlavor::S6 => (
                Path::new("/etc/s6/sv").to_path_buf(),
                format!(
                    "#!/bin/sh\n\
                     # managed by proteus — boot hook for {name} (s6)\n\
                     {exec} || true\n",
                ),
            ),
            PosixFlavor::Dinit => (
                Path::new("/etc/dinit.d").to_path_buf(),
                format!(
                    "# managed by proteus — boot hook for {name} (dinit)\n\
                     type = scripted\n\
                     command = /bin/sh -c '{exec} || true'\n",
                ),
            ),
            PosixFlavor::Unknown => (
                Path::new("/usr/local/libexec/proteus").to_path_buf(),
                format!(
                    "#!/bin/sh\n\
                     # managed by proteus — boot hook for {name}\n\
                     # No supported init detected. Run from rc.local or equivalent.\n\
                     {exec} || true\n",
                ),
            ),
        };
        let leaf = match self.flavor {
            PosixFlavor::Dinit => format!("proteus-{name}-boot"),
            _ => format!("proteus-{name}-boot/run"),
        };
        Ok(InitArtifact {
            path: dir.join(leaf),
            content,
            mode: if matches!(self.flavor, PosixFlavor::Dinit) {
                0o644
            } else {
                0o755
            },
        })
    }
}

/// True iff `root` looks like an s6-managed host. Probe order
/// matches Artix's docs: `/run/s6/` is the live supervision tree,
/// `/etc/s6/` the persistent config, `/run/service/` the alternate
/// runtime layout some derivatives use.
pub(crate) fn probe_s6(root: &Path) -> bool {
    root.join("run/s6").is_dir()
        || root.join("etc/s6").is_dir()
        || root.join("run/service").is_dir()
}

/// True iff `root` looks like a dinit-managed host. `/run/dinitctl`
/// is the control socket dinit binds; `/etc/dinit.d/` is the
/// service-definition directory the dinit daemon reads. Either is
/// a strong signal.
pub(crate) fn probe_dinit(root: &Path) -> bool {
    root.join("run/dinitctl").exists() || root.join("etc/dinit.d").is_dir()
}

/// True iff `root` has a POSIX `init` binary but none of the
/// classified detectors fire. The probe is intentionally weak so
/// the fallback only lights up after every more-specific impl has
/// said "not me" — the [`super::select::detect`] walk handles that
/// ordering.
pub(crate) fn probe_posix_unknown(root: &Path) -> bool {
    root.join("sbin/init").exists() || root.join("init").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavor_tokens_are_stable() {
        assert_eq!(PosixFlavor::S6.token(), "s6");
        assert_eq!(PosixFlavor::Dinit.token(), "dinit");
        assert_eq!(PosixFlavor::Unknown.token(), "posix-fallback");
    }

    #[test]
    fn detect_s6_via_run_s6() {
        let tmp = crate::testing::TempRoot::new("init-s6-runs6");
        std::fs::create_dir_all(tmp.path.join("run/s6")).unwrap();
        let p = PosixFallback::with_root(PosixFlavor::S6, &tmp.path);
        assert!(p.detect());
        assert_eq!(p.name(), "s6");
    }

    #[test]
    fn detect_s6_via_etc_s6() {
        let tmp = crate::testing::TempRoot::new("init-s6-etcs6");
        std::fs::create_dir_all(tmp.path.join("etc/s6")).unwrap();
        let p = PosixFallback::with_root(PosixFlavor::S6, &tmp.path);
        assert!(p.detect());
    }

    #[test]
    fn detect_dinit_via_dinitctl_socket() {
        let tmp = crate::testing::TempRoot::new("init-dinit-sock");
        std::fs::create_dir_all(tmp.path.join("run")).unwrap();
        std::fs::write(tmp.path.join("run/dinitctl"), "").unwrap();
        let p = PosixFallback::with_root(PosixFlavor::Dinit, &tmp.path);
        assert!(p.detect());
        assert_eq!(p.name(), "dinit");
    }

    #[test]
    fn detect_dinit_via_etc_dinit_d() {
        let tmp = crate::testing::TempRoot::new("init-dinit-svcdir");
        std::fs::create_dir_all(tmp.path.join("etc/dinit.d")).unwrap();
        let p = PosixFallback::with_root(PosixFlavor::Dinit, &tmp.path);
        assert!(p.detect());
    }

    #[test]
    fn detect_unknown_when_only_sbin_init_present() {
        let tmp = crate::testing::TempRoot::new("init-posix-unknown");
        std::fs::create_dir_all(tmp.path.join("sbin")).unwrap();
        std::fs::write(tmp.path.join("sbin/init"), "").unwrap();
        let p = PosixFallback::with_root(PosixFlavor::Unknown, &tmp.path);
        assert!(p.detect());
        assert_eq!(p.name(), "posix-fallback");
    }

    #[test]
    fn detect_negative_on_empty_root() {
        let tmp = crate::testing::TempRoot::new("init-posix-empty");
        for f in [PosixFlavor::S6, PosixFlavor::Dinit, PosixFlavor::Unknown] {
            let p = PosixFallback::with_root(f, &tmp.path);
            assert!(!p.detect(), "flavor {f:?} should not detect on empty root");
        }
    }

    #[test]
    fn detect_flavor_picks_specific_over_unknown() {
        let tmp = crate::testing::TempRoot::new("init-posix-specific");
        std::fs::create_dir_all(tmp.path.join("etc/s6")).unwrap();
        let f = PosixFallback::detect_flavor(&tmp.path);
        assert_eq!(f, Some(PosixFlavor::S6));
    }

    #[test]
    fn detect_flavor_returns_none_when_nothing_matches() {
        let tmp = crate::testing::TempRoot::new("init-posix-none");
        let f = PosixFallback::detect_flavor(&tmp.path);
        assert!(f.is_none());
    }

    #[test]
    fn s6_schedule_emits_supervised_loop() {
        let p = PosixFallback::new(PosixFlavor::S6);
        let art = p
            .schedule_periodic("rotate", 1800, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.starts_with("/etc/s6/sv"));
        assert!(art.path.ends_with("proteus-rotate/run"));
        assert_eq!(art.mode, 0o755);
        assert!(art.content.contains("sleep 1800"));
    }

    #[test]
    fn dinit_schedule_emits_service_definition() {
        let p = PosixFallback::new(PosixFlavor::Dinit);
        let art = p
            .schedule_periodic("rotate", 1800, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.starts_with("/etc/dinit.d"));
        assert_eq!(art.mode, 0o644);
        assert!(art.content.contains("type = process"));
        assert!(art.content.contains("/usr/local/bin/proteus rotate --yes"));
    }

    #[test]
    fn unknown_schedule_emits_libexec_script() {
        let p = PosixFallback::new(PosixFlavor::Unknown);
        let art = p
            .schedule_periodic("rotate", 600, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.starts_with("/usr/local/libexec/proteus"));
        assert_eq!(art.mode, 0o755);
        assert!(
            art.content
                .contains("No supported init detected. Wire this script")
        );
    }

    #[test]
    fn invalid_inputs_rejected() {
        let p = PosixFallback::new(PosixFlavor::S6);
        assert!(p.schedule_periodic("..", 60, "/x").is_err());
        assert!(p.schedule_periodic("ok", 0, "/x").is_err());
        assert!(p.hook_boot("ok", "\n").is_err());
        assert!(p.hook_resume("ok", "x\nbad").is_err());
    }

    #[test]
    fn hook_resume_lands_in_sleep_d_per_flavor() {
        let p = PosixFallback::new(PosixFlavor::S6);
        let art = p.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.to_string_lossy().contains("/etc/s6/sleep.d/"));

        let p = PosixFallback::new(PosixFlavor::Dinit);
        let art = p.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.to_string_lossy().contains("/etc/dinit.d/sleep.d/"));

        let p = PosixFallback::new(PosixFlavor::Unknown);
        let art = p.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.to_string_lossy().contains("/etc/proteus/sleep.d/"));
    }
}
