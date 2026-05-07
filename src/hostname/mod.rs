// SPDX-License-Identifier: GPL-3.0-or-later

//! Hostname management.
//!
//! The wordlist is embedded at compile-time so there is no on-disk file to
//! tamper with. Selection uses `getrandom` for entropy in production and a
//! deterministic seed for tests. All produced names are validated against
//! RFC 1123 before they ever reach the DBus layer.

pub mod apply;
pub mod dbus;

use anyhow::{Result, anyhow};

/// Embedded curated wordlist. ~500 router-flavored entries, RFC 1123-valid.
const WORDLIST_RAW: &str = include_str!("../../data/hostname-wordlist.txt");

/// Default fallback used by `mode = "generic"` when no `pinned_value` is set.
pub const GENERIC_DEFAULT: &str = "fedora";

/// RFC 1123: total length 1..=253; per-label length 1..=63.
pub const MAX_HOSTNAME_LEN: usize = 253;
pub const MAX_LABEL_LEN: usize = 63;

/// Mode selected by `[hostname] mode` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Wordlist,
    Generic,
    Pinned,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "wordlist" => Ok(Mode::Wordlist),
            "generic" => Ok(Mode::Generic),
            "pinned" => Ok(Mode::Pinned),
            other => Err(anyhow!(
                "unknown hostname mode '{other}'; expected 'wordlist', 'generic', or 'pinned'"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Wordlist => "wordlist",
            Mode::Generic => "generic",
            Mode::Pinned => "pinned",
        }
    }
}

/// Parse the embedded wordlist into a vector of references. Lines are trimmed,
/// blanks and `#`-comments are skipped. Each entry is checked against
/// `validate_hostname`; an invalid entry aborts loading rather than silently
/// shipping a bad name.
pub fn wordlist() -> Result<Vec<&'static str>> {
    let mut out = Vec::with_capacity(560);
    for (lineno, raw) in WORDLIST_RAW.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        validate_hostname(line)
            .map_err(|e| anyhow!("hostname-wordlist.txt line {}: {}", lineno + 1, e))?;
        out.push(line);
    }
    if out.is_empty() {
        return Err(anyhow!("hostname wordlist is empty"));
    }
    Ok(out)
}

/// RFC 1123-style hostname validator. Stricter than RFC 952 in that we permit
/// purely-numeric labels (modern hosts allow this), but we still reject
/// underscores, leading/trailing hyphens, length overflow, and non-ASCII.
pub fn validate_hostname(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("hostname is empty"));
    }
    if name.len() > MAX_HOSTNAME_LEN {
        return Err(anyhow!(
            "hostname '{}' is {} bytes (max {})",
            name,
            name.len(),
            MAX_HOSTNAME_LEN
        ));
    }
    for label in name.split('.') {
        validate_label(label, name)?;
    }
    Ok(())
}

fn validate_label(label: &str, full: &str) -> Result<()> {
    if label.is_empty() {
        return Err(anyhow!("hostname '{full}' has an empty label"));
    }
    if label.len() > MAX_LABEL_LEN {
        return Err(anyhow!(
            "hostname '{full}' label '{label}' is {} bytes (max {})",
            label.len(),
            MAX_LABEL_LEN
        ));
    }
    let bytes = label.as_bytes();
    if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
        return Err(anyhow!(
            "hostname '{full}' label '{label}' has a leading or trailing hyphen"
        ));
    }
    for &b in bytes {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        if !ok {
            return Err(anyhow!(
                "hostname '{full}' label '{label}' contains '{}' (only [a-z0-9-] allowed)",
                b as char
            ));
        }
    }
    Ok(())
}

/// Trait used by `pick_from_wordlist` so tests can drive selection
/// deterministically with a seeded picker.
pub trait IndexPicker {
    fn pick(&mut self, len: usize) -> Result<usize>;
}

/// Cryptographic-strength picker backed by `getrandom`.
pub struct RandomPicker;

impl IndexPicker for RandomPicker {
    fn pick(&mut self, len: usize) -> Result<usize> {
        if len == 0 {
            return Err(anyhow!("cannot pick from empty pool"));
        }
        let mut buf = [0u8; 8];
        getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
        let n = u64::from_le_bytes(buf) as usize;
        Ok(n % len)
    }
}

/// Deterministic xorshift64 picker, used for tests and any future
/// reproducible-rotation features. NOT for production use.
pub struct SeededPicker(u64);

impl SeededPicker {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero state — xorshift would stall on it.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
}

impl IndexPicker for SeededPicker {
    fn pick(&mut self, len: usize) -> Result<usize> {
        if len == 0 {
            return Err(anyhow!("cannot pick from empty pool"));
        }
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        Ok((x as usize) % len)
    }
}

/// Pick a hostname from the embedded wordlist using the supplied picker.
pub fn pick_from_wordlist<P: IndexPicker>(picker: &mut P) -> Result<String> {
    let pool = wordlist()?;
    let idx = picker.pick(pool.len())?;
    Ok(pool[idx].to_string())
}

