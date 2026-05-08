// SPDX-License-Identifier: GPL-3.0-or-later

//! Persona discovery and validation.
//!
//! Two sources, both flat directories of `<id>.toml` files:
//! - **Builtin** — embedded under `data/personas/` via `include_dir!`.
//!   Shipped with the binary; never written at runtime.
//! - **User** — `/etc/proteus/personas/` (system-wide; matches the
//!   root-via-polkit model). On id collision the user file shadows the
//!   builtin so a user can fork any built-in persona without forking the
//!   binary. See roadmap Milestone 2.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use include_dir::{Dir, include_dir};

use super::{Persona, PersonaSource, PersonaSummary};

/// Embedded built-in catalogue. Each `data/personas/<id>.toml` is one
/// persona; the file stem must match the `id` field.
static BUILTIN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/data/personas");

/// Default user-persona root. Loader callers pass a `Path` so tests can
/// point at a `TempDir`; production code passes this constant.
pub const DEFAULT_USER_ROOT: &str = "/etc/proteus/personas";

/// Try to load a built-in persona by id. Returns `None` when the id is
/// unknown so callers can fall through to user personas; errors during
/// parse or schema check propagate as `Err`. Issue #232: every load path
/// now schema-checks so `persona use` cannot land a malformed persona.
pub fn load_builtin(id: &str) -> Result<Option<Persona>> {
    let path = format!("{id}.toml");
    let Some(file) = BUILTIN.get_file(&path) else {
        return Ok(None);
    };
    let raw = file
        .contents_utf8()
        .ok_or_else(|| anyhow!("builtin persona {id} is not valid UTF-8"))?;
    let p: Persona = toml::from_str(raw)
        .with_context(|| format!("parsing builtin persona '{id}' (see `proteus wiki personas`)"))?;
    if p.id != id {
        anyhow::bail!(
            "builtin persona file '{id}.toml' has mismatched id field '{}' (see wiki personas)",
            p.id
        );
    }
    schema_check(&p)?;
    Ok(Some(p))
}

/// Load a user-authored persona from `<root>/<id>.toml`. Schema-checks
/// before returning (#232).
pub fn load_user(id: &str, root: &Path) -> Result<Option<Persona>> {
    let path = root.join(format!("{id}.toml"));
    if !path.exists() {
        return Ok(None);
    }
    let p = parse_file(&path)?;
    if p.id != id {
        anyhow::bail!(
            "user persona '{}' has id field '{}' that does not match its filename (see wiki personas)",
            path.display(),
            p.id
        );
    }
    schema_check(&p)?;
    Ok(Some(p))
}

/// User shadows builtin on id collision. Callers usually want this rather
/// than the two narrower helpers.
pub fn load(id: &str, user_root: &Path) -> Result<Option<(Persona, PersonaSource)>> {
    if let Some(p) = load_user(id, user_root)? {
        return Ok(Some((p, PersonaSource::User)));
    }
    if let Some(p) = load_builtin(id)? {
        return Ok(Some((p, PersonaSource::Builtin)));
    }
    Ok(None)
}

/// Enumerate every persona Proteus knows about — both built-in and the
/// user's `/etc/proteus/personas/`. Sorted by id for determinism. Returns
/// summaries; full bodies load on demand via `load`. Issue #253: schema
/// failures are surfaced via `valid: false` + a stderr warning instead of
/// being silently dropped.
pub fn list_all(user_root: &Path) -> Vec<PersonaSummary> {
    let mut out: Vec<PersonaSummary> = Vec::new();
    let mut seen_ids: std::collections::BTreeSet<String> = Default::default();

    // User entries first so id-collision shadowing is correct: builtin
    // entries with an id we've already seen get skipped.
    if let Ok(rd) = std::fs::read_dir(user_root) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let Ok(p) = parse_file(&path) else {
                eprintln!(
                    "proteus: warning: failed to parse user persona {}",
                    path.display()
                );
                continue;
            };
            seen_ids.insert(p.id.clone());
            out.push(summary_for(&p, PersonaSource::User));
        }
    }

    for f in BUILTIN.files() {
        if f.path().extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let Some(raw) = f.contents_utf8() else {
            continue;
        };
        let Ok(p) = toml::from_str::<Persona>(raw) else {
            continue;
        };
        if seen_ids.contains(&p.id) {
            continue;
        }
        out.push(summary_for(&p, PersonaSource::Builtin));
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn summary_for(p: &Persona, source: PersonaSource) -> PersonaSummary {
    let (valid, schema_error) = match schema_check(p) {
        Ok(()) => (true, None),
        Err(e) => {
            let msg = format!("{e:#}");
            eprintln!(
                "proteus: warning: persona '{}' failed schema check: {msg}",
                p.id
            );
            (false, Some(msg))
        }
    };
    PersonaSummary {
        id: p.id.clone(),
        display_name: p.display_name.clone(),
        kind: p.kind,
        category: p.category,
        source,
        valid,
        schema_error,
    }
}

/// Parse + schema-check a single persona file. Used by `proteus persona
/// validate <path>`.
pub fn validate(path: &Path) -> Result<Persona> {
    let p = parse_file(path)?;
    schema_check(&p)?;
    Ok(p)
}

/// Parse + schema-check a persona from in-memory bytes. Used by the
/// `proteus persona import` flow so the same bytes that pass validation
/// are the bytes that get written — closes the TOCTOU window where a
/// swapped source file landed different bytes than were validated (#231).
pub fn validate_bytes(bytes: &[u8], origin: &str) -> Result<Persona> {
    let raw = std::str::from_utf8(bytes).with_context(|| format!("{origin} is not valid UTF-8"))?;
    let p: Persona = toml::from_str(raw).with_context(|| {
        format!("parsing {origin} (see `proteus wiki personas` for the schema)")
    })?;
    schema_check(&p)?;
    Ok(p)
}

fn parse_file(path: &Path) -> Result<Persona> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let p: Persona = toml::from_str(&raw).with_context(|| {
        format!(
            "parsing {} (see `proteus wiki personas` for the schema)",
            path.display()
        )
    })?;
    Ok(p)
}

