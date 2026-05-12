// SPDX-License-Identifier: GPL-3.0-or-later

//! Captive portal detector + classifier.
//!
//! Pure std-net (no reqwest) HTTP/1.0 GET against a known endpoint
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
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::CaptivePortalConfig;
use crate::display::display_string;

/// N12.14: hard ceiling on bytes accepted from the captive-portal
/// detector endpoint. A portal that returns more than this is either
/// misconfigured or hostile; 64 KiB is plenty for status line + headers
/// plus a status snippet, but small enough that a malicious peer can't
/// drive our memory by drip-feeding bytes inside the per-read timeout.
pub(crate) const MAX_RESPONSE_BYTES: usize = 65_536;

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
///
/// N9 (redirect following, HTTP-only): when the detector receives a
/// `3xx` with a `Location` header that points at another `http://`
/// URL, the request is retried against that target up to
/// [`MAX_REDIRECT_HOPS`] times before the response is classified.
/// `https://` redirects are surfaced as `PortalRequired` with the
/// validated target attached — TLS validation requires a TLS dep
/// (`rustls` / `webpki-roots`) which is explicitly forbidden in this
/// crate, so cross-scheme redirect-chasing is a deferred design
/// decision.
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
    match run_bounded(timeout, move || http_get_following(&parsed, timeout)) {
        Ok(GetResult { response, hops }) => {
            let outcome = classify_response(&response, expected_body);
            let note = if hops == 0 {
                outcome.1
            } else {
                format!("{} (after {} redirect hop(s))", outcome.1, hops)
            };
            DetectionOutcome {
                classification: outcome.0,
                note,
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

/// N9: maximum number of HTTP redirect hops the detector will follow
/// before giving up. Bounded small because (a) a real captive-portal
/// detector should resolve in 1–3 hops and (b) chasing a redirect
/// loop is an attacker's denial-of-service primitive.
pub(crate) const MAX_REDIRECT_HOPS: u8 = 5;

/// Result of a (possibly redirected) HTTP GET. `hops` is the number
/// of redirect-follows we performed before this response.
struct GetResult {
    response: HttpResponse,
    hops: u8,
}

/// HTTP-only redirect-following GET. Iterates up to
/// [`MAX_REDIRECT_HOPS`] times, parsing each `Location:` header
/// through [`validate_location_header`] before re-issuing the
/// request. Stops at the first non-3xx response, or when the
/// `Location` is missing/invalid/cross-scheme/over-cap.
fn http_get_following(parts: &UrlParts, timeout: Duration) -> std::io::Result<GetResult> {
    let mut current = parts.clone();
    let mut hops: u8 = 0;
    loop {
        let response = http_get(&current, timeout)?;
        if !(300..400).contains(&response.status) {
            return Ok(GetResult { response, hops });
        }
        if hops >= MAX_REDIRECT_HOPS {
            // Exceeded budget — return the last response so the
            // classifier can render it as `PortalRequired` with the
            // (already-sanitized-by-classifier) `Location` attached.
            return Ok(GetResult { response, hops });
        }
        let raw_location = match response.header("Location") {
            Some(s) => s.to_string(),
            None => return Ok(GetResult { response, hops }),
        };
        let next_url = match resolve_redirect_target(&current, &raw_location) {
            Some(u) => u,
            // Invalid, https, or otherwise un-followable: surface the
            // 3xx as-is so the classifier reports `PortalRequired`
            // with the sanitized URL in `redirect_target`.
            None => return Ok(GetResult { response, hops }),
        };
        current = next_url;
        hops += 1;
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

    // NEV2.2: empty expected_body paired with empty body is treated
    // as Unknown rather than Clear. Without this guard, an operator
    // who left `expected_response = ""` in their config could
    // mis-classify any blank response (e.g. a portal that returned a
    // bare 200 with no body) as a clean network. Treat the
    // both-empty case as inconclusive so the operator's misconfig
    // surfaces at the next read instead of as a false-positive.
    if expected_trimmed.is_empty() && body_trimmed.is_empty() && resp.status == 200 {
        return (
            Classification::Unknown,
            "empty expected_response paired with empty body — config likely missing".to_string(),
            None,
        );
    }

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
        // S2: validate the `Location:` header per RFC before handing
        // it to any downstream consumer. Reject CR/LF (header
        // injection), megabyte-sized junk (DoS), and refs that
        // aren't absolute or scheme-/root-relative.
        //
        // Issue #241: the `Location:` header is attacker-controlled — a
        // hostile portal can stuff ANSI escapes, BiDi overrides, NULs, or
        // a megabyte of junk into it and watch the operator's terminal
        // (and journald) render the lot. We layer S2's validator first
        // (drops the bad bytes outright) and `display_string` second
        // (escapes anything still printable that would deceive the
        // operator).
        let target = resp
            .header("Location")
            .and_then(|h| validate_location_header(h).map(display_string));
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

#[derive(Debug, Clone)]
struct UrlParts {
    host: String,
    port: u16,
    path: String,
}

/// Parse `http://[user[:pass]@]host[:port][/path]` into the pieces we need
/// for the request line. Hand-rolled so we don't pull in the `url` crate
/// for one URL per detect call. Supported forms (issues #138/#145):
///
/// * bare hostname — `http://example.com/`
/// * `host:port` — `http://example.com:8080/foo`
/// * IPv6 literal — `http://[::1]/check` or `http://[fe80::1]:8080/`
/// * userinfo — `http://user:pass@host/` (we strip and discard it;
///   the detector intentionally never sends Authorization)
///
/// `https://` and other schemes are rejected (we have no TLS).
fn parse_http_url(url: &str) -> Option<UrlParts> {
    use std::net::Ipv6Addr;
    use std::str::FromStr;

    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    // Strip optional `user[:pass]@` userinfo. We don't use it — the
    // detector is unauthenticated by construction — but URLs in the wild
    // often carry it via copy-paste, and a bare `@` would otherwise
    // corrupt the host.
    let hostport = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };

    // IPv6 literals are wrapped in brackets per RFC 3986 §3.2.2 so the
    // colons inside the address don't collide with the port separator.
    let (host, port) = if let Some(bracketed) = hostport.strip_prefix('[') {
        let (raw_host, after) = bracketed.split_once(']')?;
        if raw_host.is_empty() {
            return None;
        }
        // Validate it actually parses as an IPv6 address — anything else is
        // malformed and we'd rather fail loudly than DNS-resolve garbage.
        Ipv6Addr::from_str(raw_host).ok()?;
        let port = parse_port_suffix(after)?;
        (raw_host.to_string(), port)
    } else {
        match hostport.rfind(':') {
            Some(i) => {
                let p = hostport[i + 1..].parse::<u16>().ok()?;
                (hostport[..i].to_string(), p)
            }
            None => (hostport.to_string(), 80),
        }
    };
    if host.is_empty() {
        return None;
    }
    // Security audit M-3: refuse host or path that contains CR, LF, NUL,
    // or any ASCII control byte. Without this an attacker who controls
    // the configured `detect_url` (root-owned today, but config-distribution
    // mechanisms may relax that in the future) could inject extra HTTP
    // headers — `Host: target\r\nX-Smuggle: yes` — into the request line.
    if !is_request_safe(&host) || !is_request_safe(path) {
        return None;
    }
    Some(UrlParts {
        host,
        port,
        path: path.to_string(),
    })
}

/// True iff `s` contains no characters that would terminate or extend an
/// HTTP/1.0 request line beyond what the caller intended. Rejects CR, LF,
/// NUL, and the entire C0 control range.
fn is_request_safe(s: &str) -> bool {
    s.bytes().all(|b| b >= 0x20 && b != 0x7F)
}

/// S6: build the full HTTP/1.0 request blob from `parts` through a
/// single percent-encoder pass.
///
/// Previously the request line was assembled with two independent
/// `format!()` interpolations: one for the request-target (path) and
/// one for the `Host:` header. Either string passed
/// [`is_request_safe`] in isolation, but the concatenation could
/// still cross-contaminate — a path like `"/foo HTTP/1.0\r\n
/// X-Smuggle: yes\r\nDummy: "` paired with a benign host would slip
/// past the per-string control-byte gate (`is_request_safe` accepts
/// space and printable ASCII) and smuggle a fake header into the
/// request-line position. A single encoder pass over both fields
/// catches this class of bug because the encoder rejects/escapes any
/// byte that would end the request-target token (space, CR, LF, NUL,
/// `0x7F`).
///
/// The function returns the entire blob (request line + headers +
/// terminating `\r\n\r\n`) so callers cannot accidentally construct
/// a request that re-introduces the smuggling vector by skipping a
/// step.
fn request_line_builder(parts: &UrlParts) -> Result<String, &'static str> {
    let encoded_host = percent_encode_request_safe(&parts.host)?;
    let encoded_path = percent_encode_request_target(&parts.path)?;

    // Belt-and-braces: after encoding, every byte is one of [printable
    // ASCII minus space minus delimiters], so neither field can
    // possibly carry CR/LF/NUL into the request line.
    debug_assert!(encoded_host.bytes().all(|b| (0x21..=0x7E).contains(&b)));
    debug_assert!(encoded_path.bytes().all(|b| (0x21..=0x7E).contains(&b)));

    Ok(format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: proteus-portal-detect/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        encoded_path, encoded_host
    ))
}

/// Percent-encode any byte that would corrupt the HTTP request
/// target. Allows the unreserved set + the small punctuation that
/// `parse_http_url` already accepts in a path; everything else is
/// `%XX`-escaped. Returns `Err` only for empty input — encoding is
/// otherwise total.
fn percent_encode_request_target(s: &str) -> Result<String, &'static str> {
    if s.is_empty() {
        return Err("empty request target");
    }
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if is_path_safe_byte(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    Ok(out)
}

/// Percent-encode any byte not safe in the `Host:` header. The host
/// is generally already restricted to DNS-name + `:` + digits, plus
/// IPv6 literal characters (`[]:.0-9a-fA-F`); everything else gets
/// escaped. Returns `Err` only for empty input.
fn percent_encode_request_safe(s: &str) -> Result<String, &'static str> {
    if s.is_empty() {
        return Err("empty host");
    }
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if is_host_safe_byte(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    Ok(out)
}

/// Bytes that pass through the request-target encoder unchanged.
/// RFC 3986 unreserved + the small set of `pchar` punctuation that a
/// valid HTTP/1.0 path can carry. Crucially does NOT include space,
/// CR, LF, NUL, `?`, `#`, or any of the C0/C1 control range — those
/// are exactly the bytes that could smuggle a header into the
/// request line.
fn is_path_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            // unreserved
            b'-' | b'.' | b'_' | b'~'
                // sub-delims that pchar permits
                | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
                // segment delimiter + colon + at-sign + percent
                | b'/' | b':' | b'@' | b'%'
                // query / fragment markers — kept so a configured detect_url
                // with `?key=val` round-trips, but they do not introduce
                // request-line ambiguity because they're inside the encoded
                // request-target token.
                | b'?' | b'#'
        )
}

