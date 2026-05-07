// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus config` subcommand family — first-class CLI for managing
//! `/etc/proteus/config.toml` so users don't have to hand-edit it.
//!
//! Mutating commands require root + `--yes`. Read commands work as any user.
//! Round-trips through `toml_edit` so user comments and formatting survive.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use toml_edit::{DocumentMut, Item, Value};

use crate::config::Config;
use crate::exit;

/// Default editor used when `$EDITOR` and `$VISUAL` are unset.
const DEFAULT_EDITOR: &str = "vi";

#[derive(Serialize)]
struct GetReport<'a> {
    key: &'a str,
    value: serde_json::Value,
}

#[derive(Serialize)]
struct ValidateReport {
    ok: bool,
    path: String,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct KeyEntry {
    key: String,
    #[serde(rename = "type")]
    kind: &'static str,
    default: serde_json::Value,
}

// ---------- Commands ----------

pub fn show(json: bool, config: Option<&Path>) -> Result<u8> {
    super::show_config::run(json, config)
}

pub fn get(key: &str, json: bool, config: Option<&Path>) -> Result<u8> {
    let path = super::config_path(config);
    let merged = load_merged_document(&path)?;
    let item = match lookup(&merged, key) {
        Some(it) => it,
        None => {
            eprintln!("proteus: unknown config key '{key}' (try `proteus config keys`)");
            return Ok(exit::CONFIG_ERROR);
        }
    };
    if json {
        super::print_json(&GetReport {
            key,
            value: item_to_json(item),
        })?;
    } else {
        println!("{}", item_to_display(item));
    }
    Ok(exit::SUCCESS)
}

pub fn set(key: &str, value: &str, yes: bool, config: Option<&Path>) -> Result<u8> {
    if !yes {
        eprintln!("proteus: refusing to write config without --yes (safety guard)");
        return Ok(exit::CONFIG_ERROR);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let path = super::config_path(config);
    let mut doc = load_or_empty_document(&path)?;
    let typed = match parse_value_for_key(key, value) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("proteus: cannot set '{key}': {e:#}");
            return Ok(exit::CONFIG_ERROR);
        }
    };
    set_in_doc(&mut doc, key, typed)?;
    let serialized = doc.to_string();
    if let Err(e) = parse_config_text(&serialized) {
        eprintln!("proteus: refusing to write — resulting config would not parse: {e:#}");
        return Ok(exit::CONFIG_ERROR);
    }
    super::write_atomic(&path, serialized.as_bytes())?;
    println!("set {key} = {value} in {}", path.display());
    Ok(exit::SUCCESS)
}

pub fn enable(component: &str, yes: bool, config: Option<&Path>) -> Result<u8> {
    set_enabled(component, true, None, yes, config)
}

pub fn disable(
    component: &str,
    reason: Option<&str>,
    yes: bool,
    config: Option<&Path>,
) -> Result<u8> {
    set_enabled(component, false, reason, yes, config)
}

pub fn edit(config: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let path = super::config_path(config);
    if !path.exists() {
        super::write_atomic(&path, b"")?;
    }
    // Security audit L-4: `$EDITOR` runs as root with the user's HOME
    // when sudo is invoked with `-E` or with `env_keep` rules that
    // preserve HOME. Plugin/autoload files in that HOME (vimrc, init.el,
    // and friends) then run as root and become an arbitrary-code-as-root
    // path from a malicious dotfile. Surface the risk loudly so the
    // operator can choose `sudo -H proteus config edit` (drops HOME) or
    // an inline edit via `proteus config set <key> <value> --yes`.
    if std::env::var_os("HOME").is_some_and(|h| h != *"/root") {
        eprintln!(
            "proteus: warning: $HOME is not /root — your editor's plugins / autoloads will run as root"
        );
        eprintln!(
            "proteus: prefer `sudo -H proteus config edit` (drops HOME) or `proteus config set` for narrow edits"
        );
    }
    let editor = std::env::var_os("VISUAL")
        .or_else(|| std::env::var_os("EDITOR"))
        .unwrap_or_else(|| OsString::from(DEFAULT_EDITOR));
    let status = Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("spawning editor {editor:?}"))?;
    if !status.success() {
        eprintln!("proteus: editor exited with {status}");
        return Ok(exit::GENERIC_ERROR);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    match parse_config_text(&raw) {
        Ok(_) => {
            println!("validated {} OK", path.display());
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: edited config has errors:");
            eprintln!("  {e:#}");
            eprintln!("(file saved as-is; re-run `proteus config edit` to fix)");
            Ok(exit::CONFIG_ERROR)
        }
    }
}

