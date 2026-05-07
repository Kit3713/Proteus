// SPDX-License-Identifier: GPL-3.0-or-later

//! Captive portal detector + classifier.
//!
//! Phase C. Pure std-net (no reqwest) HTTP/1.0 GET against a known endpoint
//! (default `nmcheck.gnome.org`). Compares the body to an expected string and
//! classifies the path as `clear`, `portal-required`, `portal-authed`, or
//! `unknown`.
//!
//! Why std-net:
//! - Adding `reqwest` (or even `ureq`) costs hundreds of KB of binary size.
//! - We only need GET, no TLS, no chunked encoding, no Connection: keep-alive.
//! - Detection has to work behind a portal, so HTTP/1.0 with `Connection: close`
//!   is actually the most portal-friendly thing to send.
//!
//! The classifier is split out so the unit tests can exercise it without doing
//! any network I/O.
//!
//! Public surface: [`detect`], [`Classification`], [`DetectionOutcome`].

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Classification of the network path according to the portal detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    /// Detector got the expected response. Internet works, no portal in path.
    Clear,
    /// Traffic intercepted, user has not authed yet.
    PortalRequired,
    /// User has authed, but portal infrastructure remains in path.
    PortalAuthed,
    /// Detector inconclusive (TCP failure, timeout, parse error).
    Unknown,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::PortalRequired => "portal-required",
            Self::PortalAuthed => "portal-authed",
            Self::Unknown => "unknown",
        }
    }
}

/// Result of running the detector once.
#[derive(Debug, Clone, Serialize)]
pub struct DetectionOutcome {
    pub classification: Classification,
    pub note: String,
    pub redirect_target: Option<String>,
}

/// Minimal parsed response. Pub(crate) so the classifier tests can build one.
#[derive(Debug, Clone)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Run the detector against `url`, comparing the response body to
/// `expected_body`. Returns an outcome with classification + note.
///
/// `timeout` is a **total** budget — DNS resolve, TCP connect, write, and
/// body-read all run on a worker thread that we abandon if the deadline
/// expires. Per-socket read/write timeouts cover the steady-state case;
/// the watchdog covers DNS resolution and any phase that ignores the
/// per-socket timeout (issue #129).
pub fn detect(url: &str, expected_body: &str, timeout: Duration) -> DetectionOutcome {
    let parsed = match parse_http_url(url) {
        Some(p) => p,
        None => {
            return DetectionOutcome {
                classification: Classification::Unknown,
                note: format!("invalid detect_url '{url}'"),
                redirect_target: None,
            };
        }
    };
    match run_bounded(timeout, move || http_get(&parsed, timeout)) {
        Ok(resp) => {
            let outcome = classify_response(&resp, expected_body);
            DetectionOutcome {
                classification: outcome.0,
                note: outcome.1,
                redirect_target: outcome.2,
            }
        }
        Err(e) => DetectionOutcome {
            classification: Classification::Unknown,
            note: format!("tcp/io error: {e}"),
            redirect_target: None,
        },
    }
}

/// Run `work` on a worker thread bounded by `total_timeout`. If the worker
/// is still running when the deadline expires we abandon it (the OS unblocks
/// any in-flight resolve/connect/read when the process exits) and surface a
/// synthetic `TimedOut` error. This is what fixes issue #129 —
/// `set_read_timeout` on the underlying socket only bounds *individual*
/// `read` calls, not the cumulative DNS + connect + write + body-read.
fn run_bounded<F, T>(total_timeout: Duration, work: F) -> std::io::Result<T>
where
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<std::io::Result<T>>(1);
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(total_timeout).unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "captive-portal detect timed out after {} ms",
                total_timeout.as_millis()
            ),
        ))
    })
}