/// Schema-level invariants beyond what serde enforces. Wiki-linked errors
/// so the user lands on the right page when a hand-edited file goes wrong.
/// Issue #266: validates every field, not just id/display_name/hostname/cadence.
fn schema_check(p: &Persona) -> Result<()> {
    if p.id.is_empty() || !is_kebab_case(&p.id) {
        anyhow::bail!(
            "persona id '{}' is not kebab-case (see `proteus wiki personas`)",
            p.id
        );
    }
    if p.display_name.trim().is_empty() {
        anyhow::bail!(
            "persona '{}' has empty display_name (see `proteus wiki personas`)",
            p.id
        );
    }
    if p.hostname_template.trim().is_empty() {
        anyhow::bail!(
            "persona '{}' has empty hostname_template (see `proteus wiki personas`)",
            p.id
        );
    }
    match p.kind {
        super::PersonaKind::Randomizer if p.rotate_cadence.is_none() => {
            anyhow::bail!(
                "randomizer persona '{}' must set rotate_cadence (see `proteus wiki personas`)",
                p.id
            )
        }
        _ => {}
    }
    if let Some(c) = p.rotate_cadence.as_deref() {
        validate_rotate_cadence(&p.id, c)?;
    }
    for token in &p.oui_pool {
        validate_oui_token(&p.id, token)?;
    }
    validate_dhcp_fingerprint(&p.id, &p.dhcp_fingerprint)?;
    validate_tcp_stack(&p.id, &p.tcp_stack)?;
    validate_ipv6_traits(&p.id, &p.ipv6_traits)?;
    validate_rf_traits(&p.id, &p.rf_traits)?;
    validate_mdns_traits(&p.id, &p.mdns)?;
    Ok(())
}

