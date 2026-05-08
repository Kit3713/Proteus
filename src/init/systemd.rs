// SPDX-License-Identifier: GPL-3.0-or-later

//! systemd implementation of [`InitSystem`].
//!
//! Mirrors the unit shapes already shipped under `dist/systemd/*` so a
//! generated artifact looks indistinguishable from the hand-curated
//! ones (After=, Wants=, hardening block, journald StdoutPath). The
//! shipped units are read by `tests/` to keep the canonical shape; if
//! you change those, update this module's renderers in lockstep.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{InitArtifact, InitSystem, validate_artifact_name, validate_exec};

/// systemd impl. `root` is the filesystem root probed by [`detect`];
/// production code uses `/`, tests pass a tempdir so detection is
/// hermetic.
pub struct Systemd {
    root: PathBuf,
}

impl Systemd {
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    /// Test-only constructor that re-roots the detect() probe at
    /// `root`. Production code never calls this.
    #[cfg(test)]
    pub(crate) fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Default for Systemd {
    fn default() -> Self {
        Self::new()
    }
}

impl InitSystem for Systemd {
    fn name(&self) -> &'static str {
        "systemd"
    }

    fn detect(&self) -> bool {
        // `/run/systemd/system` is the canonical "systemd is PID 1
        // and has populated its runtime tree" probe; matches the
        // `status::detect_system()` check the doctor already uses.
        let probe = self.root.join("run/systemd/system");
        probe.is_dir()
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
        // Bundle both the .timer and the .service into a single
        // artifact body, separated by a "===" sentinel header. The
        // timer alone is useless without the service it triggers, so
        // emitting both as one blob keeps the install script simple
        // (one write, one path-derive).
        let timer_path = unit_path("timer", name);
        let service_path = unit_path("service", name);
        let content = format!(
            "# {timer}\n{timer_body}\n# {service}\n{service_body}",
            timer = timer_path.display(),
            timer_body = render_periodic_timer(name, interval_seconds),
            service = service_path.display(),
            service_body = render_oneshot_service(name, exec, "timers.target"),
        );
        Ok(InitArtifact {
            path: timer_path,
            content,
            mode: 0o644,
        })
    }

    fn hook_resume(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        Ok(InitArtifact {
            path: unit_path("service", &format!("{name}-resume")),
            content: render_resume_service(name, exec),
            mode: 0o644,
        })
    }

    fn hook_boot(&self, name: &str, exec: &str) -> Result<InitArtifact> {
        validate_artifact_name(name)?;
        validate_exec(exec)?;
        Ok(InitArtifact {
            path: unit_path("service", &format!("{name}-boot")),
            content: render_oneshot_service(name, exec, "multi-user.target"),
            mode: 0o644,
        })
    }
}

fn unit_path(suffix: &str, name: &str) -> PathBuf {
    Path::new("/etc/systemd/system").join(format!("proteus-{name}.{suffix}"))
}

/// `[Timer] OnUnitActiveSec=<n>` — fires every `n` seconds after the
/// last activation. Mirrors the `dist/systemd/proteus-rotate.timer`
/// shape: `Persistent=true`, `AccuracySec=` and `RandomizedDelaySec=`
/// set so the cadence is not itself a Proteus fingerprint observable
/// across hosts (issue #303). Reuses `crate::timer::pick_jitter` so
/// generated artifacts share the same band table as the user-set
/// drop-ins.
fn render_periodic_timer(name: &str, interval_seconds: u64) -> String {
    let (accuracy, randomized) = crate::timer::pick_jitter(interval_seconds);
    format!(
        "[Unit]\n\
         Description=Proteus periodic check ({name}) ~ every {interval_seconds}s (jittered)\n\
         \n\
         [Timer]\n\
         OnUnitActiveSec={interval_seconds}\n\
         OnBootSec={interval_seconds}\n\
         Persistent=true\n\
         AccuracySec={accuracy}\n\
         RandomizedDelaySec={randomized}\n\
         Unit=proteus-{name}.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n",
    )
}

/// Boot / scheduled oneshot. Hardening block lifted from
/// `dist/systemd/proteus-rotate.service`.
fn render_oneshot_service(name: &str, exec: &str, wanted_by: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Proteus {name}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={exec}\n\
         User=root\n\
         \n\
         # Hardening (see systemd.exec(5)).\n\
         ProtectSystem=full\n\
         ProtectHome=true\n\
         PrivateTmp=true\n\
         NoNewPrivileges=true\n\
         CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
         SystemCallFilter=@system-service\n\
         \n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier=proteus\n\
         \n\
         [Install]\n\
         WantedBy={wanted_by}\n",
    )
}