pub fn validate(json: bool, config: Option<&Path>) -> Result<u8> {
    let path = super::config_path(config);
    let mut errors = Vec::new();
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if !raw.is_empty()
        && let Err(e) = parse_config_text(&raw)
    {
        errors.push(format!("{e:#}"));
    }
    let ok = errors.is_empty();
    if json {
        super::print_json(&ValidateReport {
            ok,
            path: path.display().to_string(),
            errors,
        })?;
    } else if ok {
        if raw.is_empty() {
            println!("(empty / missing config — defaults in effect)");
        }
        println!("ok: {}", path.display());
    } else {
        println!("errors in {}:", path.display());
        for e in &errors {
            println!("  {e}");
        }
    }
    if ok {
        Ok(exit::SUCCESS)
    } else {
        Ok(exit::CONFIG_ERROR)
    }
}

pub fn reset(section: Option<&str>, yes: bool, config: Option<&Path>) -> Result<u8> {
    if !yes {
        eprintln!("proteus: refusing to reset config without --yes");
        return Ok(exit::CONFIG_ERROR);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let path = super::config_path(config);
    let defaults_doc = default_document()?;
    match section {
        None => {
            super::write_atomic(&path, defaults_doc.to_string().as_bytes())?;
            println!("reset entire config to defaults: {}", path.display());
        }
        Some(name) => {
            let mut doc = load_or_empty_document(&path)?;
            let default_section = match defaults_doc.get(name) {
                Some(it) => it.clone(),
                None => {
                    eprintln!("proteus: unknown section '{name}' (try `proteus config keys`)");
                    return Ok(exit::CONFIG_ERROR);
                }
            };
            doc[name] = default_section;
            super::write_atomic(&path, doc.to_string().as_bytes())?;
            println!("reset section [{name}] to defaults: {}", path.display());
        }
    }
    Ok(exit::SUCCESS)
}

pub fn keys(json: bool) -> Result<u8> {
    let entries = enumerate_keys()?;
    if json {
        super::print_json(&entries)?;
    } else {
        for e in &entries {
            println!("{} : {} = {}", e.key, e.kind, e.default);
        }
    }
    Ok(exit::SUCCESS)
}

/// `proteus config set-profile <name>`. Writes `profile = "<name>"` at the
/// top of the config file, preserving any per-knob overrides the user has
/// already set (the override-only-if-present model). Switching to `off`
/// keeps overrides on disk; resolution ignores them while `off` is active
/// and restores them as soon as the profile changes back.
pub fn set_profile(name: &str, yes: bool, config: Option<&Path>) -> Result<u8> {
    let profile = match crate::profile::Profile::parse(name) {
        Some(p) => p,
        None => {
            let names: Vec<&str> = crate::profile::Profile::all()
                .iter()
                .map(|p| p.name())
                .collect();
            eprintln!(
                "proteus: unknown profile '{name}' (valid: {})",
                names.join(", ")
            );
            return Ok(exit::CONFIG_ERROR);
        }
    };
    if !yes {
        eprintln!("proteus: refusing to write config without --yes (safety guard)");
        return Ok(exit::CONFIG_ERROR);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let path = super::config_path(config);
    let mut doc = load_or_empty_document(&path)?;
    doc["profile"] = Item::Value(Value::from(profile.name()));
    let serialized = doc.to_string();
    if let Err(e) = parse_config_text(&serialized) {
        eprintln!("proteus: refusing to write — resulting config would not parse: {e:#}");
        return Ok(exit::CONFIG_ERROR);
    }
    super::write_atomic(&path, serialized.as_bytes())?;
    println!(
        "set profile = \"{}\" in {} ({})",
        profile.name(),
        path.display(),
        profile.description()
    );
    Ok(exit::SUCCESS)
}

// ---------- shared helpers ----------

fn set_enabled(
    component: &str,
    enabled: bool,
    reason: Option<&str>,
    yes: bool,
    config: Option<&Path>,
) -> Result<u8> {
    if !yes {
        eprintln!("proteus: refusing to write config without --yes");
        return Ok(exit::CONFIG_ERROR);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !section_has_enabled(component) {
        eprintln!(
            "proteus: component '{component}' has no `enabled` toggle (try `proteus config keys`)"
        );
        return Ok(exit::CONFIG_ERROR);
    }
    let path = super::config_path(config);
    let mut doc = load_or_empty_document(&path)?;
    set_in_doc(
        &mut doc,
        &format!("{component}.enabled"),
        Value::from(enabled),
    )?;
    if !enabled && let Some(text) = reason {
        annotate_disable_reason(&mut doc, component, text);
    }
    super::write_atomic(&path, doc.to_string().as_bytes())?;
    let verb = if enabled { "enabled" } else { "disabled" };
    match reason {
        Some(r) if !enabled => println!("{verb} {component} (reason: {r})"),
        _ => println!("{verb} {component}"),
    }
    Ok(exit::SUCCESS)
}

/// Add a `# Proteus: disabled at <date> — reason: <text>` comment above the
/// `[<component>]` table header. Surfaced in `proteus status`.
fn annotate_disable_reason(doc: &mut DocumentMut, component: &str, reason: &str) {
    let date = super::now_iso8601();
    // Security audit L-2: strip newlines so a multi-line `reason` cannot
    // inject extra TOML keys or comment-out adjacent settings. Replace
    // CR/LF with a single space to keep the comment readable.
    let safe_reason: String = reason
        .chars()
        .map(|c| match c {
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    let comment = format!("# Proteus: disabled at {date} - reason: {safe_reason}\n");
    let Some(item) = doc.get_mut(component) else {
        return;
    };
    if let Some(table) = item.as_table_mut() {
        let prefix = match table.decor().prefix().and_then(|p| p.as_str()) {
            Some(existing) => filter_old_disabled_comments(existing),
            None => String::new(),
        };
        let new_prefix = format!("{prefix}{comment}");
        table.decor_mut().set_prefix(new_prefix);
    }
}

fn filter_old_disabled_comments(prefix: &str) -> String {
    prefix
        .lines()
        .filter(|l| !l.trim_start().starts_with("# Proteus: disabled"))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn load_or_empty_document(path: &Path) -> Result<DocumentMut> {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if raw.is_empty() {
        return Ok(DocumentMut::new());
    }
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}

/// Load the user's config and overlay it onto the defaults so `get` returns
/// a value for every supported key (defaults included).
fn load_merged_document(path: &Path) -> Result<DocumentMut> {
    let mut merged = default_document()?;
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(merged),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if raw.is_empty() {
        return Ok(merged);
    }
    let user: DocumentMut = raw
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    for (k, v) in user.iter() {
        merged.insert(k, v.clone());
    }
    Ok(merged)
}

fn default_document() -> Result<DocumentMut> {
    let s = toml::to_string_pretty(&Config::default()).context("serializing default config")?;
    s.parse::<DocumentMut>()
        .context("re-parsing default config as toml_edit document")
}

fn parse_config_text(s: &str) -> Result<Config> {
    let raw: crate::config::RawConfig = toml::from_str(s).context("parsing config")?;
    Ok(raw.resolve())
}

// ---------- dotted-key plumbing ----------

/// Split a dotted key into (path, field). The path is the dotted run of
/// section / subsection names; the field is the last segment.
///
/// `mac.enabled` -> (`["mac"]`, `"enabled"`).
/// `timers.rotate.interval` -> (`["timers", "rotate"]`, `"interval"`).
///
/// Empty segments (leading or trailing dots, double dots) are rejected.
fn split_key(key: &str) -> Result<(Vec<&str>, &str)> {
    if key.is_empty() {
        return Err(anyhow!("empty key"));
    }
    let parts: Vec<&str> = key.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(anyhow!(
            "key '{key}' must be of the form section.field or section.subsection.field"
        ));
    }
    if parts.len() < 2 {
        return Err(anyhow!(
            "key '{key}' must be of the form section.field or section.subsection.field"
        ));
    }
    let (last, head) = parts.split_last().unwrap();
    Ok((head.to_vec(), last))
}

/// Walk `doc` along `path`, returning the table at the end if it's a table.
fn lookup_table<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a toml_edit::Table> {
    let (head, rest) = path.split_first()?;
    let mut table = doc.get(head)?.as_table()?;
    for seg in rest {
        table = table.get(seg)?.as_table()?;
    }
    Some(table)
}

fn lookup<'a>(doc: &'a DocumentMut, key: &str) -> Option<&'a Item> {
    let (path, field) = split_key(key).ok()?;
    lookup_table(doc, &path)?.get(field)
}

fn section_has_enabled(component: &str) -> bool {
    let Ok(doc) = default_document() else {
        return false;
    };
    doc.get(component)
        .and_then(|t| t.as_table())
        .map(|t| t.contains_key("enabled"))
        .unwrap_or(false)
}

fn set_in_doc(doc: &mut DocumentMut, key: &str, value: Value) -> Result<()> {
    let (path, field) = split_key(key)?;
    let (head, rest) = path
        .split_first()
        .ok_or_else(|| anyhow!("key '{key}' has no section"))?;
    if doc.get(head).is_none() {
        doc[*head] = Item::Table(toml_edit::Table::new());
    }
    let mut table = doc[*head]
        .as_table_mut()
        .ok_or_else(|| anyhow!("section [{head}] is not a table"))?;
    for seg in rest {
        if !table.contains_key(seg) {
            table.insert(seg, Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(seg)
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| anyhow!("section [{seg}] is not a table"))?;
    }
    // Issue #164: warn when overwriting a user-set value with a different
    // TOML type. The default-schema-driven coercion in `parse_value_for_key`
    // is correct for valid config but silently rewrites typo'd hand-edits
    // (e.g. `discoverable = "no"` → `discoverable = false`) without telling
    // the user their original was malformed. Surface it on stderr so the
    // change is visible in shell + journald.
    if let Some(existing) = table.get(field).and_then(|i| i.as_value())
        && value_type_tag(existing) != value_type_tag(&value)
    {
        eprintln!(
            "proteus: warning: replacing {key} (was {old}: {existing}) -> ({new}: {value}) — original value had unexpected type",
            old = value_type_tag(existing),
            new = value_type_tag(&value),
        );
    }
    table[field] = Item::Value(value);
    Ok(())
}

/// Coarse TOML scalar type tag used to detect type-mismatched overwrites
/// in `set_in_doc`. Issue #164.
fn value_type_tag(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Integer(_) => "integer",
        Value::Float(_) => "float",
        Value::Boolean(_) => "bool",
        Value::Datetime(_) => "datetime",
        Value::Array(_) => "array",
        Value::InlineTable(_) => "table",
    }
}

/// Coerce a CLI string into the right TOML value for `key` by consulting the
/// default's existing type. Bools accept true/false/yes/no/on/off (case-insensitive).
fn parse_value_for_key(key: &str, raw: &str) -> Result<Value> {
    let defaults = default_document()?;
    let item = lookup(&defaults, key)
        .ok_or_else(|| anyhow!("unknown config key (try `proteus config keys`)"))?;
    let val = item
        .as_value()
        .ok_or_else(|| anyhow!("'{key}' is not a scalar value"))?;
    match val {
        Value::Boolean(_) => parse_bool(raw).map(Value::from),
        Value::Integer(_) => raw
            .parse::<i64>()
            .map(Value::from)
            .map_err(|e| anyhow!("expected integer: {e}")),
        Value::Float(_) => raw
            .parse::<f64>()
            .map(Value::from)
            .map_err(|e| anyhow!("expected float: {e}")),
        Value::String(_) => Ok(Value::from(raw)),
        Value::Array(_) => {
            // Comma-separated list of strings.
            let mut arr = toml_edit::Array::new();
            for part in raw.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    arr.push(trimmed);
                }
            }
            Ok(Value::Array(arr))
        }
        Value::InlineTable(_) | Value::Datetime(_) => {
            Err(anyhow!("setting this key from the CLI is not supported"))
        }
    }
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        other => Err(anyhow!(
            "expected boolean (true/false/yes/no/on/off), got '{other}'"
        )),
    }
}