fn is_kebab_case(s: &str) -> bool {
    !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `rotate_cadence` accepts the literal `never` plus the duration grammar
/// the rotation timer parser already handles (`30m`, `2h`, `4h`, ...).
/// Reject anything else so a typo lands at the user, not silently in
/// `state.json`.
fn validate_rotate_cadence(id: &str, c: &str) -> Result<()> {
    let trimmed = c.trim();
    if trimmed.is_empty() {
        anyhow::bail!(
            "persona '{id}' has empty rotate_cadence; use 'never' or a duration like '30m' (see `proteus wiki personas`)"
        );
    }
    if trimmed == "never" {
        return Ok(());
    }
    // Duration: digits + unit (s/m/h/d).
    let last = trimmed.chars().last().unwrap();
    if !matches!(last, 's' | 'm' | 'h' | 'd') {
        anyhow::bail!(
            "persona '{id}' rotate_cadence '{c}' must end in s/m/h/d or be 'never' (see `proteus wiki personas`)"
        );
    }
    let digits = &trimmed[..trimmed.len() - 1];
    if digits.is_empty() || digits.parse::<u64>().is_err() {
        anyhow::bail!(
            "persona '{id}' rotate_cadence '{c}' has no numeric magnitude (see `proteus wiki personas`)"
        );
    }
    Ok(())
}

/// OUI pool entries are either vendor tokens (e.g. `apple`, `intel`,
/// `random-locally-administered`) or literal `aa:bb:cc` prefixes. Both
/// forms are kebab-or-hex-shaped — reject empties and mixed garbage.
fn validate_oui_token(id: &str, token: &str) -> Result<()> {
    if token.is_empty() {
        anyhow::bail!("persona '{id}' has empty oui_pool entry (see `proteus wiki personas`)");
    }
    let is_vendor_tag = token
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let is_literal_prefix = is_oui_literal(token);
    if !is_vendor_tag && !is_literal_prefix {
        anyhow::bail!(
            "persona '{id}' oui_pool entry '{token}' is neither a vendor tag (kebab-case) nor an OUI literal 'aa:bb:cc' (see `proteus wiki personas`)"
        );
    }
    Ok(())
}

fn is_oui_literal(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn validate_dhcp_fingerprint(id: &str, dhcp: &super::DhcpFingerprint) -> Result<()> {
    // Option 12 host_name: RFC 1123 label-shaped when set. Empty = "use
    // template-rendered hostname", which is fine.
    if !dhcp.host_name.is_empty() && !is_dhcp_string_safe(&dhcp.host_name) {
        anyhow::bail!(
            "persona '{id}' dhcp_fingerprint.host_name '{}' contains control bytes; ASCII printable only (see `proteus wiki personas`)",
            dhcp.host_name
        );
    }
    // Option 81 fqdn: same rule. Empty = "do not send".
    if !dhcp.fqdn.is_empty() && !is_dhcp_string_safe(&dhcp.fqdn) {
        anyhow::bail!(
            "persona '{id}' dhcp_fingerprint.fqdn '{}' contains control bytes (see `proteus wiki personas`)",
            dhcp.fqdn
        );
    }
    // Option 60 vendor-class-identifier: same rule.
    if !dhcp.vendor_class_identifier.is_empty()
        && !is_dhcp_string_safe(&dhcp.vendor_class_identifier)
    {
        anyhow::bail!(
            "persona '{id}' dhcp_fingerprint.vendor_class_identifier contains control bytes (see `proteus wiki personas`)"
        );
    }
    // Option 55 parameter-request-list: each entry is a u8 DHCP option
    // code; serde already enforces the type, but reject 0 (pad) which is
    // invalid as a request-list entry.
    for code in &dhcp.parameter_request_list {
        if *code == 0 {
            anyhow::bail!(
                "persona '{id}' dhcp_fingerprint.parameter_request_list contains 0 (pad), which is not a valid request code (see `proteus wiki personas`)"
            );
        }
    }
    Ok(())
}

fn is_dhcp_string_safe(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
}

fn validate_tcp_stack(id: &str, t: &super::TcpStackProfile) -> Result<()> {
    // window_scale: kernel accepts 0..=14.
    if t.window_scale > 14 {
        anyhow::bail!(
            "persona '{id}' tcp_stack.window_scale {} exceeds kernel max 14 (see `proteus wiki personas`)",
            t.window_scale
        );
    }
    // mss: 0 = "leave at default" (allowed). When set, must fit in a
    // standard Ethernet frame minus headers. 536 is the IPv4 minimum
    // path-MTU; 9000 covers jumbo frames.
    if t.mss != 0 && !(536..=9000).contains(&t.mss) {
        anyhow::bail!(
            "persona '{id}' tcp_stack.mss {} out of range 536..=9000 (see `proteus wiki personas`)",
            t.mss
        );
    }
    // default_ttl: 0 = "leave at kernel default". Anything > 0 must be a
    // realistic TTL — 64 (Linux/macOS), 128 (Windows), 255 (some embedded).
    if t.default_ttl != 0 && t.default_ttl < 32 {
        anyhow::bail!(
            "persona '{id}' tcp_stack.default_ttl {} is implausibly low (see `proteus wiki personas`)",
            t.default_ttl
        );
    }
    Ok(())
}

fn validate_ipv6_traits(id: &str, t: &super::Ipv6Traits) -> Result<()> {
    // addr_gen_mode: empty = "do not set"; otherwise one of the documented
    // modes. The kernel knob accepts these exact strings.
    if !t.addr_gen_mode.is_empty()
        && !matches!(
            t.addr_gen_mode.as_str(),
            "eui64" | "stable-privacy" | "random" | "none"
        )
    {
        anyhow::bail!(
            "persona '{id}' ipv6_traits.addr_gen_mode '{}' must be eui64/stable-privacy/random/none or empty (see `proteus wiki personas`)",
            t.addr_gen_mode
        );
    }
    Ok(())
}

/// Issue #305: mDNS records must follow DNS-SD service-type form
/// (`_<service>._<proto>` where proto is `_tcp` or `_udp`). Reject empty
/// strings and obvious garbage so a hand-edit lands at the user, not in
/// state.json. When `mdns_advertise = false` we still allow `services`
/// to be populated (the apply path will skip emitting them); that lets
/// a persona record the canonical service list for audit even when the
/// cover is "this device exists but does not Bonjour".
fn validate_mdns_traits(id: &str, t: &super::MdnsTraits) -> Result<()> {
    for svc in &t.services {
        if svc.is_empty() {
            anyhow::bail!(
                "persona '{id}' mdns.services contains an empty string (see `proteus wiki personas`)"
            );
        }
        if !is_dns_sd_service_type(svc) {
            anyhow::bail!(
                "persona '{id}' mdns.services entry '{svc}' is not a DNS-SD service type (expected '_service._tcp' or '_service._udp'; see `proteus wiki personas`)"
            );
        }
    }
    for hint in &t.txt_hints {
        if hint.is_empty() {
            anyhow::bail!(
                "persona '{id}' mdns.txt_hints contains an empty string (see `proteus wiki personas`)"
            );
        }
        if !is_dhcp_string_safe(hint) {
            anyhow::bail!(
                "persona '{id}' mdns.txt_hints entry '{hint}' contains control bytes (see `proteus wiki personas`)"
            );
        }
    }
    Ok(())
}

/// DNS-SD service types are `_<label>._tcp` or `_<label>._udp`. The label
/// itself is RFC 6335-shaped (alphanumeric + hyphen, 1..15 chars). Real
/// captures sometimes contain longer labels (`_apple-mobdev2`); we accept
/// up to 31 chars so vendor extensions pass.
fn is_dns_sd_service_type(s: &str) -> bool {
    if !s.starts_with('_') {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 2 {
        return false;
    }
    let svc = parts[0];
    let proto = parts[1];
    if proto != "_tcp" && proto != "_udp" {
        return false;
    }
    // Strip the leading underscore and check the label.
    let label = svc.strip_prefix('_').unwrap_or(svc);
    if label.is_empty() || label.len() > 31 {
        return false;
    }
    label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn validate_rf_traits(id: &str, t: &super::RfTraits) -> Result<()> {
    // tx_power_dbm: 0 = "leave at regulatory max". Realistic ceiling is
    // ~30 dBm (1 W); anything higher is operator error or confusion with
    // mW. Floor is 0.
    if t.tx_power_dbm > 30 {
        anyhow::bail!(
            "persona '{id}' rf_traits.tx_power_dbm {} exceeds 30 dBm (see `proteus wiki personas`)",
            t.tx_power_dbm
        );
    }
    if !t.scan_style.is_empty() && !matches!(t.scan_style.as_str(), "passive" | "active") {
        anyhow::bail!(
            "persona '{id}' rf_traits.scan_style '{}' must be 'passive' or 'active' (see `proteus wiki personas`)",
            t.scan_style
        );
    }
    if !t.power_save.is_empty() && !matches!(t.power_save.as_str(), "on" | "off" | "auto") {
        anyhow::bail!(
            "persona '{id}' rf_traits.power_save '{}' must be on/off/auto (see `proteus wiki personas`)",
            t.power_save
        );
    }
    Ok(())
}

/// Path inside `/etc/proteus/personas/<id>.toml` for write-side commands
/// (`new`, `edit`, `import`). Centralised so write semantics stay uniform.
pub fn user_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.toml"))
}

/// Iterator over every embedded built-in persona's TOML body. Used by the
/// `every embedded persona validates` test; not part of the public API.
#[cfg(test)]
pub(crate) fn builtin_raw_bodies() -> impl Iterator<Item = (&'static str, &'static str)> {
    BUILTIN.files().filter_map(|f| {
        if f.path().extension().and_then(|s| s.to_str()) != Some("toml") {
            return None;
        }
        let stem = f.path().file_stem().and_then(|s| s.to_str())?;
        let raw = f.contents_utf8()?;
        Some((stem, raw))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_persona_parses_and_passes_schema_check() {
        let mut count = 0usize;
        for (stem, raw) in builtin_raw_bodies() {
            let p: Persona = toml::from_str(raw)
                .unwrap_or_else(|e| panic!("builtin {stem} failed to parse: {e}"));
            assert_eq!(
                p.id, stem,
                "builtin {stem}.toml id field mismatches the file stem"
            );
            schema_check(&p).unwrap_or_else(|e| panic!("builtin {stem} failed schema check: {e}"));
            count += 1;
        }
        assert!(
            count >= 15,
            "expected at least 15 built-in personas, found {count}"
        );
    }

    /// Issue #305: every brand stealth persona must carry a non-default
    /// option-55 list. A real iPhone never sends an empty option 55 — and
    /// "Linux server defaults" (an empty list) applied to a brand persona
    /// is the cross-layer mismatch the issue is closing. Randomizers are
    /// allowed empty (their privacy goal is to disappear into noise, not
    /// to mimic any particular device).
    #[test]
    fn every_brand_stealth_persona_has_a_dhcp_option_55() {
        let mut checked = 0usize;
        for (stem, raw) in builtin_raw_bodies() {
            let p: Persona = toml::from_str(raw)
                .unwrap_or_else(|e| panic!("builtin {stem} failed to parse: {e}"));
            if p.kind != super::super::PersonaKind::Stealth {
                continue;
            }
            assert!(
                !p.dhcp_fingerprint.parameter_request_list.is_empty(),
                "stealth persona '{stem}' has empty DHCP option 55 — that's the \
                 cross-layer Linux-default mismatch issue #305 is fixing"
            );
            checked += 1;
        }
        assert!(
            checked >= 13,
            "expected at least 13 stealth personas to check option-55 against, found {checked}"
        );
    }

    /// Issue #305: brand stealth personas declare a coherent mDNS posture.
    /// Either the persona advertises (`mdns_advertise = true`) and lists
    /// the canonical service set the real device emits, OR it does not
    /// advertise (`mdns_advertise = false` AND `mdns.services` empty).
    /// "Advertise on but services empty" is the cross-layer mismatch we
    /// are fixing — a real iPhone with mDNS on always announces
    /// `_apple-mobdev2._tcp` etc.
    #[test]
    fn brand_stealth_personas_have_coherent_mdns_posture() {
        let mut chatty = 0usize;
        let mut quiet = 0usize;
        for (stem, raw) in builtin_raw_bodies() {
            let p: Persona = toml::from_str(raw)
                .unwrap_or_else(|e| panic!("builtin {stem} failed to parse: {e}"));
            if p.kind != super::super::PersonaKind::Stealth {
                continue;
            }
            if p.mdns_advertise {
                assert!(
                    !p.mdns.services.is_empty(),
                    "stealth persona '{stem}' has mdns_advertise = true but no \
                     mdns.services — that's the issue #305 cross-layer mismatch \
                     (real chatty device with no announcements identifies the cover)"
                );
                chatty += 1;
            } else {
                quiet += 1;
            }
        }
        // At least one of each posture in the catalogue.
        assert!(
            chatty >= 5,
            "expected several chatty brand personas, got {chatty}"
        );
        assert!(
            quiet >= 4,
            "expected several quiet brand personas, got {quiet}"
        );
    }

    /// Issue #305: brand stealth personas must carry a non-default
    /// `tcp_stack` profile. An empty (all-zero) profile leaves Linux
    /// defaults on the wire, which contradicts a brand cover (e.g.
    /// Windows TTL 128 vs the Linux 64).
    #[test]
    fn every_brand_stealth_persona_has_non_default_tcp_stack() {
        let mut checked = 0usize;
        for (stem, raw) in builtin_raw_bodies() {
            let p: Persona = toml::from_str(raw)
                .unwrap_or_else(|e| panic!("builtin {stem} failed to parse: {e}"));
            if p.kind != super::super::PersonaKind::Stealth {
                continue;
            }
            // Default TcpStackProfile is window_scale=0, mss=0, ttl=0,
            // both tcp flags false — nothing real ships that. Assert at
            // least one of the meaningful fields is set.
            let t = &p.tcp_stack;
            let nondefault = t.window_scale != 0
                || t.mss != 0
                || t.default_ttl != 0
                || t.tcp_timestamps
                || t.tcp_sack;
            assert!(
                nondefault,
                "stealth persona '{stem}' has all-default tcp_stack — issue #305 \
                 cross-layer mismatch (Linux defaults on a brand cover)"
            );
            checked += 1;
        }
        assert!(
            checked >= 13,
            "expected to check at least 13 stealth personas, got {checked}"
        );
    }

    /// Issue #305: per-OS-family coherence sanity. The canonical Apple
    /// personas (iphone-13/15, ipad, macbook-air-m3, macbook-pro-m3)
    /// must all set ttl=64 and timestamps=true (Darwin/XNU). Windows
    /// personas (dell-xps-13, surface-pro-9, xbox-series-x) must all
    /// set ttl=128 (Windows kernel default). A misconfigured persona
    /// here is a single-pass classifier giveaway.
    #[test]
    fn apple_personas_carry_darwin_tcp_signature() {
        for id in [
            "iphone-13",
            "iphone-15",
            "ipad-air",
            "macbook-air-m3",
            "macbook-pro-m3",
        ] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert_eq!(p.tcp_stack.default_ttl, 64, "{id} must use Darwin TTL 64");
            assert!(
                p.tcp_stack.tcp_timestamps,
                "{id} must have timestamps on (iOS/macOS)"
            );
            assert_eq!(
                p.tcp_stack.window_scale, 6,
                "{id} must use Darwin window scale 6"
            );
        }
    }

    #[test]
    fn windows_personas_carry_windows_tcp_signature() {
        for id in ["dell-xps-13", "surface-pro-9", "xbox-series-x"] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert_eq!(
                p.tcp_stack.default_ttl, 128,
                "{id} must use Windows TTL 128"
            );
            assert!(
                !p.tcp_stack.tcp_timestamps,
                "{id} must have timestamps off (Windows default)"
            );
        }
    }

    /// Issue #305: the apple-mobdev2 service is the iOS Bonjour
    /// fingerprint. Every iPhone/iPad persona must announce it — leaving
    /// it out of the service set is the exact bug the issue describes.
    #[test]
    fn apple_phone_personas_announce_mobdev2() {
        for id in ["iphone-13", "iphone-15", "ipad-air"] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert!(
                p.mdns_advertise,
                "{id} should advertise mDNS (real iOS does)"
            );
            assert!(
                p.mdns.services.iter().any(|s| s == "_apple-mobdev2._tcp"),
                "{id} must announce _apple-mobdev2._tcp (iOS Bonjour fingerprint); \
                 services = {:?}",
                p.mdns.services
            );
        }
    }

    /// Issue #305: googlecast personas (chromecast, nest-mini) must
    /// announce `_googlecast._tcp`. That's the diagnostic service Cast
    /// SDKs probe for when looking for receivers.
    #[test]
    fn cast_personas_announce_googlecast() {
        for id in ["chromecast", "nest-mini"] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert!(
                p.mdns_advertise,
                "{id} should advertise (Cast hardware does)"
            );
            assert!(
                p.mdns.services.iter().any(|s| s == "_googlecast._tcp"),
                "{id} must announce _googlecast._tcp (Cast diagnostic service); \
                 services = {:?}",
                p.mdns.services
            );
        }
    }

    /// Issue #305: printer personas must announce the IPP discovery
    /// stack. Network printers ARE their Bonjour announcements; a
    /// printer persona that doesn't emit `_ipp._tcp` is observably not
    /// a printer.
    #[test]
    fn printer_personas_announce_ipp() {
        for id in ["printer-generic-canon", "printer-generic-hp"] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert!(
                p.mdns_advertise,
                "{id} should advertise (printers are chatty)"
            );
            assert!(
                p.mdns.services.iter().any(|s| s == "_ipp._tcp"),
                "{id} must announce _ipp._tcp; services = {:?}",
                p.mdns.services
            );
        }
    }

    /// Issue #305: option-55 lists must be distinct between OS families.
    /// iOS 1-3-6-15-119-252 and Android 1-3-6-15-26-28-51-58-59-43 are
    /// the canonical signatures; Windows adds 31-33-43-44-46-47-121-249-252.
    /// A bug-fix here is the persona list collapsing to one shape — guard
    /// against that regression.
    #[test]
    fn option_55_lists_are_distinct_per_os_family() {
        let iphone = load_builtin("iphone-15").unwrap().unwrap();
        let pixel = load_builtin("pixel-8").unwrap().unwrap();
        let dell = load_builtin("dell-xps-13").unwrap().unwrap();
        let mac = load_builtin("macbook-pro-m3").unwrap().unwrap();
        // Apple iOS distinct from Apple macOS distinct from Android distinct from Windows.
        let lists = [
            iphone.dhcp_fingerprint.parameter_request_list,
            pixel.dhcp_fingerprint.parameter_request_list,
            dell.dhcp_fingerprint.parameter_request_list,
            mac.dhcp_fingerprint.parameter_request_list,
        ];
        for (i, a) in lists.iter().enumerate() {
            for b in lists.iter().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "two OS families have identical option-55 lists — that's a \
                     cross-layer collapse (issue #305): {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn list_all_returns_all_builtins_when_user_dir_missing() {
        let root = Path::new("/this/path/does/not/exist/proteus-personas");
        let all = list_all(root);
        assert!(
            all.len() >= 15,
            "list_all should surface at least the built-ins, got {}",
            all.len()
        );
        // Must be sorted.
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "list_all output must be sorted by id");
    }

    #[test]
    fn load_builtin_returns_none_for_unknown_id() {
        let r = load_builtin("definitely-not-a-real-persona-xyz").unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn load_builtin_returns_some_for_required_personas() {
        // Subset that the roadmap explicitly names.
        for id in ["iphone-15", "pixel-8", "macbook-air-m3", "samsung-tv-2024"] {
            let p = load_builtin(id)
                .unwrap_or_else(|e| panic!("loading {id}: {e}"))
                .unwrap_or_else(|| panic!("builtin {id} not found"));
            assert_eq!(p.id, id);
        }
    }

    #[test]
    fn user_persona_shadows_builtin_on_collision() {
        let dir =
            std::env::temp_dir().join(format!("proteus-persona-shadow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = r#"
id = "iphone-15"
display_name = "Custom iPhone 15"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "{owner} iPhone"
notes = "user override"
"#;
        std::fs::write(dir.join("iphone-15.toml"), body).unwrap();
        let (p, src) = load("iphone-15", &dir).unwrap().unwrap();
        assert_eq!(src, PersonaSource::User);
        assert_eq!(p.display_name, "Custom iPhone 15");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_check_rejects_non_kebab_id() {
        let p = Persona {
            id: "BadId".into(),
            display_name: "X".into(),
            kind: super::super::PersonaKind::Stealth,
            category: super::super::PersonaCategory::Generic,
            oui_pool: vec![],
            mac_byte_pattern: None,
            hostname_template: "h".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: false,
            mdns: Default::default(),
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        };
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("kebab-case"));
    }

    #[test]
    fn schema_check_requires_rotate_cadence_for_randomizer() {
        let p = Persona {
            id: "rand-x".into(),
            display_name: "X".into(),
            kind: super::super::PersonaKind::Randomizer,
            category: super::super::PersonaCategory::Generic,
            oui_pool: vec!["apple".into()],
            mac_byte_pattern: None,
            hostname_template: "h".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: false,
            mdns: Default::default(),
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        };
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("rotate_cadence"));
    }

    #[test]
    fn validate_accepts_a_well_formed_user_file() {
        let dir =
            std::env::temp_dir().join(format!("proteus-persona-validate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom.toml");
        let body = r#"
id = "custom"
display_name = "Custom"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "{owner} iPhone"
notes = "test"
"#;
        std::fs::write(&path, body).unwrap();
        let p = validate(&path).expect("validate must succeed");
        assert_eq!(p.id, "custom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #255: typo'd top-level field is rejected by serde rather
    /// than silently falling back to the default.
    #[test]
    fn unknown_top_level_field_is_rejected() {
        let body = r#"
id = "ok"
display_name = "OK"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "x"
notes = "test"
typoed_filed = "value"
"#;
        let r: std::result::Result<Persona, _> = toml::from_str(body);
        assert!(r.is_err(), "unknown field must be rejected");
    }

    /// Issue #255: typo'd nested field is rejected too.
    #[test]
    fn unknown_nested_field_is_rejected() {
        let body = r#"
id = "ok"
display_name = "OK"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "x"
notes = "test"
[rf_traits]
tx_pwr_dbm = 14
"#;
        let r: std::result::Result<Persona, _> = toml::from_str(body);
        assert!(r.is_err(), "unknown nested field must be rejected");
    }

    /// Issue #266: tx_power_dbm > 30 is rejected.
    #[test]
    fn schema_check_rejects_implausible_tx_power() {
        let mut p = sample_stealth_persona();
        p.rf_traits.tx_power_dbm = 50;
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("tx_power_dbm"));
    }

    /// Issue #266: bogus addr_gen_mode is rejected.
    #[test]
    fn schema_check_rejects_unknown_addr_gen_mode() {
        let mut p = sample_stealth_persona();
        p.ipv6_traits.addr_gen_mode = "definitely-not-a-mode".into();
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("addr_gen_mode"));
    }

    /// Issue #266: bogus scan_style is rejected.
    #[test]
    fn schema_check_rejects_unknown_scan_style() {
        let mut p = sample_stealth_persona();
        p.rf_traits.scan_style = "noisy".into();
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("scan_style"));
    }

    /// Issue #266: bogus power_save value is rejected.
    #[test]
    fn schema_check_rejects_unknown_power_save() {
        let mut p = sample_stealth_persona();
        p.rf_traits.power_save = "maybe".into();
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("power_save"));
    }

    /// Issue #266: TCP window scale > 14 is rejected.
    #[test]
    fn schema_check_rejects_window_scale_above_14() {
        let mut p = sample_stealth_persona();
        p.tcp_stack.window_scale = 20;
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("window_scale"));
    }

    /// Issue #266: option-55 entries can't be the pad code.
    #[test]
    fn schema_check_rejects_pad_in_parameter_request_list() {
        let mut p = sample_stealth_persona();
        p.dhcp_fingerprint.parameter_request_list = vec![1, 0, 6];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("parameter_request_list"));
    }

    /// Issue #266: control bytes in DHCP strings are rejected.
    #[test]
    fn schema_check_rejects_control_bytes_in_dhcp_strings() {
        let mut p = sample_stealth_persona();
        p.dhcp_fingerprint.vendor_class_identifier = "iPhone\nfake".into();
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("vendor_class_identifier"));
    }

    /// Issue #266: bogus rotate_cadence string is rejected.
    #[test]
    fn schema_check_rejects_bogus_rotate_cadence() {
        let mut p = sample_stealth_persona();
        p.kind = super::super::PersonaKind::Randomizer;
        p.rotate_cadence = Some("forever".into());
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("rotate_cadence"));
    }

    /// Issue #305: malformed mDNS service-type strings are rejected at
    /// load time so a hand-edit doesn't land bad data in state.json.
    #[test]
    fn schema_check_rejects_malformed_mdns_service_type() {
        let mut p = sample_stealth_persona();
        p.mdns.services = vec!["not-a-service-type".into()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("mdns.services"));
    }

    /// Issue #305: empty-string mDNS service entries are rejected.
    #[test]
    fn schema_check_rejects_empty_mdns_service() {
        let mut p = sample_stealth_persona();
        p.mdns.services = vec![String::new()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("mdns.services"));
    }

    /// Issue #305: well-formed DNS-SD strings are accepted.
    #[test]
    fn schema_check_accepts_well_formed_mdns_services() {
        let mut p = sample_stealth_persona();
        p.mdns.services = vec![
            "_apple-mobdev2._tcp".into(),
            "_googlecast._tcp".into(),
            "_ipp._tcp".into(),
            "_uscan._udp".into(),
        ];
        schema_check(&p).expect("well-formed DNS-SD strings must validate");
    }

    /// Issue #305: control bytes in TXT-record hints are rejected.
    #[test]
    fn schema_check_rejects_control_bytes_in_txt_hints() {
        let mut p = sample_stealth_persona();
        p.mdns.txt_hints = vec!["model=iPhone\nfake".into()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("mdns.txt_hints"));
    }

    /// Issue #305: bogus protocol part (not _tcp/_udp) is rejected.
    #[test]
    fn schema_check_rejects_unknown_mdns_protocol() {
        let mut p = sample_stealth_persona();
        p.mdns.services = vec!["_ipp._sctp".into()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("mdns.services"));
    }

    /// Issue #305: missing leading underscore on the service label is
    /// rejected (DNS-SD requires the leading underscore).
    #[test]
    fn schema_check_rejects_mdns_label_without_underscore() {
        let mut p = sample_stealth_persona();
        p.mdns.services = vec!["ipp._tcp".into()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("mdns.services"));
    }

    /// Issue #266: empty-string oui_pool entry is rejected.
    #[test]
    fn schema_check_rejects_empty_oui_token() {
        let mut p = sample_stealth_persona();
        p.oui_pool = vec!["apple".into(), String::new()];
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("oui_pool"));
    }

    /// Issue #253: list_all surfaces a malformed user persona with
    /// `valid: false` rather than dropping it.
    #[test]
    fn list_all_marks_malformed_user_persona_as_invalid() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-persona-listinvalid-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Schema-valid TOML but tx_power_dbm out of range.
        let body = r#"