/// Resume hook. Matches `dist/systemd/proteus-resume.service` — wired
/// into all four sleep targets so it fires for suspend, hibernate,
/// hybrid-sleep, and suspend-then-hibernate equally.
fn render_resume_service(name: &str, exec: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Proteus {name} on resume\n\
         After=suspend.target hibernate.target hybrid-sleep.target suspend-then-hibernate.target\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={exec}\n\
         User=root\n\
         \n\
         ProtectSystem=full\n\
         ProtectHome=true\n\
         PrivateTmp=true\n\
         NoNewPrivileges=true\n\
         CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
         SystemCallFilter=@system-service\n\
         \n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier=proteus-resume\n\
         \n\
         [Install]\n\
         WantedBy=sleep.target\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_systemd() {
        assert_eq!(Systemd::new().name(), "systemd");
    }

    #[test]
    fn detect_negative_on_empty_root() {
        let tmp = crate::testing::TempRoot::new("init-systemd-neg");
        let s = Systemd::with_root(&tmp.path);
        assert!(!s.detect());
    }

    #[test]
    fn detect_positive_when_run_systemd_system_exists() {
        let tmp = crate::testing::TempRoot::new("init-systemd-pos");
        std::fs::create_dir_all(tmp.path.join("run/systemd/system")).unwrap();
        let s = Systemd::with_root(&tmp.path);
        assert!(s.detect());
    }

    #[test]
    fn schedule_periodic_emits_timer_and_service() {
        let s = Systemd::new();
        let art = s
            .schedule_periodic("rotate", 7200, "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.ends_with("proteus-rotate.timer"));
        assert_eq!(art.mode, 0o644);
        assert!(art.content.contains("OnUnitActiveSec=7200"));
        assert!(art.content.contains("Unit=proteus-rotate.service"));
        // The service half is in the same blob.
        assert!(
            art.content
                .contains("ExecStart=/usr/local/bin/proteus rotate --yes")
        );
        assert!(art.content.contains("WantedBy=timers.target"));
    }

    #[test]
    fn schedule_periodic_rejects_zero_interval() {
        let s = Systemd::new();
        assert!(s.schedule_periodic("rotate", 0, "/x").is_err());
    }

    #[test]
    fn hook_resume_targets_sleep_target() {
        let s = Systemd::new();
        let art = s
            .hook_resume("rotate", "/usr/local/bin/proteus rotate --yes")
            .unwrap();
        assert!(art.path.ends_with("proteus-rotate-resume.service"));
        assert!(art.content.contains("WantedBy=sleep.target"));
        assert!(art.content.contains("After=suspend.target"));
    }

    #[test]
    fn hook_boot_targets_multi_user_target() {
        let s = Systemd::new();
        let art = s
            .hook_boot("apply", "/usr/local/bin/proteus apply --yes")
            .unwrap();
        assert!(art.path.ends_with("proteus-apply-boot.service"));
        assert!(art.content.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn invalid_name_rejected() {
        let s = Systemd::new();
        assert!(s.schedule_periodic("../etc/passwd", 60, "/x").is_err());
        assert!(s.hook_resume("a b", "/x").is_err());
        assert!(s.hook_boot("", "/x").is_err());
    }

    #[test]
    fn invalid_exec_rejected() {
        let s = Systemd::new();
        assert!(s.schedule_periodic("ok", 60, "").is_err());
        assert!(s.hook_resume("ok", "x\ny").is_err());
    }

    /// Issue #303: every periodic timer this generator emits must
    /// carry both `AccuracySec=` and `RandomizedDelaySec=` so the
    /// generated cadence is not itself a Proteus fingerprint
    /// observable across hosts.
    #[test]
    fn schedule_periodic_emits_jitter_directives() {
        let s = Systemd::new();
        for interval in [60, 300, 3_600, 7_200, 86_400] {
            let art = s.schedule_periodic("rotate", interval, "/x").unwrap();
            assert!(
                art.content.lines().any(|l| l.starts_with("AccuracySec=")),
                "interval={interval}: missing AccuracySec= (issue #303):\n{}",
                art.content
            );
            assert!(
                art.content
                    .lines()
                    .any(|l| l.starts_with("RandomizedDelaySec=")),
                "interval={interval}: missing RandomizedDelaySec= (issue #303):\n{}",
                art.content
            );
        }
    }
}
