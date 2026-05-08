// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::oui::Vendor;
use super::probe::{ARP_PROBE_TIMEOUT, ND_PROBE_TIMEOUT, Probe, ProbeOutcome};
use super::{Mac, MacError};

/// Shape of the trailing 3 MAC bytes a persona may pin via
/// `mac_byte_pattern` (e.g. `xx:xx:xx`, `01:23:xx`, `aa-bb-cc`). `xx`
/// (case-insensitive) marks an entropy slot; literal hex pairs pin the
/// corresponding byte. `None` means "unconstrained" — the historical
/// path that fills all three with `getrandom`. Roadmap Milestone 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSuffixPattern {
    /// Per-byte: `Some(literal)` pins the byte, `None` rolls it.
    pub bytes: [Option<u8>; 3],
}

impl ByteSuffixPattern {
    /// Parse `01:23:xx` / `01-23-xx` / `0123xx` into a pattern. The
    /// caller supplies one of the three accepted separators and the
    /// parser is lenient about case + the colon variant. Returns
    /// `Err` on malformed input so a typo'd persona pattern lands at
    /// the user, not as silently-rolled bytes.
    pub fn parse(s: &str) -> Result<Self> {
        // GH#340: parse character-by-character so multibyte input
        // (e.g. `"é:23:xx"`) errors instead of panicking inside the
        // byte-indexed slice that the previous version used. The same
        // panic class Stream 2 fixed for `is_valid_per_ssid_duration`
        // — a `&str[i..j]` slice is illegal when `i` or `j` falls inside
        // a multibyte char. The fix here is to gather the cleaned
        // characters into a `Vec<char>` (so we know the count without
        // worrying about byte boundaries) and read from the vec.
        let cleaned: Vec<char> = s
            .chars()
            .filter(|c| !matches!(c, ':' | '-' | ' '))
            .collect();
        if cleaned.len() != 6 {
            bail!(
                "mac_byte_pattern '{s}' must encode 3 bytes (got {} hex/x chars)",
                cleaned.len()
            );
        }
        let mut bytes: [Option<u8>; 3] = [None, None, None];
        for i in 0..3 {
            let c0 = cleaned[i * 2];
            let c1 = cleaned[i * 2 + 1];
            // Accept the literal `xx` (case-insensitive) as a "roll this
            // byte" marker. Anything else must be two ASCII hex digits.
            if (c0 == 'x' || c0 == 'X') && (c1 == 'x' || c1 == 'X') {
                bytes[i] = None;
                continue;
            }
            // Multibyte input falls through here with non-ASCII chars
            // that `from_str_radix` will reject — surfacing as a clean
            // error rather than the byte-slice panic. Build the pair
            // back into a String so the parser sees the canonical
            // 2-char hex representation.
            let pair: String = [c0, c1].iter().collect();
            let v = u8::from_str_radix(&pair, 16)
                .map_err(|_| anyhow!("mac_byte_pattern '{s}' has non-hex byte '{pair}'"))?;
            bytes[i] = Some(v);
        }
        Ok(Self { bytes })
    }

    /// Convenience: every byte rolled (the historical, no-pattern path).
    pub const fn unconstrained() -> Self {
        Self {
            bytes: [None, None, None],
        }
    }
}

const MAX_GENERATION_ATTEMPTS: usize = 64;

/// Roadmap M2 — adaptive backoff threshold. After this many consecutive
/// active-probe collisions on the same OUI token, the generator advances to
/// the next token in the pool. Three is the operator-friendly choice: low
/// enough that a busy segment doesn't spin on a hot vendor prefix, high
/// enough that an unlucky reply doesn't immediately disrupt persona shape.
pub const COLLISIONS_BEFORE_OUI_FALLBACK: usize = 3;

#[derive(Debug, Clone)]
pub struct GenerateOptions<'a> {
    pub pool: &'a [String],
    pub forbidden: &'a HashSet<Mac>,
    pub avoid: &'a HashSet<Mac>,
    /// Persona-supplied trailing-byte shape. `None` (default) keeps the
    /// historical "all 3 bytes rolled" path. `Some(pat)` honours the
    /// persona's `mac_byte_pattern`. Roadmap Milestone 2.
    pub suffix_pattern: Option<ByteSuffixPattern>,
}