/// Bytes safe in a `Host:` header value. ASCII alphanumeric, dot,
/// hyphen, colon (port separator), and the IPv6-literal brackets.
/// Everything else escapes — including spaces and any C0 control.
fn is_host_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'[' | b']')
}

/// S2 + N9: validate a `Location:` header per RFC 7231 §7.1.2 well
/// enough to refuse the bad and accept the rest.
///
/// Returns `Some(URI)` when the header is suitable for redirect
/// following; `None` otherwise. Specifically:
///
/// - **Empty / whitespace-only** → `None`. RFC requires a URI-reference.
/// - **Length > 4096 bytes** → `None`. A megabyte of junk in `Location:`
///   is the classic attacker-DoS shape; cap at 4 KiB which fits any
///   real portal target.
/// - **CR/LF/NUL/control bytes** → `None`. Header injection.
/// - **No scheme and no leading `/`** → `None`. We require either
///   absolute (`http://...`, `https://...`), scheme-relative
///   (`//host/path`), or root-relative (`/path`). RFC 7231 also
///   permits plain relative refs but they're rare in `Location:` and
///   accepting them widens the parse surface.
/// - Anything else passes through unchanged. The caller is
///   responsible for resolving relative refs and rejecting schemes
///   it can't follow (the detector currently can only follow
///   `http://`).
pub(crate) fn validate_location_header(raw: &str) -> Option<&str> {
    const MAX_LOCATION_LEN: usize = 4096;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > MAX_LOCATION_LEN {
        return None;
    }
    // Reject CR/LF/NUL/all C0 controls + DEL. `is_request_safe` is
    // the same gate `parse_http_url` uses on host/path; reuse it so
    // the policy stays identical across every byte that touches an
    // HTTP request line.
    if !is_request_safe(trimmed) {
        return None;
    }
    // Categorise.
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("//")
        || trimmed.starts_with('/')
    {
        Some(trimmed)
    } else {
        None
    }
}