/// Pure classifier — split out so tests don't need a network.
///
/// Returns (classification, note, optional redirect target).
pub(crate) fn classify_response(
    resp: &HttpResponse,
    expected_body: &str,
) -> (Classification, String, Option<String>) {
    let expected_trimmed = expected_body.trim();
    let body_trimmed = resp.body.trim();

    // 200 with the exact expected body.
    if resp.status == 200 && body_trimmed == expected_trimmed {
        // Some authed walled gardens leave portal-shaped headers in flight.
        if has_portal_shaped_headers(resp) {
            return (
                Classification::PortalAuthed,
                "expected body but portal-shaped headers present".to_string(),
                None,
            );
        }
        return (
            Classification::Clear,
            "expected response received".to_string(),
            None,
        );
    }

    // 3xx redirects almost always mean a portal redirect.
    if (300..400).contains(&resp.status) {
        let target = resp.header("Location").map(|s| s.to_string());
        return (
            Classification::PortalRequired,
            format!("HTTP {} redirect", resp.status),
            target,
        );
    }

    // 200 but with a different body — still portal-required (typical splash page).
    if resp.status == 200 {
        return (
            Classification::PortalRequired,
            "200 with unexpected body (likely splash page)".to_string(),
            None,
        );
    }

    // Other status codes — inconclusive.
    (
        Classification::Unknown,
        format!("HTTP {} from detector endpoint", resp.status),
        None,
    )
}

fn has_portal_shaped_headers(resp: &HttpResponse) -> bool {
    // WWW-Authenticate strongly suggests challenge-response auth in the path.
    if resp.header("WWW-Authenticate").is_some() {
        return true;
    }
    // X-Captive-Portal / X-Cwiserve etc — vendor-specific markers seen in the wild.
    if resp.header("X-Captive-Portal").is_some() {
        return true;
    }
    false
}

#[derive(Debug)]
struct UrlParts {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Option<UrlParts> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rfind(':') {
        Some(i) => {
            let p = hostport[i + 1..].parse::<u16>().ok()?;
            (hostport[..i].to_string(), p)
        }
        None => (hostport.to_string(), 80),
    };
    if host.is_empty() {
        return None;
    }
    Some(UrlParts {
        host,
        port,
        path: path.to_string(),
    })
}

fn http_get(parts: &UrlParts, timeout: Duration) -> std::io::Result<HttpResponse> {
    let started = Instant::now();
    let addr_iter = (parts.host.as_str(), parts.port).to_socket_addrs()?;
    let mut last_err: Option<std::io::Error> = None;
    for addr in addr_iter {
        match TcpStream::connect_timeout(&addr, remaining_budget(started, timeout)?) {
            Ok(mut stream) => {
                let req = format!(
                    "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: proteus-portal-detect/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
                    parts.path, parts.host
                );
                stream.set_write_timeout(Some(remaining_budget(started, timeout)?))?;
                stream.write_all(req.as_bytes())?;
                let mut buf = Vec::with_capacity(4096);
                // Cap at 64 KiB — portals don't need megabytes of HTML to identify.
                let mut tmp = [0u8; 1024];
                while buf.len() < 65_536 {
                    // Re-check the budget per chunk: a peer that dribbles
                    // bytes just under the per-read timeout could otherwise
                    // keep us reading past `timeout`. This is the body-read
                    // half of issue #129.
                    stream.set_read_timeout(Some(remaining_budget(started, timeout)?))?;
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(e) => return Err(e),
                    }
                }
                return parse_http_response(&buf);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no addresses resolved",
        )
    }))
}

/// How much of `total` is left given that `started` has already elapsed.
/// Errors with `TimedOut` once the budget is gone — the caller bubbles that
/// up instead of asking the kernel for a 0-duration timeout (which means
/// "block forever" for `set_read_timeout`).
fn remaining_budget(started: Instant, total: Duration) -> std::io::Result<Duration> {
    match total.checked_sub(started.elapsed()) {
        Some(d) if !d.is_zero() => Ok(d),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("budget exhausted after {} ms", total.as_millis()),
        )),
    }
}

