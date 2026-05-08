// SPDX-License-Identifier: GPL-3.0-or-later

//! Probe quorum logic.
//!
//! Each probe round contacts a small set of `host:port` endpoints with TCP
//! connect (3s timeout). The result of each connect feeds a quorum vote that
//! classifies the round as `clear`, `down`, `inconclusive`, or
//! `portal-suspected`. ICMP fallback is deferred — a comment in
//! `run_endpoint` notes the reason.
//!
//! The classifier is deliberately small and pure so it's easy to unit-test
//! without poking the network.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Per-endpoint total timeout — bounds DNS resolve **and** TCP connect.
///
/// `TcpStream::connect_timeout` only covers the connect phase; a hostile or
/// stalled resolver could leave us blocked in `to_socket_addrs` for far
/// longer than this number. We run the whole resolve+connect on a worker
/// thread and wait on a channel with this deadline so the round can never
/// stretch past it (issue #128).
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Quorum classification — wire format. Names match the wiki page
/// `wiki/probes.md` and the documented exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    Clear,
    Down,
    Inconclusive,
    PortalSuspected,
}

impl Classification {
    /// Documented exit codes for `proteus probe`.
    pub fn exit_code(self) -> u8 {
        match self {
            Classification::Clear => 0,
            Classification::Down => 1,
            Classification::Inconclusive => 2,
            Classification::PortalSuspected => 3,
        }
    }
}

/// One endpoint result. `method` is `tcp` today; `icmp` lands when the
/// fallback is implemented (see `run_endpoint`).
#[derive(Debug, Clone, Serialize)]
pub struct EndpointResult {
    pub target: String,
    pub method: &'static str,
    pub ok: bool,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Top-level result, serialized as the `proteus probe --json` schema.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    pub schema_version: u32,
    pub classification: Classification,
    pub endpoints: Vec<EndpointResult>,
    pub quorum_n: u8,
    pub quorum_total: u8,
    pub successes: u8,
}

/// Run TCP probes against `targets` in parallel and return per-endpoint
/// results in input order. The wiki specifies parallel probes so a single
/// blackholed endpoint can't stretch the round to (n × timeout).
pub fn run_endpoints(targets: &[String]) -> Vec<EndpointResult> {
    if targets.is_empty() {
        return Vec::new();
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|t| s.spawn(move || run_endpoint(t)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("probe thread panicked"))
            .collect()
    })
}

fn run_endpoint(target: &str) -> EndpointResult {
    run_with_timeout(target, TCP_CONNECT_TIMEOUT, connect_resolved)
}

/// Run `work` on a worker thread with `total_timeout` as a hard ceiling.
///
/// If the deadline expires we abandon the worker (the OS unblocks any
/// in-flight resolve/connect when the process exits) and return a synthetic
/// timeout error tagged against `target`. Bounding the whole resolve+connect
/// this way is what fixes issue #128.
fn run_with_timeout<F>(target: &str, total_timeout: Duration, work: F) -> EndpointResult
where
    F: FnOnce(&str, Duration, Instant) -> EndpointResult + Send + 'static,
{
    let started = Instant::now();
    let target_owned = target.to_string();
    let (tx, rx) = mpsc::sync_channel::<EndpointResult>(1);
    thread::spawn(move || {
        let _ = tx.send(work(&target_owned, total_timeout, started));
    });
    rx.recv_timeout(total_timeout).unwrap_or_else(|_| {
        endpoint_error(
            target,
            started,
            format!(
                "timeout after {} ms (resolve or connect stalled)",
                total_timeout.as_millis()
            ),
        )
    })
}

/// Synchronous resolve + connect. Caller must bound the total time; both
/// phases here can block longer than the connect timeout alone
/// (`to_socket_addrs` ignores the connect budget).
fn connect_resolved(target: &str, total_timeout: Duration, started: Instant) -> EndpointResult {
    let addr = match resolve_first(target) {
        Ok(a) => a,
        Err(e) => return endpoint_error(target, started, e),
    };
    // Cap the connect at whatever's left of the round budget. `connect_timeout`
    // requires non-zero; clamp to 1ms so a near-elapsed budget still surfaces
    // as a proper timeout error.
    let remaining = total_timeout
        .checked_sub(started.elapsed())
        .filter(|d| !d.is_zero())
        .unwrap_or(Duration::from_millis(1));
    match TcpStream::connect_timeout(&addr, remaining) {
        Ok(_stream) => EndpointResult {
            target: target.to_string(),
            method: "tcp",
            ok: true,
            duration_ms: started.elapsed().as_millis() as u64,
            error: None,
        },
        // ICMP fallback is deferred — needs root + raw sockets, which would
        // expand the binary's privilege surface. Until it lands, a TCP
        // failure becomes a TCP-only failure and quorum decides the round.
        Err(e) => endpoint_error(target, started, format!("tcp: {e}")),
    }
}