/// Resolve a `Location:` header value to a follow-able [`UrlParts`]
/// for the redirect-following GET path.
///
/// - Returns `None` for invalid headers (per
///   [`validate_location_header`]), `https://` targets (we have no
///   TLS), and root-relative refs that fail re-parsing.
/// - For absolute `http://` URLs we re-run [`parse_http_url`] so the
///   same `is_request_safe` gates apply.
/// - For `//host/path` (scheme-relative) we synthesize `http://` and
///   re-parse.
/// - For `/path` (root-relative) we keep the current host + port and
///   substitute the path.
fn resolve_redirect_target(current: &UrlParts, raw_location: &str) -> Option<UrlParts> {
    let validated = validate_location_header(raw_location)?;
    if let Some(rest) = validated.strip_prefix("http://") {
        let _ = rest;
        return parse_http_url(validated);
    }
    if validated.starts_with("https://") {
        // Cross-scheme redirect: cannot follow without TLS. Return
        // None so the caller surfaces the 3xx as `PortalRequired`
        // with the validated URL attached.
        return None;
    }
    if let Some(rest) = validated.strip_prefix("//") {
        let synthesized = format!("http://{rest}");
        return parse_http_url(&synthesized);
    }
    if validated.starts_with('/') {
        // Root-relative: same authority, swapped path. We still run
        // the path through the same is_request_safe gate as
        // parse_http_url to keep the policy uniform.
        if !is_request_safe(validated) {
            return None;
        }
        return Some(UrlParts {
            host: current.host.clone(),
            port: current.port,
            path: validated.to_string(),
        });
    }
    None
}

/// Helper for IPv6-literal port handling: `after` is the slice that comes
/// after the closing `]`, which is either empty (no port; default 80) or
/// `:NNN`.
fn parse_port_suffix(after: &str) -> Option<u16> {
    if after.is_empty() {
        return Some(80);
    }
    let port_str = after.strip_prefix(':')?;
    port_str.parse::<u16>().ok()
}

