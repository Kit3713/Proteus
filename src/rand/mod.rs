// SPDX-License-Identifier: GPL-3.0-or-later

//! Bias-free random index pickers shared across MAC / hostname / persona /
//! Bluetooth / scan-style selection sites.
//!
//! The naive pattern `rand_byte() % len` introduces a small but non-zero
//! distributional bias whenever `len` doesn't divide the source range —
//! `256 % 19 = 9`, so the trailing 9 of the 256 byte values are unevenly
//! split across the 19 indices, leaving 9 of the indices ~5% over-
//! represented. For a privacy tool whose entire job is producing
//! unguessable identifiers, even a 5% skew is wrong: it lets a passive
//! observer cluster fingerprints by which OUI / hostname / alias the
//! generator landed on more often than chance.
//!
//! Both helpers below use textbook rejection sampling: pull a value, and
//! retry if it falls inside the trailing partial block. Worst case is
//! roughly two extra rolls — negligible compared to the surrounding
//! syscalls (`getrandom`).
//!
//! - [`unbiased_index`] — byte-stream picker for pools where `len <= 256`
//!   (MAC OUI tokens, OUI prefixes, owner-name pool, generic alias pool,
//!   most pickers in the codebase).
//! - [`unbiased_index_u64`] — 8-byte stream picker for pools where
//!   `len > 256` (the embedded hostname wordlist is 534 entries; persona
//!   pools after filtering can grow past 256 if more personas are
//!   imported).
//!
//! Both helpers take a `next` closure rather than calling `getrandom`
//! directly so tests can drive selection deterministically (see the
//! distribution-uniformity tests below — they walk a uniform stream and
//! prove every reachable index gets exactly the same hit count).

use anyhow::{Result, anyhow};

/// Rejection-sampled `[0, len)` from a stream of bytes. Avoids the modulo
/// bias of `byte % len` when `256 % len != 0` — for the 19-entry pool the
/// naive `% 19` skews 4 of the 19 indices ~5% high (issues #143/#152/#154
/// and #226). Worst case for any `len <= 256` is two extra random byte
/// reads, so the cost is negligible.
///
/// Returns `Err` for `len == 0` (cannot pick from an empty pool) and for
/// `len > 256` (the byte-stream picker can't represent that many distinct
/// values; callers with larger pools must use [`unbiased_index_u64`]).
pub fn unbiased_index<F: FnMut() -> Result<u8>>(len: usize, mut next: F) -> Result<usize> {
    if len == 0 {
        return Err(anyhow!("cannot pick from empty pool"));
    }
    if len > 256 {
        // The byte-stream picker only covers up to 256 distinct values; this
        // never happens in practice for the byte-based call sites (their
        // pools are fixed and small) but guard explicitly so a future caller
        // can't silently fall back to biased modulo.
        return Err(anyhow!(
            "unbiased_index supports len <= 256 (got {len}); use unbiased_index_u64 for larger pools"
        ));
    }
    let span = 256 - (256 % len);
    loop {
        let byte = next()? as usize;
        if byte < span {
            return Ok(byte % len);
        }
    }
}

/// Rejection-sampled `[0, len)` from a stream of `u64` values. Used where
/// the pool exceeds 256 entries (the embedded hostname wordlist is 534;
/// persona import could grow other pools past the byte-stream limit too).
///
/// Same rejection-sampling shape as [`unbiased_index`], scaled to a
/// `u64`-wide source range. `len` must be `>= 1` and `<= 2^63` (a u64
/// can index that comfortably; in practice all our pools are <10k).
pub fn unbiased_index_u64<F: FnMut() -> Result<u64>>(len: usize, mut next: F) -> Result<usize> {
    if len == 0 {
        return Err(anyhow!("cannot pick from empty pool"));
    }
    let len_u64 = len as u64;
    // span = floor(2^64 / len) * len. Computed as (u64::MAX - (u64::MAX %
    // len_u64) - (len_u64 - 1)) so the subtraction stays in u64 without
    // overflow. Equivalent to `((u64::MAX / len_u64) * len_u64)`.
    let span = u64::MAX - (u64::MAX % len_u64);
    loop {
        let v = next()?;
        // `v < span` keeps the uniform region; the discarded tail
        // `[span, u64::MAX]` is the partial block that would otherwise
        // make `v % len` skew.
        if v < span {
            return Ok((v % len_u64) as usize);
        }
    }
}

/// Convenience: pull one byte from `getrandom` for use with [`unbiased_index`].
/// Production callers that don't need a custom source can pass this in.
pub fn getrandom_byte() -> Result<u8> {
    let mut buf = [0u8; 1];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(buf[0])
}