fn item_to_display(item: &Item) -> String {
    match item.as_value() {
        Some(Value::String(s)) => s.value().to_string(),
        Some(other) => other.to_string().trim().to_string(),
        None => item.to_string().trim().to_string(),
    }
}

fn item_to_json(item: &Item) -> serde_json::Value {
    let Some(val) = item.as_value() else {
        return serde_json::Value::Null;
    };
    value_to_json(val)
}

fn value_to_json(val: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match val {
        Value::String(s) => J::String(s.value().clone()),
        Value::Integer(i) => J::Number((*i.value()).into()),
        Value::Boolean(b) => J::Bool(*b.value()),
        Value::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Datetime(dt) => J::String(dt.value().to_string()),
        Value::Array(arr) => J::Array(arr.iter().map(value_to_json).collect()),
        Value::InlineTable(tbl) => J::Object(
            tbl.iter()
                .map(|(k, v)| (k.to_string(), value_to_json(v)))
                .collect(),
        ),
    }
}

fn enumerate_keys() -> Result<Vec<KeyEntry>> {
    let doc = default_document()?;
    let mut out = Vec::new();
    for (section_name, item) in doc.iter() {
        let Some(table) = item.as_table() else {
            continue;
        };
        walk_table(&[section_name], table, &mut out);
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// Recurse through a table, emitting one `KeyEntry` per scalar leaf.
/// Nested subsections (e.g. `[timers.rotate]`) are flattened into dotted
/// paths.
fn walk_table(path: &[&str], table: &toml_edit::Table, out: &mut Vec<KeyEntry>) {
    for (field, sub) in table.iter() {
        if let Some(v) = sub.as_value() {
            let key = if path.is_empty() {
                field.to_string()
            } else {
                format!("{}.{field}", path.join("."))
            };
            out.push(KeyEntry {
                key,
                kind: type_name(v),
                default: value_to_json(v),
            });
        } else if let Some(t) = sub.as_table() {
            let mut nested: Vec<&str> = path.to_vec();
            nested.push(field);
            walk_table(&nested, t, out);
        }
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Integer(_) => "integer",
        Value::Float(_) => "float",
        Value::Boolean(_) => "bool",
        Value::Datetime(_) => "datetime",
        Value::Array(_) => "array",
        Value::InlineTable(_) => "table",
    }
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_key_parser_splits_into_path_and_field() {
        let (path, f) = split_key("mac.enabled").unwrap();
        assert_eq!(path, vec!["mac"]);
        assert_eq!(f, "enabled");
        // Three-level keys carry the subsection in the path.
        let (path, f) = split_key("timers.rotate.interval").unwrap();
        assert_eq!(path, vec!["timers", "rotate"]);
        assert_eq!(f, "interval");
        assert!(split_key("mac").is_err());
        assert!(split_key("").is_err());
        assert!(split_key(".x").is_err());
        assert!(split_key("x.").is_err());
        assert!(split_key("x..y").is_err());
    }

    #[test]
    fn set_value_round_trips_through_doc() {
        let mut doc = default_document().unwrap();
        set_in_doc(&mut doc, "mac.enabled", Value::from(true)).unwrap();
        set_in_doc(&mut doc, "mac.rotation_interval", Value::from("30m")).unwrap();
        let serialised = doc.to_string();
        let cfg = parse_config_text(&serialised).unwrap();
        assert!(cfg.mac.enabled);
        assert_eq!(cfg.mac.rotation_interval, "30m");
    }

    #[test]
    fn set_in_doc_handles_type_mismatch_overwrite() {
        // Issue #164: when the user's existing value has a different type
        // (e.g. `mac.enabled = "no"` instead of `false`), the new value
        // still lands but the warning surfaces on stderr. Here we just
        // assert the write itself succeeds and does not regress, since
        // capturing stderr in unit tests is fragile.
        let mut doc = default_document().unwrap();
        let head = doc.as_table_mut().get_mut("mac").unwrap();
        let mac_table = head.as_table_mut().unwrap();
        mac_table["enabled"] = Item::Value(Value::from("no"));
        set_in_doc(&mut doc, "mac.enabled", Value::from(false)).unwrap();
        let serialised = doc.to_string();
        let cfg = parse_config_text(&serialised).unwrap();
        assert!(!cfg.mac.enabled);
    }

    #[test]
    fn value_type_tag_distinguishes_basic_scalars() {
        assert_eq!(value_type_tag(&Value::from(true)), "bool");
        assert_eq!(value_type_tag(&Value::from(7i64)), "integer");
        assert_eq!(value_type_tag(&Value::from("x")), "string");
    }

    #[test]
    fn parse_value_for_key_coerces_by_default_type() {
        let v = parse_value_for_key("mac.enabled", "yes").unwrap();
        assert_eq!(v.as_bool(), Some(true));
        let v = parse_value_for_key("mac.rotation_interval", "1h").unwrap();
        assert_eq!(v.as_str(), Some("1h"));
        let v = parse_value_for_key("probes.quorum_n", "5").unwrap();
        assert_eq!(v.as_integer(), Some(5));
        assert!(parse_value_for_key("mac.enabled", "maybe").is_err());
    }

    #[test]
    fn validate_reports_errors_for_bad_toml() {
        let r = parse_config_text("not = [valid");
        assert!(r.is_err());
        let r = parse_config_text(""); // empty parses fine
        assert!(r.is_ok());
    }

    #[test]
    fn disable_with_reason_writes_comment_above_section() {
        let mut doc = default_document().unwrap();
        set_in_doc(&mut doc, "dns.strip_edns_client_subnet", Value::from(false)).unwrap();
        annotate_disable_reason(&mut doc, "dns", "using dnscrypt-proxy");
        let s = doc.to_string();
        assert!(
            s.contains("# Proteus: disabled at"),
            "expected disable comment in {s}"
        );
        assert!(
            s.contains("using dnscrypt-proxy"),
            "expected reason text in {s}"
        );
        // Re-disabling collapses the old comment instead of stacking.
        annotate_disable_reason(&mut doc, "dns", "second reason");
        let s2 = doc.to_string();
        let count = s2.matches("# Proteus: disabled").count();
        assert_eq!(count, 1, "expected exactly one disable comment, got\n{s2}");
        assert!(s2.contains("second reason"));
        assert!(!s2.contains("using dnscrypt-proxy"));
    }

    #[test]
    fn enumerate_keys_includes_known_fields() {
        let entries = enumerate_keys().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert!(names.contains(&"mac.enabled"));
        assert!(names.contains(&"probes.quorum_n"));
        assert!(names.contains(&"dns.strip_edns_client_subnet"));
        // Three-level [timers.<name>] keys are included via the recursive walker.
        assert!(names.contains(&"timers.rotate.interval"));
        assert!(names.contains(&"timers.check.interval"));
    }

    #[test]
    fn set_three_level_timer_key_round_trips() {
        let mut doc = default_document().unwrap();
        set_in_doc(&mut doc, "timers.rotate.interval", Value::from("1h")).unwrap();
        let serialised = doc.to_string();
        let cfg = parse_config_text(&serialised).unwrap();
        assert_eq!(cfg.timers.rotate.interval, "1h");
    }

    #[test]
    fn section_has_enabled_recognises_mac_but_not_dns() {
        assert!(section_has_enabled("mac"));
        assert!(section_has_enabled("hostname"));
        assert!(!section_has_enabled("dns"));
        assert!(!section_has_enabled("nonexistent"));
    }
}