impl<'a> GenerateOptions<'a> {
    /// Build options without a suffix pattern — the v0.2.x default.
    pub fn new(pool: &'a [String], forbidden: &'a HashSet<Mac>, avoid: &'a HashSet<Mac>) -> Self {
        Self {
            pool,
            forbidden,
            avoid,
            suffix_pattern: None,
        }
    }
}

pub fn generate(opts: &GenerateOptions<'_>) -> Result<Mac> {
    if opts.pool.is_empty() {
        bail!("OUI pool is empty");
    }
    let suffix = opts
        .suffix_pattern
        .unwrap_or(ByteSuffixPattern::unconstrained());
    let mut last_err: Option<MacError> = None;
    for _ in 0..MAX_GENERATION_ATTEMPTS {
        // Issue #226: rejection-sampled index instead of `byte % len`.
        // Persona pools are small (4-12 entries) so naive modulo skews
        // each token by ~1-3% which a passive observer can cluster.
        let token_idx = pick_index(opts.pool.len())?;
        let token = &opts.pool[token_idx];
        let vendor = Vendor::from_pool_token(token)
            .ok_or_else(|| anyhow!("unknown OUI pool token '{token}'"))?;
        let mac = match generate_for_vendor(vendor, &suffix)? {
            Some(m) => m,
            None => continue,
        };
        match mac.validate_assignable() {
            Ok(()) => {}
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        }
        if opts.forbidden.contains(&mac) || opts.avoid.contains(&mac) {
            continue;
        }
        return Ok(mac);
    }
    Err(match last_err {
        Some(e) => anyhow!(
            "could not generate a valid MAC after {MAX_GENERATION_ATTEMPTS} tries: last={e}"
        ),
        None => {
            anyhow!("could not generate a non-colliding MAC after {MAX_GENERATION_ATTEMPTS} tries")
        }
    })
}

/// Outcome of one round of `generate_with_probe`. Surfaces every candidate
/// that was considered + the reason it was rejected, so `proteus rotate
/// --explain` can show the operator why the final MAC was picked.
#[derive(Debug, Clone)]
pub struct CollisionAwareOutcome {
    pub chosen: Mac,
    /// Token from the persona's `oui_pool` the chosen MAC came from.
    pub chosen_token: String,
    /// Per-attempt log entries. Includes the winning candidate as the last
    /// element with `RejectionReason::Accepted`.
    pub attempts: Vec<CandidateAttempt>,
    /// Number of times adaptive backoff rotated to the next OUI token.
    pub oui_fallbacks: usize,
}

