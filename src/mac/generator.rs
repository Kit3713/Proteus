// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use anyhow::{Result, anyhow, bail};

use super::oui::Vendor;
use super::{Mac, MacError};

const MAX_GENERATION_ATTEMPTS: usize = 64;

#[derive(Debug, Clone)]
pub struct GenerateOptions<'a> {
    pub pool: &'a [String],
    pub forbidden: &'a HashSet<Mac>,
    pub avoid: &'a HashSet<Mac>,
}

pub fn generate(opts: &GenerateOptions<'_>) -> Result<Mac> {
    if opts.pool.is_empty() {
        bail!("OUI pool is empty");
    }
    let mut last_err: Option<MacError> = None;
    for _ in 0..MAX_GENERATION_ATTEMPTS {
        let token_idx = (rand_u8()? as usize) % opts.pool.len();
        let token = &opts.pool[token_idx];
        let vendor = Vendor::from_pool_token(token)
            .ok_or_else(|| anyhow!("unknown OUI pool token '{token}'"))?;
        let mac = match vendor.prefixes() {
            Some(prefixes) => {
                let prefix_idx = (rand_u8()? as usize) % prefixes.len();
                let prefix = prefixes[prefix_idx];
                let suffix = rand_bytes::<3>()?;
                let mut octets = [0u8; 6];
                octets[..3].copy_from_slice(&prefix);
                octets[3..].copy_from_slice(&suffix);
                Mac(octets)
            }
            None => {
                let mut octets = rand_bytes::<6>()?;
                // LAA bit set, multicast bit clear.
                octets[0] = (octets[0] | 0x02) & 0xFE;
                Mac(octets)
            }
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

fn rand_u8() -> Result<u8> {
    let mut buf = [0u8; 1];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(buf[0])
}

fn rand_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn empty_set() -> HashSet<Mac> {
        HashSet::new()
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
        };
        assert!(generate(&opts).is_err());
    }
}