fn parse_http_response(buf: &[u8]) -> std::io::Result<HttpResponse> {
    let split = find_double_crlf(buf).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no header/body separator in response",
        )
    })?;
    let (head, body) = buf.split_at(split);
    let body = &body[4.min(body.len())..]; // skip the \r\n\r\n
    let head_str = std::str::from_utf8(head).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 response headers")
    })?;
    let mut lines = head_str.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "empty response"))?;
    let status = parse_status_line(status_line)?;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let body = String::from_utf8_lossy(body).into_owned();
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn parse_status_line(line: &str) -> std::io::Result<u16> {
    // "HTTP/1.0 200 OK" — second whitespace-separated field is the code.
    let mut it = line.split_whitespace();
    let _proto = it.next();
    let code = it
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no status code"))?;
    code.parse::<u16>().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("bad status code '{code}'"),
        )
    })
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_resp(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_string(),
        }
    }

    #[test]
    fn classifies_exact_match_as_clear() {
        let r = mk_resp(200, &[], "NetworkManager is online\n");
        let (c, _, _) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::Clear);
    }

    #[test]
    fn classifies_redirect_as_portal_required() {
        let r = mk_resp(302, &[("Location", "https://login.example/portal")], "");
        let (c, _, target) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::PortalRequired);
        assert_eq!(target.as_deref(), Some("https://login.example/portal"));
    }

    #[test]
    fn classifies_200_with_different_body_as_portal_required() {
        let r = mk_resp(200, &[], "<html>splash</html>");
        let (c, _, _) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::PortalRequired);
    }

    #[test]
    fn classifies_expected_body_with_auth_header_as_portal_authed() {
        let r = mk_resp(
            200,
            &[("WWW-Authenticate", "Basic realm=portal")],
            "NetworkManager is online\n",
        );
        let (c, _, _) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::PortalAuthed);
    }

    #[test]
    fn body_compare_is_whitespace_tolerant() {
        // Real nmcheck endpoint returns "NetworkManager is online\n" (with newline).
        // Config might or might not include the trailing newline.
        let r = mk_resp(200, &[], "  NetworkManager is online  \n");
        let (c, _, _) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::Clear);
    }

    #[test]
    fn parses_simple_http_url() {
        let p = parse_http_url("http://nmcheck.gnome.org/check_network_status.txt").unwrap();
        assert_eq!(p.host, "nmcheck.gnome.org");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/check_network_status.txt");
    }

    #[test]
    fn parses_http_url_with_port() {
        let p = parse_http_url("http://example.com:8080/foo").unwrap();
        assert_eq!(p.host, "example.com");
        assert_eq!(p.port, 8080);
        assert_eq!(p.path, "/foo");
    }

    #[test]
    fn rejects_https_and_garbage() {
        assert!(parse_http_url("https://example.com/").is_none());
        assert!(parse_http_url("notaurl").is_none());
    }

    #[test]
    fn classification_as_str_matches_kebab_case() {
        assert_eq!(Classification::Clear.as_str(), "clear");
        assert_eq!(Classification::PortalRequired.as_str(), "portal-required");
        assert_eq!(Classification::PortalAuthed.as_str(), "portal-authed");
        assert_eq!(Classification::Unknown.as_str(), "unknown");
    }

    /// Slow body-read or stalled DNS must not exceed the configured timeout.
    /// Regression cover for issue #129.
    #[test]
    fn run_bounded_surfaces_timeout_for_slow_work() {
        let started = Instant::now();
        let r: std::io::Result<()> = run_bounded(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_secs(2));
            Ok(())
        });
        let elapsed = started.elapsed();
        assert_eq!(r.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(
            elapsed < Duration::from_millis(500),
            "watchdog must release within the deadline, took {elapsed:?}"
        );
    }

    #[test]
    fn run_bounded_returns_fast_result() {
        let r: std::io::Result<u32> = run_bounded(Duration::from_secs(1), || Ok(42));
        assert_eq!(r.unwrap(), 42);
    }

    #[test]
    fn remaining_budget_yields_timeout_when_exhausted() {
        let t0 = Instant::now() - Duration::from_secs(10);
        let r = remaining_budget(t0, Duration::from_secs(1));
        assert_eq!(r.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn remaining_budget_returns_what_is_left() {
        let r = remaining_budget(Instant::now(), Duration::from_secs(5)).unwrap();
        assert!(r > Duration::from_millis(100), "got {r:?}");
        assert!(r <= Duration::from_secs(5), "got {r:?}");
    }
}