/// Convenience: pull eight bytes from `getrandom` and decode as a `u64`
/// for use with [`unbiased_index_u64`]. Little-endian to match the
/// historical convention in the rest of the codebase.
pub fn getrandom_u64() -> Result<u64> {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbiased_index_rejects_empty_pool() {
        assert!(unbiased_index(0, || Ok(0)).is_err());
    }

    #[test]
    fn unbiased_index_rejects_len_above_256() {
        // The byte-stream picker can't represent more than 256 indices;
        // make sure callers with larger pools get a clear error rather
        // than silent bias.
        assert!(unbiased_index(257, || Ok(0)).is_err());
    }

    #[test]
    fn unbiased_index_rejects_high_bytes_for_non_divisor_lens() {
        // 256 % 19 = 9, so the biased "high" range is bytes 247..=255 (the
        // last 9 of the 256 codepoints). Drive that range explicitly and
        // confirm we re-roll instead of producing biased output.
        let mut bytes: Vec<u8> = (247u8..=255).collect();
        bytes.push(7); // first non-rejected byte
        bytes.reverse();
        let idx = unbiased_index(19, || {
            bytes.pop().ok_or_else(|| anyhow!("ran out of bytes"))
        })
        .unwrap();
        assert_eq!(idx, 7);
    }

    #[test]
    fn unbiased_index_distribution_is_uniform_in_practice() {
        // Cycle through every byte 0..=255 once (a uniform stream) and
        // confirm each index in [0, 19) gets the same number of hits. With
        // pure `% 19` we would see 4 indices over-represented by ~5%.
        let len = 19;
        let span = 256 - (256 % len);
        let mut counts = vec![0usize; len];
        let mut feed: Vec<u8> = (0u8..=255).collect();
        feed.reverse();
        while !feed.is_empty() {
            let res = unbiased_index(len, || feed.pop().ok_or_else(|| anyhow!("end of stream")));
            match res {
                Ok(i) => counts[i] += 1,
                Err(_) => break, // ran out of bytes mid-rejection
            }
        }
        let expected_each = span / len; // 247 / 19 = 13
        for c in &counts {
            assert_eq!(
                *c, expected_each,
                "uniform stream should give uniform indices, got {counts:?}"
            );
        }
    }

    #[test]
    fn unbiased_index_u64_rejects_empty_pool() {
        assert!(unbiased_index_u64(0, || Ok(0)).is_err());
    }

    #[test]
    fn unbiased_index_u64_handles_large_pools() {
        // 534-entry hostname wordlist — exceeds the byte-stream limit but
        // sits comfortably inside the u64 picker. Drive a deterministic
        // counter source and confirm we get a valid index.
        let mut counter: u64 = 0;
        let len = 534;
        for _ in 0..50 {
            let idx = unbiased_index_u64(len, || {
                counter = counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
                Ok(counter)
            })
            .unwrap();
            assert!(idx < len, "index {idx} out of range for len {len}");
        }
    }

    #[test]
    fn unbiased_index_u64_rejects_high_values_for_non_divisor_lens() {
        // For len = 7, span = floor(2^64 / 7) * 7 = u64::MAX - (u64::MAX %
        // 7). The trailing partial block is [span, u64::MAX]. Drive a
        // value in that range and prove we re-roll.
        let len = 7u64;
        let span = u64::MAX - (u64::MAX % len);
        // First call returns a value that must be rejected; second call
        // returns an in-range value.
        let mut feed: Vec<u64> = vec![42, span + 1];
        let idx = unbiased_index_u64(len as usize, || {
            feed.pop().ok_or_else(|| anyhow!("ran out of values"))
        })
        .unwrap();
        // 42 % 7 = 0
        assert_eq!(idx, 0);
    }

    #[test]
    fn unbiased_index_u64_distribution_is_uniform_for_small_len() {
        // We can't enumerate all 2^64 values, but for a small `len` we can
        // walk a deterministic linear stream over a much larger range than
        // `len` and confirm every index gets the same count modulo
        // boundary effects. Use len=7 and step through 0..=4900 (a
        // multiple of 700, span large enough that each index lands ~700
        // times).
        let len = 7;
        let mut counts = vec![0usize; len];
        let span = u64::MAX - (u64::MAX % len as u64);
        for v in 0u64..4900 {
            // All values < span, so no rejection on this stream.
            assert!(v < span);
            let mut consumed = false;
            let idx = unbiased_index_u64(len, || {
                if consumed {
                    Err(anyhow!("end"))
                } else {
                    consumed = true;
                    Ok(v)
                }
            })
            .unwrap();
            counts[idx] += 1;
        }
        // 4900 / 7 = 700 each, exactly.
        for c in &counts {
            assert_eq!(*c, 700, "uniform stream over multiple of len: {counts:?}");
        }
    }

    #[test]
    fn unbiased_index_u64_works_for_len_one() {
        // Edge case: len=1 means every value maps to index 0, no rejection
        // ever needed (span == u64::MAX is unreachable but the math holds).
        for _ in 0..16 {
            let idx = unbiased_index_u64(1, || Ok(123_456_789)).unwrap();
            assert_eq!(idx, 0);
        }
    }

    #[test]
    fn convenience_byte_helper_is_in_range() {
        // Smoke: the helper must return a u8 (0..=255). No bias claim on
        // one call — the distribution is the kernel's job.
        for _ in 0..16 {
            let _ = getrandom_byte().unwrap();
        }
    }

    #[test]
    fn convenience_u64_helper_works() {
        // Smoke: the helper must return *something* (we can't prove
        // uniformity in finite calls). Just exercise the wire-up.
        for _ in 0..4 {
            let _ = getrandom_u64().unwrap();
        }
    }
}
