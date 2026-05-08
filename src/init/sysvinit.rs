// SPDX-License-Identifier: GPL-3.0-or-later

//! SysV-init implementation of [`InitSystem`]. The fallback for the
//! handful of distros (Devuan, Slackware, antiX) that ship neither
//! systemd nor a modern alternative.
//!
//! Periodic checks ride on `/etc/cron.d/` — every distro that lands
//! here also ships vixie-cron or cronie, since SysV-init has no
//! built-in scheduler. Resume hooks go to pm-utils
//! (`/etc/pm/sleep.d/`), the only portable resume hook on these
//! systems. Boot hooks become an LSB-headered `/etc/init.d/` script
//! that an installer wires into the right runlevels with `update-rc.d`
//! or `chkconfig`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{InitArtifact, InitSystem, validate_artifact_name, validate_exec};

pub struct Sysvinit {
    root: PathBuf,
}

impl Sysvinit {
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

impl Default for Sysvinit {
    fn default() -> Self {
        Self::new()
    }
}

impl InitSystem for Sysvinit {
    fn name(&self) -> &'static str {
        "sysvinit"
    }

    fn detect(&self) -> bool {
        // `/etc/init.d` exists on systemd hosts too (Debian's compat
        // shim), so we explicitly require systemd to be ABSENT for
        // SysV-init detection. This keeps the auto-pick from picking
        // sysvinit on a Debian-with-systemd host that happens to have
        // an /etc/init.d directory.
        let initd = self.root.join("etc/init.d");
        let systemd_runtime = self.root.join("run/systemd/system");
        initd.is_dir() && !systemd_runtime.is_dir()
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
        let cron_expr = cron_expr_for(interval_seconds);
        let path = Path::new("/etc/cron.d").join(format!("proteus-{name}"));
        // cron.d entries: `<schedule> <user> <command>`. SHELL/PATH are
        // set explicitly because the cron daemon's defaults vary per
        // distro and we don't want a missing /usr/local/bin to surprise
        // an installer.
        let content = format!(
            "# managed by proteus — periodic {name} every {interval_seconds}s\n\
             SHELL=/bin/sh\n\
             PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n\
             {cron_expr} root {exec}\n",
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o644,
        })
    }

    fn hook_resume(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        // pm-utils calls hooks with `$1 in {suspend,hibernate,
        // thaw,resume}`; we only fire on the wake-up half.
        let path = Path::new("/etc/pm/sleep.d").join(format!("90proteus-{name}"));
        let content = format!(
            "#!/bin/sh\n\
             # managed by proteus — pm-utils resume hook for {name}\n\
             case \"$1\" in\n\
             \tthaw|resume)\n\
             \t\t{exec} || true\n\
             \t\t;;\n\
             esac\n",
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
        // LSB-headered init script. The Required-Start / Default-Start
        // values match the conventions used by Debian's networking
        // sample so update-rc.d wires the symlinks into the right
        // runlevels.
        let path = Path::new("/etc/init.d").join(format!("proteus-{name}"));
        let content = format!(
            "#!/bin/sh\n\
             ### BEGIN INIT INFO\n\
             # Provides:          proteus-{name}\n\
             # Required-Start:    $network $remote_fs\n\
             # Required-Stop:     $network $remote_fs\n\
             # Default-Start:     2 3 4 5\n\
             # Default-Stop:      0 1 6\n\
             # Short-Description: Proteus {name} boot hook\n\
             # Description:       managed by proteus; runs once at boot.\n\
             ### END INIT INFO\n\
             \n\
             case \"$1\" in\n\
             \tstart)\n\
             \t\t{exec} || true\n\
             \t\t;;\n\
             \tstop|restart|force-reload|status)\n\
             \t\t# oneshot; nothing to stop or report.\n\
             \t\t;;\n\
             \t*)\n\
             \t\techo \"Usage: $0 {{start|stop|restart|force-reload|status}}\"\n\
             \t\texit 1\n\
             \t\t;;\n\
             esac\n\
             exit 0\n",
        );
        Ok(InitArtifact {
            path,
            content,
            mode: 0o755,
        })
    }
}

