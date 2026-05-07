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
use std::time::{Duration, Instant};

use serde::Serialize;

/// Per-endpoint TCP connect timeout. Same number used by `Probe protocol`
/// in the wiki (`wiki/probes.md`).
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
    let started = Instant::now();
    let addr = match resolve_first(target) {
        Ok(a) => a,
        Err(e) => return endpoint_error(target, started, e),
    };
    match TcpStream::connect_timeout(&addr, TCP_CONNECT_TIMEOUT) {
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
pub fn classify(results: &[EndpointResult], quorum_n: u8) -> Classification {
    let total = results.len() as u8;
    let successes = results.iter().filter(|r| r.ok).count() as u8;
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
pub fn build_report(results: Vec<EndpointResult>, quorum_n: u8, quorum_total: u8) -> ProbeReport {
    let successes = results.iter().filter(|r| r.ok).count() as u8;
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
}
