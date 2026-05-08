// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus persona` subcommand handlers — roadmap Milestone 2.
//!
//! Read commands (`list`, `show`, `current`, `random`, `validate`,
//! `export`) work for any user. Mutating commands (`use`, `clear`,
//! `new`, `edit`, `import`) require root because they write under
//! `/etc/proteus/`.
//!
//! Integration with the apply / rotate paths is the follow-up tracked
//! by roadmap Milestone 2 "Integration"; today `use`/`clear` only flip
//! `[persona] active` in the config file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::PersonaAction;
use crate::config::Config;
use crate::exit;
use crate::persona::{Persona, PersonaCategory, PersonaKind, PersonaSource, PersonaSummary, load};

/// Default editor used when `$EDITOR` and `$VISUAL` are unset; matches
/// `proteus config edit` so the two commands have identical fallback
/// ordering (#244).
const DEFAULT_EDITOR: &str = "vi";

/// Top-level dispatch for `proteus persona ...`.
pub fn run(action: PersonaAction, config_override: Option<&Path>) -> Result<u8> {
    let user_root = user_root();
    match action {
        PersonaAction::List {
            kind,
            category,
            json,
        } => list(kind.as_deref(), category.as_deref(), json, &user_root),
        PersonaAction::Show { id, json } => show(&id, json, &user_root),
        PersonaAction::Use { id, apply, yes } => {
            use_persona(&id, apply, yes, config_override, &user_root)
        }
        PersonaAction::Clear { yes } => clear(yes, config_override),
        PersonaAction::Current { json } => current(json, config_override, &user_root),
        PersonaAction::Random {
            kind,
            category,
            json,
        } => random(kind.as_deref(), category.as_deref(), json, &user_root),
        PersonaAction::New { id, from, yes } => new(&id, &from, yes, &user_root),
        PersonaAction::Edit { id } => edit(&id, &user_root),
        PersonaAction::Validate { path } => validate(&path),
        PersonaAction::Import { path, yes } => import(&path, yes, &user_root),
        PersonaAction::Export { id, path } => export(&id, &path, &user_root),
    }
}

/// Default user-persona directory; tests can override via the loader's
/// `user_root` arg, but the CLI commands take the system path as the
/// canonical (root-only) location.
fn user_root() -> PathBuf {
    PathBuf::from(load::DEFAULT_USER_ROOT)
}

// ---- list ------------------------------------------------------------

fn list(kind: Option<&str>, category: Option<&str>, json: bool, user_root: &Path) -> Result<u8> {
    let kind_filter = parse_kind_filter(kind)?;
    let cat_filter = parse_category_filter(category)?;
    let mut all = load::list_all(user_root);
    all.retain(|p| match kind_filter {
        Some(k) => p.kind == k,
        None => true,
    });
    all.retain(|p| match cat_filter {
        Some(c) => p.category == c,
        None => true,
    });
    if json {
        super::print_json(&all)?;
        return Ok(exit::SUCCESS);
    }
    if all.is_empty() {
        println!("(no personas match the filter)");
        return Ok(exit::SUCCESS);
    }
    for s in &all {
        let marker = if s.valid { "" } else { " [INVALID]" };
        println!(
            "{:<28} {:<10} {:<8} {:<7}  {}{marker}",
            s.id,
            s.kind.name(),
            s.category.name(),
            s.source.name(),
            s.display_name,
        );
    }
    Ok(exit::SUCCESS)
}

fn parse_kind_filter(s: Option<&str>) -> Result<Option<PersonaKind>> {
    let Some(v) = s else { return Ok(None) };
    PersonaKind::parse(v)
        .map(Some)
        .with_context(|| format!("unknown --kind '{v}' (see `proteus wiki personas`)"))
}

fn parse_category_filter(s: Option<&str>) -> Result<Option<PersonaCategory>> {
    let Some(v) = s else { return Ok(None) };
    PersonaCategory::parse(v)
        .map(Some)
        .with_context(|| format!("unknown --category '{v}' (see `proteus wiki personas`)"))
}

// ---- show ------------------------------------------------------------

#[derive(Serialize)]
struct ShowReport {
    source: &'static str,
    persona: Persona,
}