id = "broken-pers"
display_name = "Broken"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "x"
notes = "test"
[rf_traits]
tx_power_dbm = 99
"#;
        std::fs::write(dir.join("broken-pers.toml"), body).unwrap();
        let all = list_all(&dir);
        let entry = all
            .iter()
            .find(|s| s.id == "broken-pers")
            .expect("broken-pers should still appear in list");
        assert!(!entry.valid);
        assert!(
            entry
                .schema_error
                .as_deref()
                .unwrap()
                .contains("tx_power_dbm")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #232: load_user refuses a malformed persona.
    #[test]
    fn load_user_rejects_persona_failing_schema_check() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-persona-loaduser-bad-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let body = r#"
id = "bad-pers"
display_name = "Bad"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "x"
notes = "test"
[rf_traits]
scan_style = "noisy"
"#;
        std::fs::write(dir.join("bad-pers.toml"), body).unwrap();
        let err = load_user("bad-pers", &dir).unwrap_err();
        assert!(err.to_string().contains("scan_style"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #231: validate_bytes parses the same bytes the caller
    /// already holds, so the caller can write exactly those bytes
    /// without re-reading the source.
    #[test]
    fn validate_bytes_succeeds_on_well_formed_toml() {
        let body = br#"
id = "from-bytes"
display_name = "FromBytes"
kind = "stealth"
category = "phone"
oui_pool = ["apple"]
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "x"
notes = "test"
"#;
        let p = validate_bytes(body, "<test>").expect("must validate");
        assert_eq!(p.id, "from-bytes");
    }

    /// Issue #231: validate_bytes rejects malformed bytes.
    #[test]
    fn validate_bytes_rejects_malformed_toml() {
        let body = b"not = [valid";
        assert!(validate_bytes(body, "<test>").is_err());
    }

    fn sample_stealth_persona() -> Persona {
        Persona {
            id: "sample".into(),
            display_name: "Sample".into(),
            kind: super::super::PersonaKind::Stealth,
            category: super::super::PersonaCategory::Phone,
            oui_pool: vec!["apple".into()],
            mac_byte_pattern: None,
            hostname_template: "{owner}s-iPhone".into(),
            dhcp_fingerprint: Default::default(),
            tcp_stack: Default::default(),
            ipv6_traits: Default::default(),
            mdns_advertise: false,
            mdns: Default::default(),
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        }
    }
}