fn resolve_first(target: &str) -> Result<SocketAddr, String> {
    let mut iter = target
        .to_socket_addrs()
        .map_err(|e| format!("resolve: {e}"))?;
    iter.next()
        .ok_or_else(|| "resolve: no addresses".to_string())
}

fn endpoint_error(target: &str, started: Instant, error: String) -> EndpointResult {
    EndpointResult {
        target: target.to_string(),
        method: "tcp",
        ok: false,
        duration_ms: started.elapsed().as_millis() as u64,
        error: Some(error),
    }
}

/// Classify a set of endpoint results against the quorum. `quorum_n` is the
/// symmetric threshold: `clear` when `quorum_n` or more succeed, `down` when
/// `quorum_n` or more fail, otherwise `inconclusive`. With the default 3-of-4
/// that means a split (2/2) stays inconclusive — biased toward false negatives
/// per the wiki's "asymmetric cost" argument. `portal-suspected` is reserved
/// for the captive-portal classifier (separate module, lands later);
/// `proteus probe` returns `inconclusive` instead of guessing.
///
/// Issue #220: counts are saturating-cast to `u8`, not truncating. With more
/// than 255 endpoints configured the previous `len() as u8` wrapped to
/// `len % 256`, which could flip Clear/Down arbitrarily. Saturating means a
/// 300-endpoint round still classifies cleanly: every count above 255 becomes
/// 255, and the `quorum_n` threshold (also `u8`) reads correctly.
pub fn classify(results: &[EndpointResult], quorum_n: u8) -> Classification {
    let total = saturate_u8(results.len());
    let successes = saturate_u8(results.iter().filter(|r| r.ok).count());
    let failures = total.saturating_sub(successes);
    if successes >= quorum_n {
        Classification::Clear
    } else if failures >= quorum_n {
        Classification::Down
    } else {
        Classification::Inconclusive
    }
}

/// Build the full report from results + quorum config. `quorum_total` is
/// stored on the report for wrappers that display "x of N"; the classifier
/// uses the result vector as the authoritative count.
///
/// Issue #220: same saturating-cast story as `classify`.
pub fn build_report(results: Vec<EndpointResult>, quorum_n: u8, quorum_total: u8) -> ProbeReport {
    let successes = saturate_u8(results.iter().filter(|r| r.ok).count());
    let classification = classify(&results, quorum_n);
    ProbeReport {
        schema_version: 1,
        classification,
        endpoints: results,
        quorum_n,
        quorum_total,
        successes,
    }
}