fn show(id: &str, json: bool, user_root: &Path) -> Result<u8> {
    let Some((p, src)) = load::load(id, user_root)? else {
        eprintln!("proteus: persona '{id}' not found (try `proteus persona list`)");
        return Ok(exit::CONFIG_ERROR);
    };
    if json {
        super::print_json(&ShowReport {
            source: src.name(),
            persona: p,
        })?;
        return Ok(exit::SUCCESS);
    }
    let body = toml::to_string_pretty(&p).context("rendering persona TOML")?;
    println!("# source: {}", src.name());
    print!("{body}");
    Ok(exit::SUCCESS)
}

// ---- use / clear ------------------------------------------------------

fn use_persona(
    id: &str,
    apply: bool,
    yes: bool,
    config_override: Option<&Path>,
    user_root: &Path,
) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "writes [persona] active to config",
        "proteus help persona",
    ) {
        return Ok(code);
    }
    if load::load(id, user_root)?.is_none() {
        eprintln!("proteus: persona '{id}' not found (try `proteus persona list`)");
        return Ok(exit::CONFIG_ERROR);
    }
    write_active_to_config(Some(id), config_override)?;
    println!("active persona is now '{id}'");
    if apply {
        // The persona-shaping integration is live (roadmap M2
        // "Integration"), but `--apply` doesn't auto-rerun the
        // mutating apply orchestrator — that needs `--yes` plus root
        // and a confirmed plan. Point the operator at the next step.
        eprintln!(
            "proteus: persona is set; run `proteus apply --yes` (as root) to push the new shape onto NetworkManager"
        );
    }
    Ok(exit::SUCCESS)
}

fn clear(yes: bool, config_override: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "clears [persona] active in config",
        "proteus help persona",
    ) {
        return Ok(code);
    }
    write_active_to_config(None, config_override)?;
    println!("persona cleared; back to plain randomizer mode");
    Ok(exit::SUCCESS)
}

/// Read the user's config (creating an empty one if absent), set
/// `[persona] active`, write atomically. Uses `toml_edit` so non-persona
/// sections, comments, and key ordering are preserved.
fn write_active_to_config(active: Option<&str>, config_override: Option<&Path>) -> Result<()> {
    let path = super::config_path(config_override);
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = if raw.is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        raw.parse()
            .with_context(|| format!("parsing {}", path.display()))?
    };
    let table = doc
        .entry("persona")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .context("[persona] is not a table in the config file")?;
    match active {
        Some(id) => {
            table.insert("active", toml_edit::value(id));
        }
        None => {
            table.remove("active");
        }
    }
    super::write_atomic(&path, doc.to_string().as_bytes())?;
    Ok(())
}

// ---- current ---------------------------------------------------------

#[derive(Serialize)]
struct CurrentReport {
    active: Option<String>,
    source: Option<&'static str>,
    kind: Option<&'static str>,
    category: Option<&'static str>,
    persona_shaped_fields: Vec<&'static str>,
}

fn current(json: bool, config_override: Option<&Path>, user_root: &Path) -> Result<u8> {
    let cfg = Config::default_or_loaded(&super::config_path(config_override)).unwrap_or_default();
    let active = cfg.persona.active.clone();
    let mut report = CurrentReport {
        active: active.clone(),
        source: None,
        kind: None,
        category: None,
        persona_shaped_fields: persona_shaped_fields(),
    };
    if let Some(id) = active.as_deref()
        && let Some((p, src)) = load::load(id, user_root)?
    {
        report.source = Some(src.name());
        report.kind = Some(p.kind.name());
        report.category = Some(p.category.name());
    }
    if json {
        super::print_json(&report)?;
        return Ok(exit::SUCCESS);
    }
    match &report.active {
        None => println!("active persona: (none) — plain randomizer mode"),
        Some(id) => {
            println!("active persona: {id}");
            if let Some(src) = report.source {
                println!("  source:   {src}");
            }
            if let Some(k) = report.kind {
                println!("  kind:     {k}");
            }
            if let Some(c) = report.category {
                println!("  category: {c}");
            }
        }
    }
    println!("\nfields a persona shapes (integration is the follow-up):");
    for f in &report.persona_shaped_fields {
        println!("  - {f}");
    }
    Ok(exit::SUCCESS)
}

