// SPDX-License-Identifier: GPL-3.0-or-later

//! Driver selection. `auto` walks the candidates in priority order
//! (NM → networkd → raw) and returns the first whose `available()`
//! reports true; an explicit name short-circuits the probe.
//!
//! Priority rationale: NM is the established path and the only
//! fully-wired backend in this PR, so on a host with NM running
//! `auto` keeps current behaviour. networkd beats raw because the
//! DBus path is structured (per-connection drop-ins, cleaner revert)
//! versus raw's `ip`+`iw` which is the last-resort fallback.

use anyhow::{Result, bail};

use super::{NetworkBackend, networkd::NetworkdBackend, nm::NmBackend, raw::RawBackend};

/// Acceptable values of `[backend] driver` and the `--backend` flag
/// once it lands. Validated at config load via
/// [`is_valid_driver`].
pub const VALID_DRIVERS: &[&str] = &["auto", "nm", "networkd", "raw"];

/// Whether `driver` is one of [`VALID_DRIVERS`].
pub fn is_valid_driver(driver: &str) -> bool {
    VALID_DRIVERS.contains(&driver)
}

/// Resolve a driver string into a concrete backend. `"auto"` walks
/// NM → networkd → raw and returns the first available backend;
/// explicit names instantiate the matching backend regardless of
/// availability so the caller can surface a backend-specific error
/// from a real method (`available()` is allowed to be wrong, e.g. on
/// a brand-new init system that hasn't started networkd yet).
pub async fn select(driver: &str) -> Result<Box<dyn NetworkBackend>> {
    match driver {
        "auto" => select_auto().await,
        "nm" => Ok(Box::new(NmBackend::new())),
        "networkd" => Ok(Box::new(NetworkdBackend::new())),
        "raw" => Ok(Box::new(RawBackend::new())),
        other => bail!(
            "unknown backend driver '{other}'; expected one of {}; see proteus wiki backend",
            VALID_DRIVERS.join(", ")
        ),
    }
}

async fn select_auto() -> Result<Box<dyn NetworkBackend>> {
    let nm = NmBackend::new();
    if nm.available().await {
        return Ok(Box::new(nm));
    }
    let nd = NetworkdBackend::new();
    if nd.available().await {
        return Ok(Box::new(nd));
    }
    let raw = RawBackend::new();
    if raw.available().await {
        return Ok(Box::new(raw));
    }
    bail!(
        "no backend available — install NetworkManager, enable systemd-networkd, \
         or install iproute2 (`ip`) and re-run. See `proteus doctor`; see proteus wiki backend"
    )
}

/// Probe each candidate's `available()` and return a list suitable
/// for the doctor matrix. Order matches selection priority so the
/// rendered output makes the auto path obvious.
pub async fn availability_matrix() -> Vec<(&'static str, bool)> {
    let nm = NmBackend::new().available().await;
    let nd = NetworkdBackend::new().available().await;
    let raw = RawBackend::new().available().await;
    vec![("nm", nm), ("networkd", nd), ("raw", raw)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn is_valid_driver_accepts_documented_names() {
        for d in VALID_DRIVERS {
            assert!(is_valid_driver(d));
        }
        assert!(!is_valid_driver("garbage"));
        assert!(!is_valid_driver(""));
        assert!(!is_valid_driver("NM"));
    }

    #[test]
    fn select_auto_returns_some_backend_or_clear_error() {
        // We can't predict which backend the test host has, only that
        // `auto` either picks one or fails with a clear message. The
        // important behaviour is "no panic, no silent default".
        rt().block_on(async {
            match select("auto").await {
                Ok(b) => {
                    let n = b.name();
                    assert!(["nm", "networkd", "raw"].contains(&n));
                }
                Err(e) => {
                    let msg = format!("{e}");
                    assert!(msg.contains("no backend available"), "got: {msg}");
                }
            }
        });
    }

    #[test]
    fn select_garbage_driver_errors() {
        rt().block_on(async {
            // `Box<dyn NetworkBackend>` is not `Debug`, so we can't
            // call `.unwrap_err()`; pattern-match instead.
            match select("garbage").await {
                Ok(_) => panic!("expected error for unknown driver"),
                Err(e) => {
                    let msg = format!("{e}");
                    assert!(msg.contains("unknown backend driver"));
                    assert!(msg.contains("garbage"));
                }
            }
        });
    }

    #[test]
    fn select_explicit_nm_does_not_check_availability() {
        rt().block_on(async {
            let backend = select("nm").await.unwrap();
            assert_eq!(backend.name(), "nm");
        });
    }

    #[test]
    fn select_explicit_networkd_returns_networkd() {
        rt().block_on(async {
            let backend = select("networkd").await.unwrap();
            assert_eq!(backend.name(), "networkd");
        });
    }

    #[test]
    fn select_explicit_raw_returns_raw() {
        rt().block_on(async {
            let backend = select("raw").await.unwrap();
            assert_eq!(backend.name(), "raw");
        });
    }

    #[test]
    fn availability_matrix_orders_nm_networkd_raw() {
        rt().block_on(async {
            let m = availability_matrix().await;
            assert_eq!(m.len(), 3);
            assert_eq!(m[0].0, "nm");
            assert_eq!(m[1].0, "networkd");
            assert_eq!(m[2].0, "raw");
        });
    }
}