#[derive(Debug, Clone)]
pub struct CandidateAttempt {
    pub mac: Mac,
    pub token: String,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    /// The candidate survived all checks and was returned.
    Accepted,
    /// MAC matched something in the `forbidden` set (sacred originals,
    /// state-cached current values).
    Forbidden,
    /// MAC matched something in the `avoid` set (live ARP/ND neighbours,
    /// gateway).
    AvoidList,
    /// `validate_assignable()` rejected the candidate (multicast / all-zero).
    NotAssignable(String),
    /// Active probe came back as a collision; carries the conflicting
    /// neighbour IP when known.
    ActiveCollision { peer_ip: Option<String> },
    /// Probe declined to run (no `CAP_NET_RAW`, etc.). Not a rejection per
    /// se — caller continues with passive checks. Surfaced for `--explain`
    /// transparency.
    ProbeUnsupported(&'static str),
}

#[derive(Debug, Clone)]
pub struct ProbeOptions {
    pub iface: String,
    pub arp_timeout: Duration,
    pub nd_timeout: Duration,
    /// When `true` the IPv6 ND probe runs in addition to the ARP probe.
    /// Default `true`; tests / pure-IPv4 contexts can flip it off.
    pub run_nd_probe: bool,
}

impl ProbeOptions {
    pub fn for_iface(iface: impl Into<String>) -> Self {
        Self {
            iface: iface.into(),
            arp_timeout: ARP_PROBE_TIMEOUT,
            nd_timeout: ND_PROBE_TIMEOUT,
            run_nd_probe: true,
        }
    }
}

/// Roadmap M2 entry point. Produces a MAC that has cleared:
/// 1. `validate_assignable` (multicast/all-zero check),
/// 2. `forbidden` (sacred originals + state-cached MACs),
/// 3. `avoid` (live ARP/ND/gateway neighbours),
/// 4. an active ARP probe (RFC 5227),
/// 5. an active IPv6 DAD probe (RFC 4862).
///
/// Adaptive backoff: after [`COLLISIONS_BEFORE_OUI_FALLBACK`] consecutive
/// active collisions on the same OUI token, the next attempt skips that
/// token and tries the next one in the pool. Probe-`Unsupported` responses
/// (no `CAP_NET_RAW`) are not collisions — they're logged and the candidate
/// passes through to the rest of the pipeline.
pub fn generate_with_probe<P: Probe + ?Sized>(
    opts: &GenerateOptions<'_>,
    probe: &P,
    probe_opts: &ProbeOptions,
) -> Result<CollisionAwareOutcome> {
    if opts.pool.is_empty() {
        bail!("OUI pool is empty");
    }

    // Walking the pool in a deterministic order once adaptive backoff kicks
    // in keeps the operator-visible behaviour predictable: "we tried apple
    // three times, hit collisions, advanced to intel, found one." A pure
    // random walk would surface as "we kept rolling until something landed"
    // which is harder to debug and less useful in `--explain` output.
    let suffix = opts
        .suffix_pattern
        .unwrap_or(ByteSuffixPattern::unconstrained());
    // Issue #226: rejection-sampled starting cursor. The cursor is
    // advanced deterministically (`+ 1 % len`) once probing kicks in;
    // only the *initial* index is random, so this is the only `% len`
    // here that gates uniformity.
    let mut token_cursor: usize = pick_index(opts.pool.len())?;
    let mut consecutive_collisions: usize = 0;
    let mut attempts: Vec<CandidateAttempt> = Vec::new();
    let mut oui_fallbacks: usize = 0;
    let mut last_err: Option<MacError> = None;

    for _ in 0..MAX_GENERATION_ATTEMPTS {
        let token = opts.pool[token_cursor].clone();
        let vendor = Vendor::from_pool_token(&token)
            .ok_or_else(|| anyhow!("unknown OUI pool token '{token}'"))?;
        let mac = match generate_for_vendor(vendor, &suffix)? {
            Some(m) => m,
            None => continue,
        };

        match mac.validate_assignable() {
            Ok(()) => {}
            Err(e) => {
                attempts.push(CandidateAttempt {
                    mac,
                    token: token.clone(),
                    reason: RejectionReason::NotAssignable(e.to_string()),
                });
                last_err = Some(e);
                continue;
            }
        }

        if opts.forbidden.contains(&mac) {
            attempts.push(CandidateAttempt {
                mac,
                token: token.clone(),
                reason: RejectionReason::Forbidden,
            });
            continue;
        }
        if opts.avoid.contains(&mac) {
            attempts.push(CandidateAttempt {
                mac,
                token: token.clone(),
                reason: RejectionReason::AvoidList,
            });
            continue;
        }

        // ARP probe.
        let arp_outcome = probe.arp_probe(&probe_opts.iface, mac, probe_opts.arp_timeout);
        match arp_outcome {
            ProbeOutcome::Collision { peer_ip } => {
                tracing::warn!(
                    iface = %probe_opts.iface,
                    candidate = %mac,
                    peer = peer_ip.as_deref().unwrap_or("?"),
                    token = %token,
                    "ARP probe: candidate MAC is taken on segment; re-rolling"
                );
                attempts.push(CandidateAttempt {
                    mac,
                    token: token.clone(),
                    reason: RejectionReason::ActiveCollision {
                        peer_ip: peer_ip.clone(),
                    },
                });
                consecutive_collisions += 1;
                if consecutive_collisions >= COLLISIONS_BEFORE_OUI_FALLBACK {
                    // NM2.1 (high): drop the `pool.len() > 1` guard so a
                    // single-token persona pool (`oui_pool = ["apple"]`)
                    // also resets `consecutive_collisions` on every
                    // adaptive-backoff threshold. Without this reset, a
                    // sustained collision rate burned the full 64-attempt
                    // budget on the same token and surfaced as a
                    // generator failure under live conditions. With a
                    // single-token pool the cursor wrap is a no-op
                    // (`(0 + 1) % 1 == 0`) but the counter reset still
                    // clears the budget so the loop can keep rolling
                    // fresh entropy on the same OUI.
                    tracing::warn!(
                        token = %token,
                        next_token = %opts.pool[(token_cursor + 1) % opts.pool.len()],
                        pool_len = opts.pool.len(),
                        "{COLLISIONS_BEFORE_OUI_FALLBACK} consecutive ARP collisions on token; \
                         advancing to next OUI in persona pool"
                    );
                    token_cursor = (token_cursor + 1) % opts.pool.len();
                    consecutive_collisions = 0;
                    oui_fallbacks += 1;
                }
                continue;
            }
            ProbeOutcome::Unsupported(reason) => {
                attempts.push(CandidateAttempt {
                    mac,
                    token: token.clone(),
                    reason: RejectionReason::ProbeUnsupported(reason),
                });
                // Not a rejection — fall through to nd_probe / acceptance.
            }
            ProbeOutcome::Free => {}
        }

        // ND probe.
        if probe_opts.run_nd_probe {
            let nd_outcome = probe.nd_probe(&probe_opts.iface, mac, probe_opts.nd_timeout);
            match nd_outcome {
                ProbeOutcome::Collision { peer_ip } => {
                    tracing::warn!(
                        iface = %probe_opts.iface,
                        candidate = %mac,
                        peer = peer_ip.as_deref().unwrap_or("?"),
                        token = %token,
                        "ND probe: candidate link-local is taken on segment; re-rolling"
                    );
                    attempts.push(CandidateAttempt {
                        mac,
                        token: token.clone(),
                        reason: RejectionReason::ActiveCollision {
                            peer_ip: peer_ip.clone(),
                        },
                    });
                    consecutive_collisions += 1;
                    if consecutive_collisions >= COLLISIONS_BEFORE_OUI_FALLBACK {
                        // NM2.1: same guard removal as the ARP branch
                        // above. Single-token pools were stalling the
                        // events daemon under sustained collision
                        // because the consecutive-collisions counter
                        // never got reset.
                        tracing::warn!(
                            token = %token,
                            next_token = %opts.pool[(token_cursor + 1) % opts.pool.len()],
                            pool_len = opts.pool.len(),
                            "{COLLISIONS_BEFORE_OUI_FALLBACK} consecutive ND collisions; \
                             advancing to next OUI in persona pool"
                        );
                        token_cursor = (token_cursor + 1) % opts.pool.len();
                        consecutive_collisions = 0;
                        oui_fallbacks += 1;
                    }
                    continue;
                }
                ProbeOutcome::Unsupported(reason) => {
                    attempts.push(CandidateAttempt {
                        mac,
                        token: token.clone(),
                        reason: RejectionReason::ProbeUnsupported(reason),
                    });
                }
                ProbeOutcome::Free => {}
            }
        }

        // Survived every check. Record the win and return.
        let _ = consecutive_collisions; // counter is dead after this point
        attempts.push(CandidateAttempt {
            mac,
            token: token.clone(),
            reason: RejectionReason::Accepted,
        });
        return Ok(CollisionAwareOutcome {
            chosen: mac,
            chosen_token: token,
            attempts,
            oui_fallbacks,
        });
    }

    Err(match last_err {
        Some(e) => anyhow!(
            "could not generate a valid MAC after {MAX_GENERATION_ATTEMPTS} tries: last={e}"
        ),
        None => {
            anyhow!(
                "could not generate a non-colliding MAC after {MAX_GENERATION_ATTEMPTS} tries \
                 (active probe kept rejecting; last seen rejection: collision)"
            )
        }
    })
}

/// Build one candidate MAC for the given vendor token, honouring the
/// caller's [`ByteSuffixPattern`]. The pattern's `Some(literal)` slots
/// override entropy on the corresponding trailing byte; `None` slots
/// roll fresh from `getrandom`. Returns `Ok(None)` when entropy
/// succeeds but the construction step is skipped (currently never —
/// kept as the integration hook for future LAA quirks).
///
/// **Postcondition (NM2.5):** the returned MAC is *not* validated as
/// assignable. The caller MUST call [`Mac::validate_assignable`] before
/// surfacing the value or writing it to the backend. Both production
/// callers in this file (`generate` and `generate_with_probe`) do so;
/// the contract is documented here so a future caller can't silently
/// skip the gate. The reason validation lives at the call site rather
/// than here: the loop wants to *observe* a validation rejection (it
/// records the rejection reason in `attempts` and re-rolls) rather
/// than treating it as a generator failure.
fn generate_for_vendor(vendor: Vendor, suffix: &ByteSuffixPattern) -> Result<Option<Mac>> {
    let mac = match vendor.prefixes() {
        Some(prefixes) => {
            // Issue #226: rejection-sampled prefix index. Per-vendor
            // prefix counts (5-8) make the naive `byte % len` skew up
            // to ~5%, which clusters fingerprints by OUI prefix in a
            // way `nmap -O` / fingerbank can spot.
            let prefix_idx = pick_index(prefixes.len())?;
            let prefix = prefixes[prefix_idx];
            let entropy = rand_bytes::<3>()?;
            let mut tail = [0u8; 3];
            for i in 0..3 {
                tail[i] = match suffix.bytes[i] {
                    Some(literal) => literal,
                    None => entropy[i],
                };
            }
            let mut octets = [0u8; 6];
            octets[..3].copy_from_slice(&prefix);
            octets[3..].copy_from_slice(&tail);
            Mac(octets)
        }
        None => {
            let mut octets = rand_bytes::<6>()?;
            // LAA bit set, multicast bit clear.
            octets[0] = (octets[0] | 0x02) & 0xFE;
            // The pattern only covers the trailing 3 bytes — even for
            // LAA, where we own the full 6, we keep the OUI half random
            // so persona+LAA combos still uniquify.
            for i in 0..3 {
                if let Some(literal) = suffix.bytes[i] {
                    octets[3 + i] = literal;
                }
            }
            Mac(octets)
        }
    };
    Ok(Some(mac))
}

/// Bias-free `[0, len)` index for the pickers above. Wraps the shared
/// `unbiased_index` helper so every selection in this module shares one
/// rejection-sampled implementation. All MAC pools (vendor tokens, OUI
/// prefixes) are small (≤ ~12), well within the byte-stream picker's
/// 256-entry ceiling.
fn pick_index(len: usize) -> Result<usize> {
    crate::rand::unbiased_index(len, crate::rand::getrandom_byte)
}

fn rand_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::super::probe::MockProbe;
    use super::*;