/// Pick a cron expression that approximates `interval_seconds`. Cron
/// is bucket-shaped, not continuous, so we map to the closest standard
/// cadence rather than try to render a `*/N` expression for arbitrary
/// values.
fn cron_expr_for(interval_seconds: u64) -> &'static str {
    match interval_seconds {
        // sub-minute: clamp to every minute (cron's finest grain).
        0..=60 => "* * * * *",
        61..=300 => "*/5 * * * *",
        301..=900 => "*/15 * * * *",
        901..=1800 => "*/30 * * * *",
        1801..=3600 => "0 * * * *",
        3601..=21_600 => "0 */2 * * *",
        21_601..=43_200 => "0 */6 * * *",
        43_201..=86_400 => "0 0 * * *",
        86_401..=604_800 => "0 0 * * 0",
        _ => "0 0 1 * *",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_sysvinit() {
        assert_eq!(Sysvinit::new().name(), "sysvinit");
    }

    #[test]
    fn detect_negative_on_empty_root() {
        let tmp = crate::testing::TempRoot::new("init-sysv-neg");
        let s = Sysvinit::with_root(&tmp.path);
        assert!(!s.detect());
    }

    #[test]
    fn detect_positive_when_initd_exists_and_systemd_absent() {
        let tmp = crate::testing::TempRoot::new("init-sysv-pos");
        std::fs::create_dir_all(tmp.path.join("etc/init.d")).unwrap();
        let s = Sysvinit::with_root(&tmp.path);
        assert!(s.detect());
    }

    #[test]
    fn detect_negative_when_systemd_also_present() {
        // Debian-with-systemd: /etc/init.d still exists as a compat
        // shim. SysV-init detection must NOT fire here.
        let tmp = crate::testing::TempRoot::new("init-sysv-debian");
        std::fs::create_dir_all(tmp.path.join("etc/init.d")).unwrap();
        std::fs::create_dir_all(tmp.path.join("run/systemd/system")).unwrap();
        let s = Sysvinit::with_root(&tmp.path);
        assert!(!s.detect());
    }

    #[test]
    fn schedule_periodic_emits_cron_d_entry() {
        let s = Sysvinit::new();
        let art = s
            .schedule_periodic("rotate", 7200, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.starts_with("/etc/cron.d/"));
        assert_eq!(art.mode, 0o644);
        // 7200s = 2h → every-2h cron expression.
        assert!(art.content.contains("0 */2 * * *"));
        assert!(
            art.content
                .contains("root /usr/local/bin/proteus rotate --yes")
        );
    }

    #[test]
    fn hook_resume_writes_pm_utils_hook() {
        let s = Sysvinit::new();
        let art = s.hook_resume("rotate", "/x").unwrap();
        assert!(art.path.starts_with("/etc/pm/sleep.d"));
        assert_eq!(art.mode, 0o755);
        assert!(art.content.contains("thaw|resume"));
    }

    #[test]
    fn hook_boot_writes_lsb_init_script() {
        let s = Sysvinit::new();
        let art = s
            .hook_boot("apply", "/usr/local/bin/proteus apply --yes")
            .unwrap();
        assert!(art.path.starts_with("/etc/init.d/"));
        assert_eq!(art.mode, 0o755);
        assert!(art.content.contains("### BEGIN INIT INFO"));
        assert!(art.content.contains("### END INIT INFO"));
        assert!(art.content.contains("Default-Start:     2 3 4 5"));
    }

    #[test]
    fn cron_expr_buckets_make_sense() {
        assert_eq!(cron_expr_for(30), "* * * * *");
        assert_eq!(cron_expr_for(300), "*/5 * * * *");
        assert_eq!(cron_expr_for(3600), "0 * * * *");
        assert_eq!(cron_expr_for(86_400), "0 0 * * *");
    }

    #[test]
    fn invalid_inputs_rejected() {
        let s = Sysvinit::new();
        assert!(s.schedule_periodic("..", 60, "/x").is_err());
        assert!(s.hook_boot("ok", "").is_err());
        assert!(s.hook_resume("ok", "x\ny").is_err());
    }
}
