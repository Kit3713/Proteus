// SPDX-License-Identifier: GPL-3.0-or-later

//! Init-system selection. Walks Systemd → OpenRC → Runit → SysVinit
//! and returns the first whose `detect()` reports true.
//!
//! Systemd is the priority pick because (a) it's the primary target
//! for the project, (b) it's also the only init that runs on
//! Debian/Ubuntu/Fedora/RHEL — so a host that detects positive for
//! systemd plus something else is overwhelmingly a systemd host
//! with a vestigial /etc/init.d. The default fallback when nothing
//! detects (e.g. a sealed container with none of these probes
//! visible) is also systemd, since that produces the most
//! informative error from a downstream consumer.

use super::{InitSystem, openrc::Openrc, runit::Runit, systemd::Systemd, sysvinit::Sysvinit};

/// Resolve which init system this host is using. Returns the first
/// impl whose `detect()` fires; defaults to Systemd if none do, so
/// callers can always render an artifact even on an exotic host.
pub fn detect() -> Box<dyn InitSystem> {
    let systemd = Systemd::new();
    if systemd.detect() {
        return Box::new(systemd);
    }
    let openrc = Openrc::new();
    if openrc.detect() {
        return Box::new(openrc);
    }
    let runit = Runit::new();
    if runit.detect() {
        return Box::new(runit);
    }
    let sysvinit = Sysvinit::new();
    if sysvinit.detect() {
        return Box::new(sysvinit);
    }
    Box::new(Systemd::new())
}

/// Probe each impl's `detect()` and return a list suitable for the
/// doctor matrix. Order matches selection priority so the rendered
/// output makes the auto-pick obvious. Mirrors
/// `crate::backend::select::availability_matrix`.
pub fn available_systems() -> Vec<(&'static str, bool)> {
    vec![
        ("systemd", Systemd::new().detect()),
        ("openrc", Openrc::new().detect()),
        ("runit", Runit::new().detect()),
        ("sysvinit", Sysvinit::new().detect()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_some_init_system() {
        // We can't predict which init system the test host runs, only
        // that the function never panics and always returns something.
        let chosen = detect();
        let n = chosen.name();
        assert!(
            ["systemd", "openrc", "runit", "sysvinit"].contains(&n),
            "unexpected init name: {n}"
        );
    }

    #[test]
    fn available_systems_orders_systemd_first() {
        let m = available_systems();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0].0, "systemd");
        assert_eq!(m[1].0, "openrc");
        assert_eq!(m[2].0, "runit");
        assert_eq!(m[3].0, "sysvinit");
    }

    #[test]
    fn available_systems_runs_without_panic() {
        // Sanity: probing every impl on the test host must not crash
        // (it runs without root, against whatever filesystem layout
        // the test host happens to have).
        let _ = available_systems();
    }

    #[test]
    fn detect_default_is_systemd_on_fedora_test_host() {
        // The CI runner and developer hosts are systemd-based; this
        // is a soft assertion that documents the expected default.
        // Skips on hosts where systemd isn't running.
        if std::path::Path::new("/run/systemd/system").is_dir() {
            assert_eq!(detect().name(), "systemd");
        }
    }
}