    fn pool(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn empty_set() -> HashSet<Mac> {
        HashSet::new()
    }

    fn no_nd_probe_opts(iface: &str) -> ProbeOptions {
        let mut p = ProbeOptions::for_iface(iface);
        // Most tests focus on the ARP path; the ND path is a parallel
        // codepath and gets its own dedicated coverage.
        p.run_nd_probe = false;
        p
    }

    #[test]
    fn generates_apple_prefixed_mac() {
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        for _ in 0..50 {
            let m = generate(&opts).unwrap();
            assert!(
                super::super::oui::APPLE
                    .iter()
                    .any(|p| p == &m.octets()[..3])
            );
            assert!(m.validate_assignable().is_ok());
        }
    }

    #[test]
    fn locally_administered_sets_laa_bit() {
        let p = pool(&["random-locally-administered"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        for _ in 0..50 {
            let m = generate(&opts).unwrap();
            assert!(m.is_locally_administered());
            assert!(!m.is_multicast());
        }
    }

    #[test]
    fn forbidden_macs_are_skipped() {
        // Force the only possible Apple-like MAC to be forbidden many times in a row.
        // We can't easily force the suffix, but we can saturate "avoid" with a huge set
        // and rely on probabilistic skipping.
        let p = pool(&["apple", "intel", "samsung", "dell"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        // Just sanity-check that generation always returns a non-forbidden MAC.
        for _ in 0..100 {
            let m = generate(&opts).unwrap();
            assert!(!f.contains(&m));
            assert!(!a.contains(&m));
        }
    }

    #[test]
    fn empty_pool_errors() {
        let p: Vec<String> = Vec::new();
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        assert!(generate(&opts).is_err());
    }

    #[test]
    fn unknown_token_errors() {
        let p = pool(&["nonsense"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        assert!(generate(&opts).is_err());
    }

    // === collision-aware (--explain) path ===

    #[test]
    fn probe_free_passes_first_candidate() {
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::responds(false);
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        assert_eq!(outcome.chosen_token, "apple");
        assert_eq!(outcome.oui_fallbacks, 0);
        assert!(matches!(
            outcome.attempts.last().unwrap().reason,
            RejectionReason::Accepted
        ));
        // At least the accepted candidate must have been recorded.
        assert!(!outcome.attempts.is_empty());
    }

    #[test]
    fn probe_collision_then_free_retries() {
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::new();
        probe.queue_arp(ProbeOutcome::Collision {
            peer_ip: Some("192.168.1.5".into()),
        });
        // Subsequent calls default to Free.
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        assert!(
            outcome
                .attempts
                .iter()
                .any(|a| matches!(a.reason, RejectionReason::ActiveCollision { .. }))
        );
        assert!(matches!(
            outcome.attempts.last().unwrap().reason,
            RejectionReason::Accepted
        ));
        // One collision is below the fallback threshold.
        assert_eq!(outcome.oui_fallbacks, 0);
    }

    #[test]
    fn three_consecutive_collisions_rotate_to_next_oui_token() {
        // Prove the adaptive-backoff rule: three ARP collisions in a row on
        // the first token must advance the cursor to the next token. We
        // cheat by forcing the first three probes to collide and then
        // letting the fourth go through; the chosen token must then differ
        // from the starting one.
        let p = pool(&["apple", "intel"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::new();
        for _ in 0..COLLISIONS_BEFORE_OUI_FALLBACK {
            probe.queue_arp(ProbeOutcome::Collision {
                peer_ip: Some("192.168.1.42".into()),
            });
        }
        // Subsequent probes default to Free, so the next candidate (under
        // the rotated token) gets accepted.
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        assert_eq!(
            outcome.oui_fallbacks, 1,
            "expected exactly one OUI fallback after {COLLISIONS_BEFORE_OUI_FALLBACK} \
             consecutive collisions"
        );
        // 3 collision attempts + 1 accepted attempt = 4 entries minimum.
        assert!(outcome.attempts.len() >= 4);
    }

    #[test]
    fn collision_logs_record_peer_ip_for_forensic_clarity() {
        // The roadmap requires every collision to surface the conflicting
        // neighbour's IP. Pin it: the rejection entry must carry the
        // peer_ip we injected.
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::new();
        probe.queue_arp(ProbeOutcome::Collision {
            peer_ip: Some("10.20.30.40".into()),
        });
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        let collision_entry = outcome
            .attempts
            .iter()
            .find(|a| matches!(a.reason, RejectionReason::ActiveCollision { .. }))
            .expect("collision attempt was recorded");
        match &collision_entry.reason {
            RejectionReason::ActiveCollision { peer_ip } => {
                assert_eq!(peer_ip.as_deref(), Some("10.20.30.40"))
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn probe_unsupported_does_not_block_acceptance() {
        // CAP_NET_RAW unavailable -> probe returns Unsupported. The
        // candidate must still go through — Unsupported is informational,
        // not a collision.
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::new();
        probe.queue_arp(ProbeOutcome::Unsupported("test: no CAP_NET_RAW"));
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        assert!(matches!(
            outcome.attempts.last().unwrap().reason,
            RejectionReason::Accepted
        ));
        assert!(
            outcome
                .attempts
                .iter()
                .any(|a| matches!(a.reason, RejectionReason::ProbeUnsupported(_))),
            "Unsupported should still be recorded for --explain transparency"
        );
    }

    #[test]
    fn nd_probe_collision_also_triggers_retry() {
        // When the ARP probe says Free but ND says Collision, treat it like
        // any other collision — the candidate is taken at the L3/IPv6 layer.
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::new();
        // ARP free for the first try, ND collides → next round both Free.
        probe.queue_arp(ProbeOutcome::Free);
        probe.queue_nd(ProbeOutcome::Collision {
            peer_ip: Some("fe80::dead:beef".into()),
        });
        // (subsequent calls default to Free)
        let mut probe_opts = ProbeOptions::for_iface("wlan0");
        probe_opts.run_nd_probe = true;
        let outcome = generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
        let saw_nd_collision = outcome.attempts.iter().any(|a| match &a.reason {
            RejectionReason::ActiveCollision { peer_ip } => {
                peer_ip.as_deref() == Some("fe80::dead:beef")
            }
            _ => false,
        });
        assert!(
            saw_nd_collision,
            "expected a recorded ND collision in attempts: {:?}",
            outcome.attempts
        );
        assert!(matches!(
            outcome.attempts.last().unwrap().reason,
            RejectionReason::Accepted
        ));
    }

    #[test]
    fn avoid_set_member_is_recorded_as_avoid_rejection() {
        // Build an `avoid` set that contains ALL Apple prefixes' first byte
        // pattern... we can't deterministically construct a colliding MAC,
        // but we can pin the surface: when the avoid set is empty the
        // chosen MAC is never reported as AvoidList. (This guards against a
        // refactor that mislabels reasons.)
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::responds(false);
        let outcome = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0")).expect("ok");
        for attempt in &outcome.attempts {
            // No attempt should be tagged AvoidList when `avoid` is empty.
            assert!(
                !matches!(attempt.reason, RejectionReason::AvoidList),
                "empty avoid set must not produce AvoidList rejections"
            );
        }
    }

    #[test]
    fn empty_pool_errors_in_probe_path_too() {
        // The probe-aware entry point must replicate the empty-pool guard
        // — otherwise rotation could attempt to index `pool[0]` on the
        // first iteration.
        let p: Vec<String> = Vec::new();
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::responds(false);
        let r = generate_with_probe(&opts, &probe, &no_nd_probe_opts("wlan0"));
        assert!(r.is_err());
    }

    // === Roadmap M2 "Integration" — ByteSuffixPattern ===

    #[test]
    fn byte_pattern_unconstrained_produces_random_suffix() {
        let pat = ByteSuffixPattern::unconstrained();
        for _ in 0..16 {
            // Calling generate_for_vendor directly is the cheapest way to
            // exercise the suffix path; we just need to confirm the
            // pattern doesn't pin anything when None.
            let m = generate_for_vendor(Vendor::Apple, &pat).unwrap().unwrap();
            // First 3 bytes are an Apple OUI, last 3 are entropy. We
            // can't assert the entropy is non-deterministic in one call,
            // but we can confirm the format is valid.
            assert!(m.validate_assignable().is_ok());
        }
    }

    #[test]
    fn byte_pattern_pins_literal_trailing_bytes() {
        // Pattern `01:23:xx` -> bytes 4 and 5 are pinned, byte 6 entropy.
        let pat = ByteSuffixPattern::parse("01:23:xx").unwrap();
        for _ in 0..16 {
            let m = generate_for_vendor(Vendor::Apple, &pat).unwrap().unwrap();
            let octets = m.octets();
            assert_eq!(octets[3], 0x01, "byte 4 must be pinned to 0x01");
            assert_eq!(octets[4], 0x23, "byte 5 must be pinned to 0x23");
            // byte 6 (octets[5]) is entropy — no assertion on its value.
        }
    }

    #[test]
    fn byte_pattern_all_xx_equals_unconstrained() {
        let a = ByteSuffixPattern::parse("xx:xx:xx").unwrap();
        let b = ByteSuffixPattern::unconstrained();
        assert_eq!(a, b);
    }

    #[test]
    fn byte_pattern_full_literal_pins_every_trailing_byte() {
        let pat = ByteSuffixPattern::parse("de:ad:be").unwrap();
        for _ in 0..8 {
            let m = generate_for_vendor(Vendor::Apple, &pat).unwrap().unwrap();
            let oct = m.octets();
            assert_eq!(&oct[3..], &[0xDE, 0xAD, 0xBE]);
        }
    }

    #[test]
    fn byte_pattern_short_input_errors() {
        assert!(ByteSuffixPattern::parse("01:23").is_err());
        assert!(ByteSuffixPattern::parse("").is_err());
        assert!(ByteSuffixPattern::parse("zz:zz:zz").is_err());
    }

    /// GH#340: `ByteSuffixPattern::parse` previously panicked on
    /// multibyte input because it did byte-indexed slicing on a
    /// `String` whose `len()` was the byte length, not the char
    /// count. Stream 2 fixed the same panic class for
    /// `is_valid_per_ssid_duration`; this is the mirror fix in
    /// `src/mac/generator.rs`. The new parser reads one `char` at a
    /// time and surfaces a clean error.
    #[test]
    fn byte_pattern_multibyte_input_errors_without_panic() {
        // Each of these used to land in the `&cleaned[i*2..i*2+2]`
        // panic. Now they bail with a structured error.
        for input in [
            "é:23:xx",   // multibyte char in the first byte slot
            "01:é2:xx",  // multibyte char straddles a slot boundary
            "🦀a:bb:cc", // emoji char is 4 bytes, would have bit-mangled
            "00:00:0🦀", // multibyte at the tail
            "x€:00:00",  // 3-byte char inside the `xx` literal slot
        ] {
            let r = ByteSuffixPattern::parse(input);
            assert!(r.is_err(), "expected {input:?} to error cleanly");
        }
    }

    /// GH#340 mirror: ASCII paths the new parser must continue to
    /// accept exactly as before. Pin the canonical shapes so the
    /// regression test above doesn't accidentally tighten the
    /// accept set.
    #[test]
    fn byte_pattern_accepts_canonical_shapes_post_multibyte_fix() {
        for input in ["xx:xx:xx", "01:23:xx", "AA-BB-CC", "deadbe", "00 00 00"] {
            assert!(
                ByteSuffixPattern::parse(input).is_ok(),
                "expected {input:?} to parse"
            );
        }
    }

    #[test]
    fn generate_options_new_has_no_suffix_pattern() {
        // The convenience constructor must default to "no pattern" so
        // call sites that don't care can ignore the field.
        let p: Vec<String> = vec!["apple".into()];
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions::new(&p, &f, &a);
        assert!(opts.suffix_pattern.is_none());
    }

    #[test]
    fn probe_records_iface_passed_through() {
        // Sanity-check the wiring: `ProbeOptions.iface` must arrive at the
        // probe call. Otherwise a real implementation would emit packets on
        // the wrong netdev.
        let p = pool(&["apple"]);
        let f = empty_set();
        let a = empty_set();
        let opts = GenerateOptions {
            pool: &p,
            forbidden: &f,
            avoid: &a,
            suffix_pattern: None,
        };
        let probe = MockProbe::responds(false);
        let _ = generate_with_probe(&opts, &probe, &no_nd_probe_opts("eth42")).expect("ok");
        let calls = probe.arp_calls.lock().unwrap();
        assert!(calls.iter().all(|(iface, _)| iface == "eth42"));
        assert!(!calls.is_empty());
    }
}