/// Names match the `Persona` struct fields. Surfaced verbatim in
/// `proteus persona current` so users see what is in scope.
fn persona_shaped_fields() -> Vec<&'static str> {
    vec![
        "oui_pool",
        "mac_byte_pattern",
        "hostname_template",
        "dhcp_fingerprint",
        "tcp_stack",
        "ipv6_traits",
        "mdns_advertise",
        "mdns",
        "bt_name_template",
        "rf_traits",
        "rotate_cadence",
    ]
}

// ---- random ----------------------------------------------------------

fn random(kind: Option<&str>, category: Option<&str>, json: bool, user_root: &Path) -> Result<u8> {
    let kind_filter = parse_kind_filter(kind)?;
    let cat_filter = parse_category_filter(category)?;
    let mut pool: Vec<PersonaSummary> = load::list_all(user_root);
    // Skip personas that fail schema check so `random` never picks one
    // that `use` would then refuse to load (#232 / #253 interaction).
    pool.retain(|p| p.valid);
    pool.retain(|p| kind_filter.is_none_or(|k| p.kind == k));
    pool.retain(|p| cat_filter.is_none_or(|c| p.category == c));
    if pool.is_empty() {
        eprintln!("proteus: no personas match the filter");
        return Ok(exit::CONFIG_ERROR);
    }
    // Issue #226: rejection-sampled u64 picker. The persona pool can
    // grow beyond 256 once users start importing custom personas via
    // `proteus persona import`, so we use the u64-stream variant rather
    // than the byte-stream one. For the small built-in pools this is
    // identical to the cheaper byte path in observable behaviour.
    let idx = crate::rand::unbiased_index_u64(pool.len(), crate::rand::getrandom_u64)?;
    let pick = &pool[idx];
    if json {
        super::print_json(pick)?;
        return Ok(exit::SUCCESS);
    }
    println!("{}", pick.id);
    Ok(exit::SUCCESS)
}

// ---- new / edit / validate / import / export -------------------------

fn new(id: &str, from: &str, yes: bool, user_root: &Path) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "creates a new persona file under /etc/proteus/personas/",
        "proteus help persona",
    ) {
        return Ok(code);
    }
    let Some((mut p, _)) = load::load(from, user_root)? else {
        eprintln!("proteus: source persona '{from}' not found");
        return Ok(exit::CONFIG_ERROR);
    };
    p.id = id.to_string();
    let dest = load::user_path(user_root, id);
    if dest.exists() {
        eprintln!(
            "proteus: {} already exists; refusing to overwrite",
            dest.display()
        );
        return Ok(exit::CONFIG_ERROR);
    }
    let body = toml::to_string_pretty(&p).context("serializing cloned persona")?;
    super::write_atomic(&dest, body.as_bytes())?;
    println!("created {} (cloned from '{}')", dest.display(), from);
    Ok(exit::SUCCESS)
}

fn edit(id: &str, user_root: &Path) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let path = load::user_path(user_root, id);
    if !path.exists() {
        eprintln!(
            "proteus: {} does not exist (use `proteus persona new {id} --from <existing>` first)",
            path.display()
        );
        return Ok(exit::CONFIG_ERROR);
    }
    // Issues #230 / #244: same HOME-not-/root warning as `proteus config
    // edit`. `sudo -E` (or env_keep) preserves HOME, which makes the
    // editor's plugins / autoloads run as root from the user's HOME — an
    // arbitrary-code-as-root path from a malicious dotfile. Surface the
    // risk so the operator can choose `sudo -H proteus persona edit`.
    if std::env::var_os("HOME").is_some_and(|h| h != *"/root") {
        eprintln!(
            "proteus: warning: $HOME is not /root — your editor's plugins / autoloads will run as root"
        );
        eprintln!(
            "proteus: prefer `sudo -H proteus persona edit` (drops HOME) or edit the file manually"
        );
    }
    // Issue #244: $VISUAL beats $EDITOR; fall back to vi when neither is
    // set. Same precedence as `proteus config edit`.
    let editor = std::env::var_os("VISUAL")
        .or_else(|| std::env::var_os("EDITOR"))
        .unwrap_or_else(|| OsString::from(DEFAULT_EDITOR));
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("spawning editor {editor:?}"))?;
    if !status.success() {
        eprintln!("proteus: editor exited non-zero ({status})");
        return Ok(exit::GENERIC_ERROR);
    }
    // Reparse to give the user immediate feedback if they introduced a typo.
    match load::validate(&path) {
        Ok(_) => {
            println!("validated {}", path.display());
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: {e:#}");
            Ok(exit::CONFIG_ERROR)
        }
    }
}

