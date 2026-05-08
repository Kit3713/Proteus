// SPDX-License-Identifier: GPL-3.0-or-later

//! runit implementation of [`InitSystem`]. Targets Void, Artix-Runit.
//!
//! runit doesn't ship a periodic-task driver, so `schedule_periodic`
//! produces a long-running supervised service whose `run` script
//! sleeps the interval and execs the command in a loop. That keeps
//! us inside the runit shape (everything is a service directory)
//! instead of bolting a cron clone on top.
//!
//! Resume + boot hooks land under `/etc/runit/core-services/` —
//! Void's idiomatic spot for "ride along with the global boot
//! sequence" oneshots.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{InitArtifact, InitSystem, validate_artifact_name, validate_exec};

pub struct Runit {
    root: PathBuf,
}

impl Runit {
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Default for Runit {
    fn default() -> Self {
        Self::new()
    }
}

impl InitSystem for Runit {
    fn name(&self) -> &'static str {
        "runit"
    }

    fn detect(&self) -> bool {
        // `/etc/runit/runsvdir` is runit's "active service directory"
        // pointer — present on every Void / Artix-Runit install. We
        // additionally accept `/run/runit` for hosts that use the
        // alternate runtime layout.
        let svdir = self.root.join("etc/runit/runsvdir");
        let runtime = self.root.join("run/runit");
        svdir.exists() || runtime.is_dir()
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
        // The artifact `path` points at the `run` script inside the
        // service directory — that's the file an installer writes.
        // The directory itself (`/etc/sv/proteus-<name>/`) is created
        // by the installer's mkdir step, not by us.
        let path = Path::new("/etc/sv")
            .join(format!("proteus-{name}"))
            .join("run");
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — periodic {name} every {interval_seconds}s\n\
             # runit lacks a cron-style scheduler; this service loops with\n\
             # `sleep` so each tick is supervised the same as any runit job.\n\
             exec 2>&1\n\
             while :; do\n\
             \tsleep {interval_seconds}\n\
             \t{exec} || true\n\
             done\n",
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o755,
        })
    }

    fn hook_resume(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        // runit has no native suspend/resume hook. We drop a script
        // into core-services so it ships with the layout, but the
        // real wiring (an elogind sleep.d shim or a per-distro hack)
        // is the installer's job — same as OpenRC.
        let path =
            Path::new("/etc/runit/core-services").join(format!("90-proteus-{name}-resume.sh"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — resume hook for {name}\n\
             # runit has no native resume target; pair with elogind's sleep.d\n\
             # if your distro ships it.\n\
             {exec} || true\n",
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
        let path = Path::new("/etc/runit/core-services").join(format!("80-proteus-{name}-boot.sh"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — boot hook for {name}\n\
             {exec} || true\n",
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o755,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_runit() {
        assert_eq!(Runit::new().name(), "runit");
    }

    #[test]
    fn detect_negative_on_empty_root() {
        let tmp = crate::testing::TempRoot::new("init-runit-neg");
        let r = Runit::with_root(&tmp.path);
        assert!(!r.detect());
    }

    #[test]
    fn detect_positive_with_runsvdir() {
        let tmp = crate::testing::TempRoot::new("init-runit-pos");
        std::fs::create_dir_all(tmp.path.join("etc/runit")).unwrap();
        // runsvdir is a symlink on a real system; a regular file is
        // enough for the .exists() probe in tests.
        std::fs::write(tmp.path.join("etc/runit/runsvdir"), "").unwrap();
        let r = Runit::with_root(&tmp.path);
        assert!(r.detect());
    }

    #[test]
    fn schedule_periodic_emits_supervised_loop() {
        let r = Runit::new();
        let art = r
            .schedule_periodic("rotate", 7200, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.ends_with("proteus-rotate/run"));
        assert_eq!(art.mode, 0o755);
        assert!(art.content.contains("sleep 7200"));
        assert!(art.content.contains("/usr/local/bin/proteus rotate --yes"));
        assert!(art.content.contains("while :"));
    }

    #[test]
    fn hook_boot_lands_in_core_services() {
        let r = Runit::new();
        let art = r
            .hook_boot("apply", "/usr/local/bin/proteus apply --yes")
            .unwrap();
        assert!(
            art.path.starts_with("/etc/runit/core-services"),
            "got {}",
            art.path.display()
        );
        assert!(art.path.to_string_lossy().contains("80-proteus-apply"));
    }

    #[test]
    fn hook_resume_lands_in_core_services() {
        let r = Runit::new();
        let art = r.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.starts_with("/etc/runit/core-services"));
        assert!(art.content.contains("elogind"));
    }

    #[test]
    fn invalid_inputs_rejected() {
        let r = Runit::new();
        assert!(r.schedule_periodic("..", 60, "/x").is_err());
        assert!(r.schedule_periodic("ok", 0, "/x").is_err());
        assert!(r.hook_boot("ok", "\n").is_err());
    }
}
