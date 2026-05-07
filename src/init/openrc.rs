// SPDX-License-Identifier: GPL-3.0-or-later

//! OpenRC implementation of [`InitSystem`]. Targets Alpine, Gentoo,
//! Artix-OpenRC.
//!
//! Periodic checks land in `/etc/periodic/<bucket>/proteus-<name>` —
//! Alpine + Gentoo both ship a cron driver that walks those buckets.
//! Resume / boot hooks ride on `/etc/local.d/<name>.start`, the
//! `local` service's spool of one-shot start scripts.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{InitArtifact, InitSystem, validate_artifact_name, validate_exec};

pub struct Openrc {
    root: PathBuf,
}

impl Openrc {
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

impl Default for Openrc {
    fn default() -> Self {
        Self::new()
    }
}

impl InitSystem for Openrc {
    fn name(&self) -> &'static str {
        "openrc"
    }

    fn detect(&self) -> bool {
        // OpenRC's runtime tree is `/run/openrc`; its launcher binary
        // is `/sbin/openrc-run`. Either is sufficient — the binary
        // alone (without runtime) means "openrc is installed but
        // didn't boot us", and the runtime alone (without binary)
        // never happens but we accept it for defensive symmetry.
        let openrc_run = self.root.join("sbin/openrc-run");
        let openrc_runtime = self.root.join("run/openrc");
        openrc_run.exists() || openrc_runtime.is_dir()
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
        let bucket = bucket_for(interval_seconds);
        let path = Path::new("/etc/periodic")
            .join(bucket)
            .join(format!("proteus-{name}"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — periodic {name} ({bucket} bucket)\n\
             # do not edit; manage via `proteus apply` or your installer\n\
             exec {exec}\n",
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
        // OpenRC has no first-class suspend hook. The portable answer
        // is /etc/local.d/<name>.start, which fires when the `local`
        // service starts — not on resume. The expected install path
        // is to also drop a hook into elogind's sleep.d (handled by
        // the future install script), but the InitSystem contract is
        // the local.d snippet so the artifact has a single home.
        let path = Path::new("/etc/local.d").join(format!("proteus-{name}-resume.start"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — resume hook for {name}\n\
             # OpenRC has no native resume target; pair with elogind's sleep.d\n\
             # if your distro ships it.\n\
             exec {exec}\n",
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
        let path = Path::new("/etc/local.d").join(format!("proteus-{name}-boot.start"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — boot hook for {name}\n\
             exec {exec}\n",
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o755,
        })
    }
}

/// Map an interval to one of OpenRC's periodic buckets. The drivers
/// shipped on Alpine/Gentoo only know `15min`/`hourly`/`daily`/
/// `weekly`/`monthly`; pick the largest bucket the interval still
/// satisfies so we under-fire rather than over-fire (over-firing on
/// a `15min` bucket when the user asked for daily would surprise).
fn bucket_for(interval_seconds: u64) -> &'static str {
    match interval_seconds {
        0..=900 => "15min",
        901..=3600 => "hourly",
        3601..=86_400 => "daily",
        86_401..=604_800 => "weekly",
        _ => "monthly",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_openrc() {
        assert_eq!(Openrc::new().name(), "openrc");
    }

    #[test]
    fn detect_negative_on_empty_root() {
        let tmp = crate::testing::TempRoot::new("init-openrc-neg");
        let o = Openrc::with_root(&tmp.path);
        assert!(!o.detect());
    }

    #[test]
    fn detect_positive_with_openrc_run_binary() {
        let tmp = crate::testing::TempRoot::new("init-openrc-bin");
        std::fs::create_dir_all(tmp.path.join("sbin")).unwrap();
        std::fs::write(tmp.path.join("sbin/openrc-run"), "#!/bin/sh\n").unwrap();
        let o = Openrc::with_root(&tmp.path);
        assert!(o.detect());
    }

    #[test]
    fn detect_positive_with_runtime_dir() {
        let tmp = crate::testing::TempRoot::new("init-openrc-rt");
        std::fs::create_dir_all(tmp.path.join("run/openrc")).unwrap();
        let o = Openrc::with_root(&tmp.path);
        assert!(o.detect());
    }

    #[test]
    fn schedule_periodic_picks_hourly_bucket_for_one_hour() {
        let o = Openrc::new();
        let art = o
            .schedule_periodic("rotate", 3600, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.starts_with("/etc/periodic/hourly"));
        assert_eq!(art.mode, 0o755);
        assert!(art.content.starts_with("#!/bin/sh\n"));
        assert!(art.content.contains("exec /usr/local/bin/proteus rotate --yes"));
    }

    #[test]
    fn schedule_periodic_picks_daily_bucket_for_long_interval() {
        let o = Openrc::new();
        let art = o.schedule_periodic("rotate", 86_400, "/x").unwrap();
        assert!(art.path.starts_with("/etc/periodic/daily"));
    }

    #[test]
    fn hook_boot_lands_in_local_d() {
        let o = Openrc::new();
        let art = o.hook_boot("apply", "/x").unwrap();
        assert!(
            art.path
                .ends_with("proteus-apply-boot.start"),
            "got {}",
            art.path.display()
        );
        assert_eq!(art.mode, 0o755);
        assert!(art.content.contains("exec /x"));
    }

    #[test]
    fn hook_resume_lands_in_local_d() {
        let o = Openrc::new();
        let art = o.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.ends_with("proteus-rotate-resume.start"));
        assert!(art.content.contains("elogind"));
    }

    #[test]
    fn invalid_inputs_rejected() {
        let o = Openrc::new();
        assert!(o.schedule_periodic("../bad", 60, "/x").is_err());
        assert!(o.schedule_periodic("ok", 0, "/x").is_err());
        assert!(o.hook_boot("ok", "").is_err());
    }
}