/// Build a side-effect-free preview of `proteus hostname rotate` for
/// `proteus dry-run`. We seed the wordlist picker so the preview reads
/// concretely with a stable name; the real rotation uses `RandomPicker`.
pub fn plan_rotate(config: &crate::config::Config) -> crate::dry_run::Plan {
    use crate::dry_run::{Plan, PlanStep, StepKind};
    let mut plan = Plan::new("hostname");
    if !config.hostname.enabled {
        plan.note("hostname: disabled in config (hostname.enabled = false)");
        return plan;
    }
    let mode = config.hostname.mode.as_str();
    let example = resolve_with(&config.hostname, &mut SeededPicker::new(0xCAFE_F00D))
        .unwrap_or_else(|_| "linksys".to_string());
    plan.push(PlanStep {
        kind: StepKind::HostnameSet,
        message: format!("would set hostname (mode={mode}, e.g. '{example}')"),
        detail: Some("writes kernel/pretty/transient via systemd hostname1 DBus interface".into()),
    });
    plan.push(PlanStep {
        kind: StepKind::DbusCall,
        message: "would call org.freedesktop.hostname1.SetStaticHostname / SetPrettyHostname"
            .into(),
        detail: None,
    });
    plan.push(PlanStep {
        kind: StepKind::StateUpdate,
        message: "would update state.json: originals.hostname (first apply only)".into(),
        detail: None,
    });
    plan
}

/// Resolve a hostname using the supplied picker. Shared by `resolve_hostname`
/// (production: `RandomPicker`) and `plan_rotate` (preview: `SeededPicker`).
fn resolve_with<P: IndexPicker>(
    cfg: &crate::config::HostnameConfig,
    picker: &mut P,
) -> Result<String> {
    let mode = Mode::parse(&cfg.mode)?;
    let name = match mode {
        Mode::Wordlist => pick_from_wordlist(picker)?,
        Mode::Generic => cfg
            .pinned_value
            .clone()
            .unwrap_or_else(|| GENERIC_DEFAULT.to_string()),
        Mode::Pinned => cfg.pinned_value.clone().ok_or_else(|| {
            anyhow!("hostname mode = 'pinned' but [hostname] pinned_value is unset")
        })?,
    };
    validate_hostname(&name)?;
    Ok(name)
}

/// Resolve the hostname Proteus should apply for a given config.
///
/// `wordlist` -> a fresh random pick from the embedded list.
/// `generic`  -> the user's `pinned_value` if set, else `GENERIC_DEFAULT`.
/// `pinned`   -> the user's `pinned_value` (required).
pub fn resolve_hostname(cfg: &crate::config::HostnameConfig) -> Result<String> {
    resolve_with(cfg, &mut RandomPicker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_parses_and_has_expected_size() {
        let entries = wordlist().expect("wordlist must parse");
        assert!(
            entries.len() >= 500,
            "wordlist should have ~500 entries, found {}",
            entries.len()
        );
        // Spot-check a couple of canonical router names exist.
        assert!(entries.contains(&"linksys"));
        assert!(entries.contains(&"tplink") || entries.contains(&"fedora"));
    }

    #[test]
    fn rfc_1123_validator_accepts_valid_names() {
        for name in &[
            "fedora",
            "linksys-7a3f",
            "a",
            "a1",
            "router.local",
            "x".repeat(63).as_str(),
        ] {
            assert!(validate_hostname(name).is_ok(), "should accept '{name}'");
        }
    }

    #[test]
    fn rfc_1123_validator_rejects_invalid_names() {
        for name in &[
            "",            // empty
            "MyLaptop",    // uppercase
            "my_laptop",   // underscore
            "-leading",    // leading hyphen
            "trailing-",   // trailing hyphen
            "double..dot", // empty label
            "host name",   // space
            "café",        // non-ASCII
        ] {
            assert!(validate_hostname(name).is_err(), "should reject '{name}'");
        }
        // Label too long (64 chars) — fail
        let too_long_label = "x".repeat(64);
        assert!(validate_hostname(&too_long_label).is_err());
    }

    #[test]
    fn seeded_picker_is_deterministic() {
        let mut p1 = SeededPicker::new(42);
        let mut p2 = SeededPicker::new(42);
        for _ in 0..32 {
            assert_eq!(p1.pick(534).unwrap(), p2.pick(534).unwrap());
        }
    }

    #[test]
    fn seeded_pick_from_wordlist_is_stable() {
        let mut p = SeededPicker::new(7);
        let a = pick_from_wordlist(&mut p).unwrap();
        let mut p = SeededPicker::new(7);
        let b = pick_from_wordlist(&mut p).unwrap();
        assert_eq!(a, b);
        // And it must be a real entry, not garbage.
        let entries = wordlist().unwrap();
        assert!(entries.contains(&a.as_str()));
    }

    #[test]
    fn mode_parse_round_trips() {
        for m in [Mode::Wordlist, Mode::Generic, Mode::Pinned] {
            assert_eq!(Mode::parse(m.as_str()).unwrap(), m);
        }
        assert!(Mode::parse("nonsense").is_err());
    }

    #[test]
    fn resolve_dispatches_per_mode() {
        let mut cfg = crate::config::HostnameConfig {
            enabled: true,
            mode: "generic".into(),
            pinned_value: None,
            rotate_with_mac: false,
        };
        assert_eq!(resolve_hostname(&cfg).unwrap(), GENERIC_DEFAULT);

        cfg.mode = "pinned".into();
        cfg.pinned_value = Some("trustedlaptop".into());
        assert_eq!(resolve_hostname(&cfg).unwrap(), "trustedlaptop");

        cfg.mode = "pinned".into();
        cfg.pinned_value = None;
        assert!(resolve_hostname(&cfg).is_err());

        cfg.mode = "wordlist".into();
        cfg.pinned_value = None;
        let pick = resolve_hostname(&cfg).unwrap();
        assert!(wordlist().unwrap().contains(&pick.as_str()));

        // Invalid pinned value bubbles up via the validator.
        cfg.mode = "pinned".into();
        cfg.pinned_value = Some("Bad_Name".into());
        assert!(resolve_hostname(&cfg).is_err());
    }
}
