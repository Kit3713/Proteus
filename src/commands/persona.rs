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
use crate::display::display_safe;
use crate::exit;
use crate::persona::{Persona, PersonaCategory, PersonaKind, PersonaSource, PersonaSummary, load};

/// Default editor used when `$EDITOR` and `$VISUAL` are unset; matches
/// `proteus config edit` so the two commands have identical fallback
/// ordering (#244).
const DEFAULT_EDITOR: &str = "vi";

/// Issue GH#342 — path-traversal hardening on `persona {new,edit,show,use}`.
/// The id flows into `<root>/<id>.toml` via [`load::user_path`]. Without a
/// strict allow-list a caller could pass `../../etc/passwd` and the loader
/// would stat / write under `/etc/passwd.toml` (or, with explicit chars,
/// land at the literal `/etc/passwd`). Mirror the kebab-case rule that
/// the persona-schema validator enforces on the on-disk `id` field.
///
/// The grammar is intentionally narrower than what `Persona.id` itself
/// allows (`load::is_kebab_case`): we accept ASCII letters, digits, `-`,
/// and `_`, refuse leading/trailing `-` or `_`, and refuse the literal
/// `.` and `..`. The `_` makes `is_valid_persona_id` a defense-in-depth
/// gate: a hand-authored persona file with an underscore in its id would
/// already be rejected at load by `is_kebab_case`, so this check just
/// stops the path-traversal attack on its way *to* the loader.
fn is_valid_persona_id(id: &str) -> bool {
    if id.is_empty() || id == "." || id == ".." {
        return false;
    }
    // Cap length so an attacker can't smuggle a multi-kilobyte path
    // through here. 64 bytes is well above any persona id we ship.
    if id.len() > 64 {
        return false;
    }
    let bytes = id.as_bytes();
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    if first == b'-' || first == b'_' || last == b'-' || last == b'_' {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}

/// Reject + report. Returns `Err(rc)` shape: callers `Ok(rc)?` style isn't
/// readable here, so we keep the explicit `if let` at the call site.
fn reject_invalid_id(id: &str) -> u8 {
    // Sanitize for the error message: an attacker-crafted id may itself
    // contain ANSI escapes.
    eprintln!(
        "proteus: invalid persona id '{}': must match [A-Za-z0-9_-], 1..=64 chars, no leading/trailing '-'/'_'",
        display_safe(id),
    );
    exit::CONFIG_ERROR
}

/// GH#361 helper — reject `$EDITOR` / `$VISUAL` values containing control
/// bytes. Non-ASCII passes through (some operators legitimately use
/// editor names with extended chars in their PATH); only NUL and the C0
/// range are refused. The `OsStr` form handles both UTF-8 and platform-
/// encoded byte content uniformly.
fn is_safe_editor_value(v: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let bytes = v.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    !bytes.iter().any(|b| *b == 0 || (*b < 0x20) || *b == 0x7f)
}

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
        PersonaAction::Export {
            id,
            path,
            yes,
            force,
        } => export(&id, &path, yes, force, &user_root),
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
    // Issues #374/#380: persona display_name and notes can come from
    // user-authored TOML under /etc/proteus/personas/. Even though the
    // schema validator (`persona/load.rs::schema_check`) rejects control
    // bytes in DHCP strings, it does NOT yet do so for free-form
    // display_name / notes — and a hand-edit can plant ANSI/BiDi there.
    // Sanitize on the way out. The id is kebab-case-validated at load
    // time, but defense-in-depth: route it through display_safe too.
    for s in &all {
        let marker = if s.valid { "" } else { " [INVALID]" };
        println!(
            "{:<28} {:<10} {:<8} {:<7}  {}{marker}",
            display_safe(&s.id),
            s.kind.name(),
            s.category.name(),
            s.source.name(),
            display_safe(&s.display_name),
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
    // GH#342: gate path-component args before they reach the loader.
    if !is_valid_persona_id(id) {
        return Ok(reject_invalid_id(id));
    }
    let Some((p, src)) = load::load(id, user_root)? else {
        eprintln!(
            "proteus: persona '{}' not found (try `proteus persona list`)",
            display_safe(id)
        );
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
    // GH#342: gate path-component args before they reach the loader.
    if !is_valid_persona_id(id) {
        return Ok(reject_invalid_id(id));
    }
    if load::load(id, user_root)?.is_none() {
        eprintln!(
            "proteus: persona '{}' not found (try `proteus persona list`)",
            display_safe(id)
        );
        return Ok(exit::CONFIG_ERROR);
    }
    write_active_to_config(Some(id), config_override)?;
    println!("active persona is now '{}'", display_safe(id));
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
    // GH#342: both `id` (target file name) and `from` (loader path
    // component) flow into `<root>/<*>.toml` paths. Gate both.
    if !is_valid_persona_id(id) {
        return Ok(reject_invalid_id(id));
    }
    if !is_valid_persona_id(from) {
        return Ok(reject_invalid_id(from));
    }
    let Some((mut p, _)) = load::load(from, user_root)? else {
        eprintln!("proteus: source persona '{}' not found", display_safe(from));
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
    // GH#342: gate path-component arg before joining with user_root.
    if !is_valid_persona_id(id) {
        return Ok(reject_invalid_id(id));
    }
    let path = load::user_path(user_root, id);
    if !path.exists() {
        eprintln!(
            "proteus: {} does not exist (use `proteus persona new {} --from <existing>` first)",
            path.display(),
            display_safe(id),
        );
        return Ok(exit::CONFIG_ERROR);
    }
    // GH#361 — `persona edit` previously ran $EDITOR as root with the
    // caller's $HOME, making the editor's plugins / autoloads execute
    // root-owned code from a user-writable dotfile. Refuse the run when
    // we detect this shape rather than just warning. The mitigations:
    //   1. `sudo -H proteus persona edit ...` — sudo resets HOME to root's.
    //   2. SUDO_USER unset (direct root login) — HOME is /root by build
    //      convention.
    //   3. `proteus persona edit ...` invoked as a non-root user — the
    //      `require_root` gate above already refused.
    // Issues #230 / #244 also asked for a warning here; we KEEP the
    // warning text for diagnosability but bump the policy from "warn and
    // continue" to "warn and refuse" because root-as-user-editor is the
    // class of bug `sudo -H` solves and we shouldn't ship the foot-gun
    // (the existing `--yes`-style override is `sudo -H`).
    if std::env::var_os("HOME").is_some_and(|h| h != *"/root") {
        eprintln!(
            "proteus: refusing to launch editor: $HOME is not /root, so the editor's \
             plugins / autoloads would run as root from a user-writable directory."
        );
        eprintln!(
            "proteus: re-run with `sudo -H proteus persona edit ...` (drops HOME) \
             or edit {} manually.",
            path.display()
        );
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #244: $VISUAL beats $EDITOR; fall back to vi when neither is
    // set. Same precedence as `proteus config edit`.
    //
    // GH#361: refuse an obviously-attacker-shaped $EDITOR/$VISUAL
    // (newline / NUL / shell metacharacters). The OsString form means
    // we can't regex it, but we can check for control bytes which is
    // enough to catch the documented attack shape (an attacker-set
    // EDITOR injecting a fresh argv via `\n`).
    let editor_var = std::env::var_os("VISUAL").or_else(|| std::env::var_os("EDITOR"));
    let editor = if let Some(v) = editor_var {
        if !is_safe_editor_value(&v) {
            eprintln!("proteus: refusing to spawn editor: $VISUAL/$EDITOR contains control bytes.");
            return Ok(exit::CONFIG_ERROR);
        }
        v
    } else {
        OsString::from(DEFAULT_EDITOR)
    };
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

/// Issue #286: bring `export` up to import-parity. Without these guards a
/// typo like `proteus persona export iphone-15 /etc/sudoers` (intended
/// `~/sudoers.toml`) silently overwrites root-owned config files; a symlink
/// pre-placed at the destination follows through to whatever it targets.
///
/// Parity surface, mirroring [`import`]:
/// * `--yes` is required (uses [`super::require_yes`]).
/// * The target must not be a symlink — checked via `lstat` so the symlink
///   itself is examined rather than its target.
/// * An existing regular file blocks the export; `--force` is the explicit
///   opt-in for overwrite, but even `--force` does not bypass the symlink
///   check (a symlink with `--force` is still refused).
/// * The write goes through [`super::write_atomic`] for the same TOCTOU /
///   crash-safety guarantees the rest of the codebase relies on.
fn export(id: &str, path: &Path, yes: bool, force: bool, user_root: &Path) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "writes a persona TOML to the given path",
        "proteus help persona",
    ) {
        return Ok(code);
    }
    let Some((p, src)) = load::load(id, user_root)? else {
        eprintln!("proteus: persona '{id}' not found");
        return Ok(exit::CONFIG_ERROR);
    };
    // lstat-based existence/type check: symlink_metadata does NOT follow
    // symlinks, so a symlink pre-placed at `path` is detected as a symlink
    // rather than reporting the target's type. NotFound is the happy path
    // (we'll create the file fresh); any other error gets bubbled to the
    // operator.
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                eprintln!(
                    "proteus: refusing to export through symlink {} (security)",
                    path.display()
                );
                return Ok(exit::CONFIG_ERROR);
            }
            if !force {
                eprintln!(
                    "proteus: {} already exists; pass --force to overwrite",
                    path.display()
                );
                return Ok(exit::CONFIG_ERROR);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::Error::from(e)).with_context(|| format!("stat {}", path.display()));
        }
    }
    let body = toml::to_string_pretty(&p).context("rendering persona TOML")?;
    super::write_atomic(path, body.as_bytes())?;
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

    /// Issue #286: `--yes` and `--force` parse on the export subcommand,
    /// matching `import`'s shape.
    #[test]
    fn cli_parses_export_with_yes_and_force() {
        let w = Wrap::try_parse_from(["x", "export", "iphone-15", "/tmp/p.toml", "--yes"])
            .expect("parse");
        match w.cmd {
            PersonaAction::Export {
                id,
                path,
                yes,
                force,
            } => {
                assert_eq!(id, "iphone-15");
                assert_eq!(path.to_str(), Some("/tmp/p.toml"));
                assert!(yes);
                assert!(!force);
            }
            _ => panic!("wrong action"),
        }

        let w2 = Wrap::try_parse_from([
            "x",
            "export",
            "iphone-15",
            "/tmp/p.toml",
            "--yes",
            "--force",
        ])
        .expect("parse");
        match w2.cmd {
            PersonaAction::Export { yes, force, .. } => {
                assert!(yes);
                assert!(force);
            }
            _ => panic!("wrong action"),
        }
    }

    /// Issue #286 — without `--yes`, export bails with the confirmation
    /// exit code and writes nothing. The wiki-hint string matches the
    /// rest of the persona surface (`proteus help persona`).
    #[test]
    fn export_refuses_without_yes() {
        let tmp = crate::testing::TempRoot::new("persona-export-noyes");
        let dest = tmp.path.join("out.toml");
        // user_root is irrelevant here — the require_yes gate trips first.
        let user_root = std::path::Path::new("/dev/null/persona-root");
        let rc = export("iphone-15", &dest, false, false, user_root).expect("call ok");
        assert_eq!(rc, crate::exit::CONFIRMATION_REQUIRED);
        assert!(!dest.exists(), "export must not have written without --yes");
    }

    /// Issue #286 — refusing to overwrite is the default. An existing
    /// regular file at the destination yields CONFIG_ERROR and the file
    /// is left untouched.
    #[test]
    fn export_refuses_overwrite_without_force() {
        let tmp = crate::testing::TempRoot::new("persona-export-over");
        let dest = tmp.path.join("out.toml");
        std::fs::write(&dest, b"DO NOT TOUCH").unwrap();
        let rc = export("iphone-15", &dest, true, false, &tmp.path).expect("call ok");
        assert_eq!(rc, crate::exit::CONFIG_ERROR);
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"DO NOT TOUCH",
            "destination must not be touched on overwrite refusal"
        );
    }

    /// Issue #286 — `--force` permits overwriting a regular file. The
    /// new contents are valid persona TOML for the picked builtin.
    #[test]
    fn export_force_overwrites_existing_regular_file() {
        let tmp = crate::testing::TempRoot::new("persona-export-force");
        let dest = tmp.path.join("out.toml");
        std::fs::write(&dest, b"old contents").unwrap();
        let rc = export("iphone-15", &dest, true, true, &tmp.path).expect("call ok");
        assert_eq!(rc, crate::exit::SUCCESS);
        let body = std::fs::read_to_string(&dest).unwrap();
        assert!(
            body.contains("id = \"iphone-15\""),
            "force-overwritten file must contain new persona body, got: {body}"
        );
    }

    /// Issue #286 — a symlink at the destination is refused even with
    /// `--force`. lstat-based check prevents following the link to its
    /// target (which could be `/etc/sudoers` or similar).
    #[test]
    fn export_refuses_symlink_destination_even_with_force() {
        let tmp = crate::testing::TempRoot::new("persona-export-sym");
        let target = tmp.path.join("victim");
        std::fs::write(&target, b"sensitive").unwrap();
        let dest = tmp.path.join("link");
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        // --force is set but symlinks must be refused regardless.
        let rc = export("iphone-15", &dest, true, true, &tmp.path).expect("call ok");
        assert_eq!(
            rc,
            crate::exit::CONFIG_ERROR,
            "symlink destination must be refused even with --force"
        );
        // Target file must be untouched.
        assert_eq!(std::fs::read(&target).unwrap(), b"sensitive");
        // The symlink itself stays in place.
        let meta = std::fs::symlink_metadata(&dest).unwrap();
        assert!(meta.file_type().is_symlink());
    }

    /// Issue #286 — happy path: writing to a fresh path lands a 0o600
    /// file via `write_atomic` and exits SUCCESS.
    #[test]
    fn export_writes_atomically_to_fresh_path() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = crate::testing::TempRoot::new("persona-export-fresh");
        let dest = tmp.path.join("out.toml");
        let rc = export("iphone-15", &dest, true, false, &tmp.path).expect("call ok");
        assert_eq!(rc, crate::exit::SUCCESS);
        assert!(dest.exists(), "export must have created the destination");
        let body = std::fs::read_to_string(&dest).unwrap();
        assert!(body.contains("id = \"iphone-15\""));
        // write_atomic lands files at 0o600.
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "exported file must be 0o600, got 0o{mode:o}");
    }
}