fn validate(path: &Path) -> Result<u8> {
    match load::validate(path) {
        Ok(p) => {
            println!(
                "ok: {} (id='{}', kind={})",
                path.display(),
                p.id,
                p.kind.name()
            );
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: {e:#}");
            Ok(exit::CONFIG_ERROR)
        }
    }
}

fn import(path: &Path, yes: bool, user_root: &Path) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "copies a persona file into /etc/proteus/personas/",
        "proteus help persona",
    ) {
        return Ok(code);
    }
    // Issue #231: read once, validate the bytes we read, write the same
    // bytes. The previous flow called validate(path) (which re-read the
    // file) and then std::fs::read(path) again — a swapped source file
    // between the two reads landed bytes that had never been validated.
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading source persona {}", path.display()))?;
    let p = load::validate_bytes(&bytes, &path.display().to_string())
        .context("source file failed schema check")?;
    let dest = load::user_path(user_root, &p.id);
    if dest.exists() {
        eprintln!(
            "proteus: {} already exists; refusing to overwrite",
            dest.display()
        );
        return Ok(exit::CONFIG_ERROR);
    }
    super::write_atomic(&dest, &bytes)?;
    println!("imported '{}' to {}", p.id, dest.display());
    Ok(exit::SUCCESS)
}

fn export(id: &str, path: &Path, user_root: &Path) -> Result<u8> {
    let Some((p, src)) = load::load(id, user_root)? else {
        eprintln!("proteus: persona '{id}' not found");
        return Ok(exit::CONFIG_ERROR);
    };
    let body = toml::to_string_pretty(&p).context("rendering persona TOML")?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    if matches!(src, PersonaSource::Builtin) {
        eprintln!(
            "proteus: note — exported a built-in persona; world-readable permissions on {} are your call",
            path.display()
        );
    }
    println!("exported '{id}' to {}", path.display());
    Ok(exit::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Wrap {
        #[command(subcommand)]
        cmd: PersonaAction,
    }

    #[test]
    fn cli_parses_list_with_kind_filter() {
        let w = Wrap::try_parse_from(["x", "list", "--kind=stealth"]).expect("parse");
        match w.cmd {
            PersonaAction::List { kind, .. } => {
                assert_eq!(kind.as_deref(), Some("stealth"));
            }
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_use_with_apply_flag() {
        let w = Wrap::try_parse_from(["x", "use", "iphone-15", "--apply", "--yes"]).expect("parse");
        match w.cmd {
            PersonaAction::Use { id, apply, yes } => {
                assert_eq!(id, "iphone-15");
                assert!(apply);
                assert!(yes);
            }
            _ => panic!("wrong action"),
        }
    }

    #[test]
    fn cli_parses_validate_with_path() {
        let w = Wrap::try_parse_from(["x", "validate", "/tmp/foo.toml"]).expect("parse");
        match w.cmd {
            PersonaAction::Validate { path } => {
                assert_eq!(path.to_str(), Some("/tmp/foo.toml"));
            }
            _ => panic!("wrong action"),
        }
    }

    /// Issue #302: `write_active_to_config` is the inner writer for
    /// `persona use` / `persona clear`. It must accept a `--config`
    /// path that does not yet exist by treating "missing file" as
    /// "empty doc, will be created on write" — same shape as
    /// `proteus config edit`.
    #[test]
    fn write_active_to_config_creates_missing_file() {
        let dir = std::env::temp_dir().join(format!(
            "proteus-persona-missing-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        assert!(!path.exists());
        write_active_to_config(Some("randomizer-med"), Some(&path)).unwrap();
        assert!(path.exists());
        let cfg = Config::default_or_loaded(&path).unwrap();
        assert_eq!(cfg.persona.active.as_deref(), Some("randomizer-med"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
