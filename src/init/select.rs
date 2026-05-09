// SPDX-License-Identifier: GPL-3.0-or-later

//! Init-system selection. Walks Systemd → OpenRC → Runit → SysVinit →
//! s6 → dinit → POSIX-fallback and returns the first whose `detect()`
//! reports true.
//!
//! Systemd is the priority pick because (a) it's the primary target
//! for the project, (b) it's also the only init that runs on
//! Debian/Ubuntu/Fedora/RHEL — so a host that detects positive for
//! systemd plus something else is overwhelmingly a systemd host
//! with a vestigial /etc/init.d. The default fallback when nothing
//! detects (e.g. a sealed container with none of these probes
//! visible) is also systemd, since that produces the most
//! informative error from a downstream consumer.
//!
//! N11: the s6 / dinit / POSIX-fallback tail expands the hardcoded
//! list to cover Artix-s6, Adelie, Chimera, and a generic
//! "unknown but POSIX-compliant" host. The fallback rendering is a
//! plain shell script in `/usr/local/libexec/proteus` so an
//! installer always has something concrete to write, even on a
//! host where Proteus cannot identify the init.

use std::path::PathBuf;

use super::posix_fallback::{PosixFallback, PosixFlavor};
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
    // N11 — extended detection beyond the original four. Probe in
    // specificity order: a host with `/etc/s6/` and `/sbin/init`
    // should be classified as s6, not as posix-fallback.
    let root = PathBuf::from("/");
    if let Some(flavor) = PosixFallback::detect_flavor(&root) {
        return Box::new(PosixFallback::new(flavor));
    }
    let posix_unknown = PosixFallback::new(PosixFlavor::Unknown);
    if posix_unknown.detect() {
        return Box::new(posix_unknown);
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
        // N11: the new tail. `s6` / `dinit` / `posix-fallback` are
        // mutually exclusive in practice (a Chimera host has dinit
        // and not s6, etc.), but we report all three so the doctor
        // matrix is unambiguous.
        ("s6", PosixFallback::new(PosixFlavor::S6).detect()),
        ("dinit", PosixFallback::new(PosixFlavor::Dinit).detect()),
        (
            "posix-fallback",
            PosixFallback::new(PosixFlavor::Unknown).detect(),
        ),
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
            [
                "systemd",
                "openrc",
                "runit",
                "sysvinit",
                "s6",
                "dinit",
                "posix-fallback",
            ]
            .contains(&n),
            "unexpected init name: {n}"
        );
    }

    #[test]
    fn available_systems_includes_n11_extensions() {
        let m = available_systems();
        // N11 extended the matrix from 4 → 7 entries.
        assert_eq!(m.len(), 7);
        assert_eq!(m[0].0, "systemd");
        assert_eq!(m[1].0, "openrc");
        assert_eq!(m[2].0, "runit");
        assert_eq!(m[3].0, "sysvinit");
        assert_eq!(m[4].0, "s6");
        assert_eq!(m[5].0, "dinit");
        assert_eq!(m[6].0, "posix-fallback");
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
