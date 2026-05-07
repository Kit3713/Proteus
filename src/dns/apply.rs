// SPDX-License-Identifier: GPL-3.0-or-later

//! Drop-in writer for the `EDNSClientSubnet=no` knob.
//!
//! All of the policy lives in `super`; this module only knows how to
//! render, write, and remove the single drop-in file. Pure functions
//! where possible so unit tests can verify formatting without touching
//! the real `/etc`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{PROTEUS_DROPIN_NAME, Paths};
use crate::version;

/// Body of the drop-in **without** the sha256 marker. Kept separate so the
/// hash can be computed over this exact text and stamped on top.
pub fn render_body() -> String {
    "[Resolve]\nEDNSClientSubnet=no\n".to_string()
}

/// Full drop-in contents including the management headers. The sha256
/// marker covers the body returned by `render_body` so the SHA stays
/// stable regardless of which proteus version stamped the header.
pub fn render_dropin() -> String {
    let body = render_body();
    let sha = sha256_hex(body.as_bytes());
    format!(
        "# managed by proteus v{version}\n# do not edit; manage via /etc/proteus/config.toml or `proteus dns apply`\n# sha256:{sha}\n{body}",
        version = version::VERSION,
    )
}

/// Returns the path Proteus's drop-in lives at given a `Paths` layout.
pub fn dropin_path(paths: &Paths) -> PathBuf {
    paths.resolved_dropin_dir().join(PROTEUS_DROPIN_NAME)
}

/// Write the drop-in atomically. Creates the parent directory if it does
/// not already exist (the typical Fedora install ships with the dir
/// missing until something writes to it).
pub fn write_dropin(paths: &Paths) -> Result<PathBuf> {
    let dir = paths.resolved_dropin_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating drop-in dir {}", dir.display()))?;
    let path = dropin_path(paths);
    let body = render_dropin();
    write_atomic(&path, body.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Remove the drop-in if present. Idempotent.
pub fn remove_dropin(paths: &Paths) -> Result<bool> {
    let path = dropin_path(paths);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow::Error::new(e).context(format!("removing {}", path.display()))),
    }
}

/// Returns true if Proteus's drop-in exists right now.
pub fn dropin_present(paths: &Paths) -> bool {
    dropin_path(paths).is_file()
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("conf.proteus-tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

// ------- SHA-256, hand-rolled to keep the dep budget at zero -------
//
// The drop-in carries `# sha256:<hex>` so phase G can detect manual edits.
// We need the digest for our own writer and for `proteus diff` later.
// 64 lines of well-tested algorithm beats pulling in `sha2` and its half-
// dozen transitive crates. Implementation follows FIPS 180-4 §6.2 verbatim.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let off = i * 4;
            *word =
                u32::from_be_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let mj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(mj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    let mut s = String::with_capacity(64);
    for b in &digest {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector_empty() {
        // FIPS 180-4 example: SHA-256 of "" is e3b0c44...
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_known_vector_abc() {
        // FIPS 180-4 example: SHA-256("abc") = ba7816bf...
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn render_body_is_minimal_and_stable() {
        let body = render_body();
        assert!(body.contains("[Resolve]"));
        assert!(body.contains("EDNSClientSubnet=no"));
        // We never want this drop-in to ship anything but the one knob.
        assert!(!body.contains("DNSOverTLS"));
        assert!(!body.contains("DNSSEC"));
        assert!(!body.contains("DNS="));
    }

    #[test]
    fn render_dropin_includes_sha_of_body() {
        let body = render_body();
        let expected = sha256_hex(body.as_bytes());
        let full = render_dropin();
        assert!(full.contains(&format!("sha256:{expected}")));
        assert!(full.contains("[Resolve]"));
        assert!(full.contains("EDNSClientSubnet=no"));
    }

    #[test]
    fn dropin_path_uses_proteus_filename() {
        let paths = Paths::default();
        let p = dropin_path(&paths);
        assert!(p.ends_with("10-proteus-no-ecs.conf"));
    }
}