/// N12.7: build the `Host:` header value for an HTTP/1.0 request.
/// IPv6 literals MUST be bracketed per RFC 7230 §5.4 (`[::1]`,
/// `[fe80::1]:8080`); a bare colon-bearing host trips the
/// downstream parser at every well-behaved server. The default
/// port (80) is omitted to match the canonical Host shape.
///
/// `host` here is the raw host extracted in `parse_http_url`
/// (brackets already stripped); we re-add brackets when it parses
/// as an `Ipv6Addr`.
fn format_host_header(host: &str, port: u16) -> String {
    use std::net::Ipv6Addr;
    use std::str::FromStr;
    let bracketed = if Ipv6Addr::from_str(host).is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == 80 {
        bracketed
    } else {
        format!("{bracketed}:{port}")
    }
}

fn http_get(parts: &UrlParts, timeout: Duration) -> std::io::Result<HttpResponse> {
    let started = Instant::now();
    let addrs: Vec<std::net::SocketAddr> = (parts.host.as_str(), parts.port)
        .to_socket_addrs()?
        .collect();
    // N10: prefer IPv4 addresses first when the resolver returns a
    // mix. Many captive portals' `nmcheck` endpoints have v6 records
    // that route into a black hole on portals that only NAT v4; the
    // detector wastes its connect-budget on the v6 addr first under
    // the default kernel ordering. Stable-sort so within each family
    // the resolver's order is preserved.
    let mut ordered = addrs;
    ordered.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });
    let addr_iter = ordered.into_iter();
    let mut last_err: Option<std::io::Error> = None;
    // S6 (Wave 3B) + N12.7 (Wave 2): build the request via
    // `request_line_builder` (single percent-encoder pass over host +
    // path) and fold the IPv6-bracketed Host header from
    // `format_host_header` over the encoded host bytes. The
    // percent-encoder is the canonical source of safety for the
    // request line; the IPv6 brackets just make the resulting Host:
    // header RFC 7230 §5.4-compliant for literal addresses.
    let req = request_line_builder(parts).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("captive-portal request build: {e}"),
        )
    })?;
    let req = if parts.host.parse::<std::net::Ipv6Addr>().is_ok() {
        let host_for_header = format_host_header(&parts.host, parts.port);
        req.replace(
            &format!("Host: {}", parts.host),
            &format!("Host: {host_for_header}"),
        )
    } else {
        req
    };
    for addr in addr_iter {
        match TcpStream::connect_timeout(&addr, remaining_budget(started, timeout)?) {
            Ok(mut stream) => {
                stream.set_write_timeout(Some(remaining_budget(started, timeout)?))?;
                stream.write_all(req.as_bytes())?;
                let mut buf = Vec::with_capacity(4096);
                // N12.14: hard cap at MAX_RESPONSE_BYTES so a hostile or
                // misconfigured portal can't drive memory or CPU by streaming
                // megabytes of HTML at our detector. The response only needs
                // to be big enough to identify the portal (status line +
                // a few headers + a few hundred bytes of body); 64 KiB is
                // generous for that and still trivially small. Bytes past
                // the cap are dropped on the floor and the connection is
                // closed.
                let mut tmp = [0u8; 1024];
                while buf.len() < MAX_RESPONSE_BYTES {
                    // Re-check the budget per chunk: a peer that dribbles
                    // bytes just under the per-read timeout could otherwise
                    // keep us reading past `timeout`. This is the body-read
                    // half of issue #129.
                    stream.set_read_timeout(Some(remaining_budget(started, timeout)?))?;
                    let remaining = MAX_RESPONSE_BYTES - buf.len();
                    let slice_end = remaining.min(tmp.len());
                    match stream.read(&mut tmp[..slice_end]) {
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
    let (head, body) = body_slice(buf).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no header/body separator in response",
        )
    })?;
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

/// Split an HTTP/1.x response buffer into `(headers, body)` at the first
/// `\r\n\r\n` boundary. Returns `None` if the buffer does not contain a
/// terminator — the caller surfaces that as a parse error.
///
/// Roadmap P5: the previous inline form was
///
/// ```ignore
/// let (head, body) = buf.split_at(split);
/// let body = &body[4.min(body.len())..]; // skip the \r\n\r\n
/// ```
///
/// which silently truncated the body when the buffer ended exactly at the
/// separator (no body bytes) instead of returning an empty slice cleanly,
/// and which used the slightly-confusing `4.min(body.len())` pattern that
/// invited an off-by-one if a future maintainer ever adjusted it.
/// Hoisting the slice into its own function lets us test the boundary
/// shapes — empty body, separator at start, separator at end of buffer,
/// no separator — directly without driving the whole HTTP parser.
fn body_slice(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    let split = find_double_crlf(buf)?;
    let head = &buf[..split];
    // `find_double_crlf` matched 4 bytes starting at `split`, so
    // `split + 4 <= buf.len()`. No `min` needed; an explicit slice index
    // also makes the off-by-one impossible: body is "everything after the
    // 4-byte separator", which may be the empty slice.
    let body = &buf[split + 4..];
    Some((head, body))
}

/// R4 — captive-portal config reload primitive.
///
/// One `Arc<RwLock<CaptivePortalConfig>>` owned by the daemon and cloned
/// to every reader. Reloaders call [`Self::swap`] to replace the config
/// in-place; readers call [`Self::snapshot`] to take a value copy that
/// outlives the lock.
///
/// Wired from `src/commands/events.rs` on `SIGHUP` in a follow-up; the
/// primitive ships here so other event sources can adopt the same
/// reload contract without inventing their own.
pub struct CaptivePortalReload {
    inner: Arc<RwLock<CaptivePortalConfig>>,
}

impl CaptivePortalReload {
    pub fn new(cfg: CaptivePortalConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(cfg)),
        }
    }

    /// Take a value copy of the current config. Recovers from a
    /// poisoned reader by treating the inner value as still well-formed
    /// — `swap` replaces the whole struct in one assignment, so a
    /// writer panic can't leave a partial config behind.
    pub fn snapshot(&self) -> CaptivePortalConfig {
        match self.inner.read() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// Replace the stored config with `new_cfg`, returning the previous
    /// value. Poison-recovering for the same reason as [`Self::snapshot`].
    pub fn swap(&self, new_cfg: CaptivePortalConfig) -> CaptivePortalConfig {
        let mut g = match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        std::mem::replace(&mut *g, new_cfg)
    }

    /// Hand out the underlying `Arc` for callers that want raw access
    /// (e.g. a status reporter that wants to share the same lock).
    pub fn handle(&self) -> Arc<RwLock<CaptivePortalConfig>> {
        Arc::clone(&self.inner)
    }
}

impl Clone for CaptivePortalReload {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
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

    /// Issue #241 + S2: a hostile portal can stuff ANSI / BiDi /
    /// control bytes into the `Location:` header. The classifier
    /// runs the value through [`validate_location_header`] first; a
    /// header containing CR/LF/NUL/C0 controls is rejected outright
    /// (returned as `None`) rather than escaped, because the
    /// detector cannot trust *any* part of an attacker-controlled
    /// string that already injected bytes intended to break the
    /// operator's terminal or log scraper.
    #[test]
    fn redirect_target_strips_terminal_control_sequences() {
        let raw = "https://evil.example/\x1b[2J\x1b[31mfake\u{202e}.bank.com";
        let r = mk_resp(302, &[("Location", raw)], "");
        let (c, _, target) = classify_response(&r, "NetworkManager is online");
        assert_eq!(c, Classification::PortalRequired);
        assert!(
            target.is_none(),
            "hostile Location header must be rejected, got {target:?}"
        );
    }

    /// S2: a benign BiDi-override character in a *valid* https URL
    /// is still surfaced through the display-string escaper. The
    /// classifier passes the validated URL through
    /// `display_string`, which escapes the override codepoint so
    /// downstream renderers don't get spoofed.
    #[test]
    fn redirect_target_escapes_benign_bidi_override() {
        // No CR/LF/NUL — only a BiDi override codepoint, which is a
        // multibyte UTF-8 sequence that `is_request_safe` accepts
        // (every byte ≥ 0x20 and ≠ 0x7F).
        let raw = "https://example.com/path\u{202e}/back";
        let r = mk_resp(302, &[("Location", raw)], "");
        let (_, _, target) = classify_response(&r, "NetworkManager is online");
        let t = target.expect("benign-bytes Location must be captured");
        assert!(
            !t.chars().any(|c| c as u32 == 0x202e),
            "BiDi override must be escaped by display_string: {t:?}",
        );
        assert!(t.contains("\\u{202e}"));
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
    fn parses_ipv6_literal_with_default_port() {
        let p = parse_http_url("http://[::1]/check").unwrap();
        assert_eq!(p.host, "::1");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/check");
    }

    #[test]
    fn parses_ipv6_literal_with_explicit_port() {
        let p = parse_http_url("http://[fe80::1]:8080/foo").unwrap();
        assert_eq!(p.host, "fe80::1");
        assert_eq!(p.port, 8080);
        assert_eq!(p.path, "/foo");
    }

    #[test]
    fn parses_ipv6_literal_without_path() {
        let p = parse_http_url("http://[2001:db8::1]").unwrap();
        assert_eq!(p.host, "2001:db8::1");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/");
    }

    #[test]
    fn rejects_malformed_ipv6_brackets() {
        // Empty brackets, missing close bracket, non-IPv6 inside.
        assert!(parse_http_url("http://[]/").is_none());
        assert!(parse_http_url("http://[::1/").is_none());
        assert!(parse_http_url("http://[notipv6]/").is_none());
    }

    #[test]
    fn strips_userinfo_from_authority() {
        let p = parse_http_url("http://user:pass@host.example/check").unwrap();
        assert_eq!(p.host, "host.example");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/check");
    }

    #[test]
    fn strips_userinfo_with_port() {
        let p = parse_http_url("http://user@host.example:8080/").unwrap();
        assert_eq!(p.host, "host.example");
        assert_eq!(p.port, 8080);
    }

    #[test]
    fn strips_userinfo_with_ipv6_literal() {
        let p = parse_http_url("http://user:pass@[::1]:8080/foo").unwrap();
        assert_eq!(p.host, "::1");
        assert_eq!(p.port, 8080);
        assert_eq!(p.path, "/foo");
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

    /// Roadmap P5: cover every CRLF-boundary shape `body_slice` can
    /// encounter so a future tweak to `find_double_crlf` or the slice
    /// index can't reintroduce the off-by-one.
    #[test]
    fn body_slice_handles_crlf_boundaries() {
        // Normal: headers, separator, body.
        let (h, b) = body_slice(b"HTTP/1.0 200 OK\r\nFoo: bar\r\n\r\nhello").unwrap();
        assert_eq!(h, b"HTTP/1.0 200 OK\r\nFoo: bar");
        assert_eq!(b, b"hello");

        // Buffer ends exactly at the separator — body is empty, not truncated.
        let (h, b) = body_slice(b"HTTP/1.0 204 No Content\r\n\r\n").unwrap();
        assert_eq!(h, b"HTTP/1.0 204 No Content");
        assert_eq!(b, b"");

        // Buffer starts with the separator (degenerate but valid input).
        let (h, b) = body_slice(b"\r\n\r\nbody").unwrap();
        assert_eq!(h, b"");
        assert_eq!(b, b"body");

        // Buffer is exactly the separator and nothing else.
        let (h, b) = body_slice(b"\r\n\r\n").unwrap();
        assert_eq!(h, b"");
        assert_eq!(b, b"");

        // Buffer with body containing later \r\n\r\n is preserved (we
        // split on the first occurrence only).
        let (h, b) = body_slice(b"H\r\n\r\nx\r\n\r\ny").unwrap();
        assert_eq!(h, b"H");
        assert_eq!(b, b"x\r\n\r\ny");

        // Trailing partial separator — no full CRLFCRLF, must be None.
        assert!(body_slice(b"H\r\n\r").is_none());
        assert!(body_slice(b"H\r\n").is_none());
        assert!(body_slice(b"\r").is_none());
        assert!(body_slice(b"").is_none());

        // Body containing a single byte right after the separator.
        let (_, b) = body_slice(b"a\r\n\r\nz").unwrap();
        assert_eq!(b, b"z");
    }

    /// Roadmap P5: drive the full parser at the same boundaries to prove
    /// the body_slice extraction didn't break anything downstream.
    #[test]
    fn parse_http_response_handles_empty_body() {
        let resp = parse_http_response(b"HTTP/1.0 204 No Content\r\nFoo: bar\r\n\r\n").unwrap();
        assert_eq!(resp.status, 204);
        assert_eq!(resp.body, "");
        assert_eq!(resp.header("Foo"), Some("bar"));
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

    // === S2 (validate Location header) ===

    #[test]
    fn validate_location_accepts_absolute_http_and_https() {
        assert_eq!(
            validate_location_header("http://example.com/check"),
            Some("http://example.com/check")
        );
        assert_eq!(
            validate_location_header("https://login.example/portal"),
            Some("https://login.example/portal")
        );
    }

    #[test]
    fn validate_location_accepts_scheme_relative_and_root_relative() {
        assert_eq!(
            validate_location_header("//example.com/x"),
            Some("//example.com/x")
        );
        assert_eq!(
            validate_location_header("/portal/login"),
            Some("/portal/login")
        );
    }

    #[test]
    fn validate_location_trims_surrounding_whitespace() {
        assert_eq!(
            validate_location_header("   http://example.com/   "),
            Some("http://example.com/")
        );
    }

    #[test]
    fn validate_location_rejects_empty_and_whitespace() {
        assert!(validate_location_header("").is_none());
        assert!(validate_location_header("   ").is_none());
    }

    #[test]
    fn validate_location_rejects_relative_refs() {
        // RFC 7231 also permits plain relative refs but the detector
        // intentionally narrows the parse surface.
        assert!(validate_location_header("portal/login").is_none());
        assert!(validate_location_header("./next").is_none());
    }

    #[test]
    fn validate_location_rejects_header_injection() {
        // CR / LF in a Location header is the classic header-injection
        // primitive. Reject outright.
        assert!(validate_location_header("http://x/\r\nX-Smuggle: yes").is_none());
        assert!(validate_location_header("http://x/\nFoo: bar").is_none());
        assert!(validate_location_header("http://x/\rFoo: bar").is_none());
        // NUL.
        assert!(validate_location_header("http://x/\0evil").is_none());
        // ESC + other C0.
        assert!(validate_location_header("http://x/\x1b[2J").is_none());
        assert!(validate_location_header("http://x/\x07").is_none());
        // DEL.
        assert!(validate_location_header("http://x/\x7f").is_none());
    }

    #[test]
    fn validate_location_caps_oversized_input() {
        let huge = "http://example.com/".to_string() + &"a".repeat(8_000);
        assert!(validate_location_header(&huge).is_none());
        // Just under cap is still accepted.
        let ok = "http://example.com/".to_string() + &"a".repeat(2_000);
        assert!(validate_location_header(&ok).is_some());
    }

    // === S6 (single-pass percent-encoder for request line) ===

    #[test]
    fn request_line_builder_round_trip_simple() {
        let p = parse_http_url("http://nmcheck.gnome.org/check_network_status.txt").unwrap();
        let line = request_line_builder(&p).unwrap();
        assert!(line.starts_with("GET /check_network_status.txt HTTP/1.0\r\n"));
        assert!(line.contains("\r\nHost: nmcheck.gnome.org\r\n"));
        assert!(line.ends_with("\r\n\r\n"));
    }

    /// S6: the single-pass encoder must escape any byte in either
    /// host or path that would let an attacker terminate the request
    /// line and inject another header. `parse_http_url` already
    /// rejects most of these via `is_request_safe`, but the encoder
    /// is the second line of defence.
    #[test]
    fn request_line_builder_encodes_unsafe_path_bytes() {
        // Synthesize a UrlParts directly to bypass parse_http_url's
        // pre-filter — the encoder must catch hostile bytes regardless
        // of how the parts were assembled.
        let p = UrlParts {
            host: "example.com".into(),
            port: 80,
            // Space, CR, LF — exactly the bytes that would smuggle
            // a header into the request-line position.
            path: "/foo bar\r\nX-Smuggle: yes".into(),
        };
        let line = request_line_builder(&p).unwrap();
        // First line must end with " HTTP/1.0\r\n"; nothing earlier
        // can carry a raw CR/LF/space into the request line.
        let first_line = line.split("\r\n").next().unwrap();
        assert!(first_line.ends_with(" HTTP/1.0"), "got {first_line:?}");
        // The hostile bytes are %-encoded.
        assert!(
            first_line.contains("%20") || first_line.contains("%0D") || first_line.contains("%0A"),
            "expected percent-encoding of hostile bytes; got {first_line:?}",
        );
        // No raw smuggled header.
        assert!(!line.contains("X-Smuggle: yes\r\n"));
    }

    #[test]
    fn request_line_builder_encodes_unsafe_host_bytes() {
        let p = UrlParts {
            host: "example.com\r\nX-Smuggle: yes".into(),
            port: 80,
            path: "/check".into(),
        };
        let line = request_line_builder(&p).unwrap();
        assert!(!line.contains("X-Smuggle: yes\r\n"));
        // Host header line ends with \r\n, then User-Agent follows.
        assert!(line.contains("\r\nUser-Agent: proteus-portal-detect/1.0\r\n"));
    }

    /// S6: IPv6 literal hosts should round-trip through the encoder
    /// without their `:` bytes being percent-encoded (colon is in
    /// the host-safe set). The bracket question is N12.7 territory
    /// (Stream 4); for now we just pin that the encoder doesn't
    /// corrupt the address itself.
    #[test]
    fn request_line_builder_preserves_ipv6_address_bytes() {
        let p = parse_http_url("http://[::1]:8080/check").unwrap();
        let line = request_line_builder(&p).unwrap();
        assert!(line.contains("Host: ::1\r\n"), "got line: {line:?}");
    }

    // === N9 (HTTP-only redirect following) ===

    #[test]
    fn resolve_redirect_target_follows_http_absolute() {
        let cur = parse_http_url("http://a.example/").unwrap();
        let r = resolve_redirect_target(&cur, "http://b.example:8080/x").unwrap();
        assert_eq!(r.host, "b.example");
        assert_eq!(r.port, 8080);
        assert_eq!(r.path, "/x");
    }

    #[test]
    fn resolve_redirect_target_refuses_https_when_no_tls() {
        let cur = parse_http_url("http://a.example/").unwrap();
        // We can't follow https:// without a TLS dep — return None
        // so the caller surfaces the 3xx as PortalRequired instead.
        assert!(resolve_redirect_target(&cur, "https://b.example/x").is_none());
    }

    #[test]
    fn resolve_redirect_target_handles_root_relative() {
        let cur = parse_http_url("http://a.example:8081/old").unwrap();
        let r = resolve_redirect_target(&cur, "/portal/login").unwrap();
        assert_eq!(r.host, "a.example");
        assert_eq!(r.port, 8081);
        assert_eq!(r.path, "/portal/login");
    }

    #[test]
    fn resolve_redirect_target_handles_scheme_relative() {
        let cur = parse_http_url("http://a.example/").unwrap();
        let r = resolve_redirect_target(&cur, "//b.example:9090/y").unwrap();
        assert_eq!(r.host, "b.example");
        assert_eq!(r.port, 9090);
        assert_eq!(r.path, "/y");
    }

    #[test]
    fn resolve_redirect_target_rejects_header_injection() {
        let cur = parse_http_url("http://a.example/").unwrap();
        assert!(resolve_redirect_target(&cur, "http://b/\r\nX: y").is_none());
        assert!(resolve_redirect_target(&cur, "/safe-looking\nX: y").is_none());
    }

    #[test]
    fn max_redirect_hops_is_bounded_small() {
        // Pin the bound so a refactor can't accidentally let the
        // detector chase a redirect loop forever.
        const _: () = assert!(MAX_REDIRECT_HOPS <= 10);
        const _: () = assert!(MAX_REDIRECT_HOPS >= 1);
    }

    // === R4 (SIGHUP reload primitive) ===

    fn cfg(detect_url: &str, timeout_secs: u64) -> CaptivePortalConfig {
        CaptivePortalConfig {
            enabled: true,
            detect_url: detect_url.into(),
            expected_response: "NetworkManager is online".into(),
            policy: "rotate-before-auth".into(),
            fresh_mac_per_visit: true,
            timeout_secs,
        }
    }

    #[test]
    fn reload_snapshot_is_a_coherent_clone() {
        let r = CaptivePortalReload::new(cfg("http://a/", 5));
        let s = r.snapshot();
        assert_eq!(s.detect_url, "http://a/");
        assert_eq!(s.timeout_secs, 5);
    }

    #[test]
    fn reload_swap_replaces_config_atomically() {
        let r = CaptivePortalReload::new(cfg("http://a/", 5));
        let prev = r.swap(cfg("http://b/", 10));
        assert_eq!(prev.detect_url, "http://a/");
        assert_eq!(prev.timeout_secs, 5);
        let now = r.snapshot();
        assert_eq!(now.detect_url, "http://b/");
        assert_eq!(now.timeout_secs, 10);
    }

    #[test]
    fn reload_clone_shares_underlying_config() {
        // Two clones of the handle must observe each other's swaps —
        // this is the whole point of the primitive (one daemon-side
        // reload, many task-side readers).
        let a = CaptivePortalReload::new(cfg("http://a/", 5));
        let b = a.clone();
        let _ = a.swap(cfg("http://b/", 10));
        assert_eq!(b.snapshot().detect_url, "http://b/");
        assert_eq!(b.snapshot().timeout_secs, 10);
    }

    #[test]
    fn reload_handle_round_trip() {
        // The internal `Arc` is exposed via `handle()` so tests can
        // assert pointer-identity across clones. Mostly a smoke test
        // that the primitive is composable with anything that wants
        // a raw `Arc<RwLock<...>>` (e.g. a status reporter).
        let r = CaptivePortalReload::new(cfg("http://a/", 5));
        let h = r.handle();
        assert_eq!(h.read().unwrap().detect_url, "http://a/");
    }

    #[test]
    fn reload_recovers_from_poisoned_writer() {
        let r = CaptivePortalReload::new(cfg("http://a/", 5));
        let r_for_thread = r.clone();
        let _ = std::thread::spawn(move || {
            let _g = r_for_thread.inner.write().unwrap();
            panic!("synthetic poison");
        })
        .join();
        assert!(r.inner.is_poisoned());
        // Subsequent snapshot/swap must still succeed.
        let s = r.snapshot();
        assert_eq!(s.detect_url, "http://a/");
        let _ = r.swap(cfg("http://recovered/", 7));
        assert_eq!(r.snapshot().detect_url, "http://recovered/");
    }

    /// N12.7: bare IPv4 / hostname Host header passes through
    /// unchanged. The default port (80) is omitted to match the
    /// canonical Host header shape.
    #[test]
    fn host_header_for_ipv4_and_hostname_is_unchanged() {
        assert_eq!(
            format_host_header("nmcheck.gnome.org", 80),
            "nmcheck.gnome.org"
        );
        assert_eq!(
            format_host_header("nmcheck.gnome.org", 8080),
            "nmcheck.gnome.org:8080"
        );
        assert_eq!(format_host_header("203.0.113.5", 80), "203.0.113.5");
    }

    /// N12.7: IPv6 literals in the Host header MUST be bracketed
    /// per RFC 7230 §5.4. Without the brackets, a server's URL
    /// parser rejects the request line with a 400.
    #[test]
    fn host_header_for_ipv6_literal_is_bracketed() {
        assert_eq!(format_host_header("::1", 80), "[::1]");
        assert_eq!(format_host_header("::1", 8080), "[::1]:8080");
        assert_eq!(format_host_header("fe80::1", 80), "[fe80::1]");
        assert_eq!(format_host_header("2001:db8::1", 443), "[2001:db8::1]:443");
    }

    /// NEV2.2: an empty `expected_body` paired with an empty
    /// response body must classify as `Unknown` rather than `Clear`
    /// (which is what the previous code did because `"".trim() ==
    /// "".trim()` always wins). Pin so an operator who forgot to
    /// configure `expected_response` sees the misconfig instead of
    /// a false-positive clean signal.
    #[test]
    fn empty_expected_body_with_empty_body_is_unknown() {
        let r = mk_resp(200, &[], "");
        let (c, _, _) = classify_response(&r, "");
        assert_eq!(c, Classification::Unknown);
    }

    /// NEV2.2 mirror: a non-empty body still goes through the
    /// normal classifier even when `expected_body` is empty —
    /// avoids regressing the clear path for users who wired the
    /// config correctly.
    #[test]
    fn empty_expected_body_with_real_body_still_classifies() {
        let r = mk_resp(200, &[], "anything");
        let (c, _, _) = classify_response(&r, "");
        // 200 with body that doesn't match the (empty) expected ->
        // PortalRequired (existing splash-page rule), not Clear.
        assert_eq!(c, Classification::PortalRequired);
    }
}