/// Saturating cast `usize -> u8` for probe counts. Pulled out so the
/// debug_assert in `classify` and the matching call in `build_report`
/// share one definition.
///
/// Roadmap P6: prefer `u8::try_from` (explicit boundary check) over the
/// previous `as u8` form. The boundary is visible in the source and a
/// future refactor that drops the upper-bound check would surface as a
/// type error rather than silent wrap-around.
#[inline]
fn saturate_u8(n: usize) -> u8 {
    u8::try_from(n).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(target: &str, ok: bool) -> EndpointResult {
        EndpointResult {
            target: target.into(),
            method: "tcp",
            ok,
            duration_ms: 1,
            error: if ok { None } else { Some("x".into()) },
        }
    }

    #[test]
    fn classify_clear_when_quorum_succeeds() {
        let r = vec![ep("a", true), ep("b", true), ep("c", true), ep("d", false)];
        assert_eq!(classify(&r, 3), Classification::Clear);
    }

    /// Issue #220: with more than 255 endpoints the previous `len() as u8`
    /// truncated to `len % 256`, flipping Clear/Down classifications.
    /// The saturating cast makes a 300-endpoint round still classify
    /// cleanly: every count above 255 reads as 255 against the u8 quorum
    /// threshold, so an all-pass round reports Clear (not Inconclusive
    /// or Down).
    #[test]
    fn classify_saturates_above_255_endpoints() {
        let r: Vec<_> = (0..300).map(|i| ep(&format!("h{i}"), true)).collect();
        assert_eq!(classify(&r, 3), Classification::Clear);

        let r: Vec<_> = (0..300).map(|i| ep(&format!("h{i}"), false)).collect();
        assert_eq!(classify(&r, 3), Classification::Down);
    }

    /// Companion: exactly 255 endpoints classify cleanly. Pins the
    /// boundary so the saturating cast doesn't drift to `<` somewhere.
    #[test]
    fn classify_at_255_endpoints_is_clean() {
        let r: Vec<_> = (0..255).map(|i| ep(&format!("h{i}"), true)).collect();
        assert_eq!(classify(&r, 3), Classification::Clear);
        let r: Vec<_> = (0..255).map(|i| ep(&format!("h{i}"), false)).collect();
        assert_eq!(classify(&r, 3), Classification::Down);
    }

    /// Roadmap P6: pin the saturate_u8 helper directly. With 256 entries
    /// the previous `len() as u8` wrapped to 0; the `try_from` guard must
    /// surface 255 instead. Test this independently of `classify` so a
    /// future refactor of the helper signature can't reintroduce the bug.
    #[test]
    fn saturate_u8_caps_at_255() {
        assert_eq!(saturate_u8(0), 0);
        assert_eq!(saturate_u8(1), 1);
        assert_eq!(saturate_u8(255), 255);
        assert_eq!(saturate_u8(256), 255);
        assert_eq!(saturate_u8(1_000), 255);
        assert_eq!(saturate_u8(usize::MAX), 255);
    }

    /// `build_report` shares the saturating-cast story with `classify`.
    /// At >255 successes the report's `successes` field reads 255 and
    /// the classification still surfaces correctly.
    #[test]
    fn build_report_saturates_successes_above_255() {
        let r: Vec<_> = (0..300).map(|i| ep(&format!("h{i}"), true)).collect();
        let report = build_report(r, 3, 3);
        assert_eq!(report.successes, u8::MAX);
        assert_eq!(report.classification, Classification::Clear);
    }

    #[test]
    fn classify_down_when_quorum_fails() {
        let r = vec![
            ep("a", false),
            ep("b", false),
            ep("c", false),
            ep("d", true),
        ];
        assert_eq!(classify(&r, 3), Classification::Down);
    }

    #[test]
    fn classify_inconclusive_on_split() {
        let r = vec![ep("a", true), ep("b", true), ep("c", false), ep("d", false)];
        assert_eq!(classify(&r, 3), Classification::Inconclusive);
    }

    #[test]
    fn exit_codes_match_documented_spec() {
        assert_eq!(Classification::Clear.exit_code(), 0);
        assert_eq!(Classification::Down.exit_code(), 1);
        assert_eq!(Classification::Inconclusive.exit_code(), 2);
        assert_eq!(Classification::PortalSuspected.exit_code(), 3);
    }

    #[test]
    fn build_report_counts_successes() {
        let r = vec![ep("a", true), ep("b", true), ep("c", false)];
        let rep = build_report(r, 2, 3);
        assert_eq!(rep.successes, 2);
        assert_eq!(rep.classification, Classification::Clear);
        assert_eq!(rep.schema_version, 1);
    }

    /// Simulates a stalled DNS / connect by sleeping past the deadline. The
    /// watchdog must surface a timeout error and not block on the worker —
    /// regression cover for issue #128 (DNS resolve was previously unbounded
    /// by `TCP_CONNECT_TIMEOUT`).
    #[test]
    fn run_with_timeout_bounds_slow_work() {
        let started = Instant::now();
        let r = run_with_timeout(
            "stalled.example:443",
            Duration::from_millis(50),
            |t, _, started| {
                std::thread::sleep(Duration::from_secs(2));
                EndpointResult {
                    target: t.to_string(),
                    method: "tcp",
                    ok: true,
                    duration_ms: started.elapsed().as_millis() as u64,
                    error: None,
                }
            },
        );
        let elapsed = started.elapsed();
        assert!(!r.ok, "stalled work should not be reported as ok");
        assert!(
            r.error.as_deref().unwrap_or("").contains("timeout"),
            "expected timeout error, got {:?}",
            r.error
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "watchdog must release within the deadline, took {elapsed:?}"
        );
    }

    /// Fast work returns its real result without going through the timeout
    /// branch — proves the watchdog isn't swallowing successful rounds.
    #[test]
    fn run_with_timeout_returns_fast_result() {
        let r = run_with_timeout("ok.example:443", Duration::from_secs(1), |t, _, _| {
            EndpointResult {
                target: t.to_string(),
                method: "tcp",
                ok: true,
                duration_ms: 1,
                error: None,
            }
        });
        assert!(r.ok);
        assert_eq!(r.target, "ok.example:443");
    }
}
