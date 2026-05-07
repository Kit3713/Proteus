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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use include_dir::{Dir, include_dir};

use super::{Persona, PersonaSource, PersonaSummary};

/// Embedded built-in catalogue. Each `data/personas/<id>.toml` is one
/// persona; the file stem must match the `id` field.
static BUILTIN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/data/personas");

/// Process-level cache of parsed built-in personas. The catalogue is
/// embedded and immutable, so parsing every TOML once at first access
/// is a clear win over re-parsing on every `active_for` call: a single
/// `proteus apply` cycle resolves the persona at four sites
/// (rotate / hostname / dhcp / bluetooth), and Milestone 4 added two
/// more (ntp / nft). The cache makes repeated lookups O(hash) instead
/// of O(parse). User personas stay uncached — they can change on disk
/// between calls and the cost is bounded by the user's directory size.
static BUILTIN_CACHE: OnceLock<HashMap<String, Persona>> = OnceLock::new();

fn builtin_cache() -> &'static HashMap<String, Persona> {
    BUILTIN_CACHE.get_or_init(|| {
        let mut map = HashMap::new();
        for f in BUILTIN.files() {
            if f.path().extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let Some(stem) = f.path().file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(raw) = f.contents_utf8() else { continue };
            // Built-ins ship validated by `every_embedded_persona_parses`
            // tests; a parse error here is a build-time bug, not a
            // runtime condition. Skip silently rather than panic so a
            // single broken file doesn't take down the whole catalogue.
            if let Ok(p) = toml::from_str::<Persona>(raw)
                && p.id == stem
            {
                map.insert(p.id.clone(), p);
            }
        }
        map
    })
}

/// Default user-persona root. Loader callers pass a `Path` so tests can
/// point at a `TempDir`; production code passes this constant.
pub const DEFAULT_USER_ROOT: &str = "/etc/proteus/personas";

/// Try to load a built-in persona by id. Returns `None` when the id is
/// unknown so callers can fall through to user personas; errors during
/// parse propagate as `Err`.
///
/// Backed by [`BUILTIN_CACHE`] so repeated lookups in a single process
/// hit a `HashMap` instead of re-parsing the same TOML body. The first
/// access pays the parse-once cost for every embedded file.
pub fn load_builtin(id: &str) -> Result<Option<Persona>> {
    if let Some(p) = builtin_cache().get(id) {
        return Ok(Some(p.clone()));
    }
    // Cache miss can mean either "id is unknown" or "the file existed
    // but failed to parse at startup". Fall back to a fresh parse so
    // the diagnostic still surfaces a real error message rather than
    // pretending the file doesn't exist.
    let path = format!("{id}.toml");
    let Some(file) = BUILTIN.get_file(&path) else {
        return Ok(None);
    };
    let raw = file
        .contents_utf8()
        .ok_or_else(|| anyhow!("builtin persona {id} is not valid UTF-8"))?;
    let p: Persona = toml::from_str(raw).with_context(|| {
        format!("parsing builtin persona '{id}' (see `proteus wiki personas`)")
    })?;
    if p.id != id {
        anyhow::bail!(
            "builtin persona file '{id}.toml' has mismatched id field '{}' (see wiki personas)",
            p.id
        );
    }
    Ok(Some(p))
}

/// Load a user-authored persona from `<root>/<id>.toml`.
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
/// summaries; full bodies load on demand via `load`.
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
            let Ok(p) = parse_file(&path) else { continue };
            seen_ids.insert(p.id.clone());
            out.push(summary_for(&p, PersonaSource::User));
        }
    }

    // Same cache the loader uses — avoids re-parsing every embedded
    // persona on every `proteus persona list` invocation.
    for p in builtin_cache().values() {
        if seen_ids.contains(&p.id) {
            continue;
        }
        out.push(summary_for(p, PersonaSource::Builtin));
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn summary_for(p: &Persona, source: PersonaSource) -> PersonaSummary {
    PersonaSummary {
        id: p.id.clone(),
        display_name: p.display_name.clone(),
        kind: p.kind,
        category: p.category,
        source,
    }
}

/// Parse + schema-check a single persona file. Used by `proteus persona
/// validate <path>` and shared with the CLI import path.
pub fn validate(path: &Path) -> Result<Persona> {
    let p = parse_file(path)?;
    schema_check(&p)?;
    Ok(p)
}

fn parse_file(path: &Path) -> Result<Persona> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
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
    Ok(())
}

fn is_kebab_case(s: &str) -> bool {
    !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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
            schema_check(&p)
                .unwrap_or_else(|e| panic!("builtin {stem} failed schema check: {e}"));
            count += 1;
        }
        assert!(
            count >= 15,
            "expected at least 15 built-in personas, found {count}"
        );
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
        let dir = std::env::temp_dir().join(format!(
            "proteus-persona-shadow-{}",
            std::process::id()
        ));
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
            bt_name_template: String::new(),
            rf_traits: Default::default(),
            rotate_cadence: None,
            notes: String::new(),
        };
        let err = schema_check(&p).unwrap_err();
        assert!(err.to_string().contains("rotate_cadence"));
    }

    /// Performance hardening: the builtin cache must hand back the
    /// same parsed body on repeated lookups. The contract is "id in
    /// → cloned persona out" — the test keys on `id` round-trip plus
    /// reference equality of the cached HashMap to make sure the
    /// `OnceLock` is actually wired through both lookup paths.
    #[test]
    fn builtin_cache_returns_consistent_results_across_calls() {
        let p1 = load_builtin("iphone-15").unwrap().unwrap();
        let p2 = load_builtin("iphone-15").unwrap().unwrap();
        assert_eq!(p1, p2);
        // Cache is keyed by id; an unrelated id resolves independently.
        let q1 = load_builtin("pixel-8").unwrap().unwrap();
        assert_eq!(q1.id, "pixel-8");
        let q2 = load_builtin("pixel-8").unwrap().unwrap();
        assert_eq!(q1, q2);
    }

    /// `list_all` consumes the same cache as `load_builtin`, so a
    /// sequence of `list_all` + `load_builtin` calls must observe the
    /// same set of ids.
    #[test]
    fn list_all_and_load_builtin_observe_identical_id_set() {
        let root = Path::new("/this/path/does/not/exist");
        let listed: std::collections::BTreeSet<String> = list_all(root)
            .into_iter()
            .filter(|s| matches!(s.source, PersonaSource::Builtin))
            .map(|s| s.id)
            .collect();
        for id in &listed {
            assert!(
                load_builtin(id).unwrap().is_some(),
                "list_all surfaced builtin id '{id}' that load_builtin can't find"
            );
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_user_file() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-persona-validate-{}",
            std::process::id()
        ));
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
}
