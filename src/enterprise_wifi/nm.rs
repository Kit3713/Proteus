// SPDX-License-Identifier: GPL-3.0-or-later

//! NetworkManager `802-1x.*` connection-settings bridge for the enterprise
//! Wi-Fi anonymous-outer-identity feature.
//!
//! Two operations matter here:
//!
//! * `read_snapshot` — extract `802-1x.identity` and `802-1x.anonymous-identity`
//!   for one connection. The inner identity is what we derive the realm from
//!   in `auto` strategy, and we need both fields to surface a meaningful
//!   `status` line.
//! * `write_anonymous_identity` — push a new value into
//!   `802-1x.anonymous-identity`. Empty string clears the field; NM treats
//!   the empty string as unset on persist.
//!
//! Mirrors `ipv6::nm` deliberately so future 802-1x knobs can land in the
//! same shape.

use anyhow::{Context, Result};
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::nm::{ConnectionProxy, ConnectionSettings};

/// Settings dict key for the 802.1X section.
pub const SECTION: &str = "802-1x";
/// Setting key for the inner (real) identity.
pub const IDENTITY_KEY: &str = "identity";
/// Setting key for the cleartext outer identity. Set to `anonymous@<realm>`
/// when Proteus is enabled for a connection, cleared on disable.
pub const ANONYMOUS_IDENTITY_KEY: &str = "anonymous-identity";

/// Snapshot of the two 802.1X fields Proteus reads. Both `Option`-wrapped
/// because NM omits unset keys from `GetSettings`. A `None` for `identity`
/// usually means the connection isn't 802.1X at all (no `802-1x` section).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EapSnapshot {
    pub identity: Option<String>,
    pub anonymous_identity: Option<String>,
    /// True when the connection has an `802-1x` section at all. Used by
    /// callers to tell "not 802.1X" apart from "802.1X with empty identity".
    pub has_eap_section: bool,
}

impl EapSnapshot {
    /// Extract the snapshot from a `GetSettings` result.
    pub fn from_settings(settings: &ConnectionSettings) -> Self {
        let mut out = Self::default();
        let Some(section) = settings.get(SECTION) else {
            return out;
        };
        out.has_eap_section = true;
        out.identity = lookup_str(section, IDENTITY_KEY);
        out.anonymous_identity = lookup_str(section, ANONYMOUS_IDENTITY_KEY);
        out
    }
}

/// Read the 802.1X-flavored fields off a connection profile.
pub async fn read_snapshot(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
) -> Result<EapSnapshot> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let settings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    Ok(EapSnapshot::from_settings(&settings))
}

/// Write `802-1x.anonymous-identity = <value>` on the supplied connection.
/// An empty string clears the field — that's NM's documented contract for
/// "unset" on save (the next `GetSettings` will return the key absent).
///
/// Every other field on the profile, including the inner identity, EAP
/// method, certificate paths, and domain-suffix-match, is preserved verbatim.
///
/// `GetSettings` does NOT return secret-typed keys (passwords, PSKs, private
/// key passphrases, etc.); calling `Update` with a secrets-stripped dict
/// would overwrite NM's secrets store and break the user's auth. We pull
/// the `802-1x` secrets via `GetSecrets` first and merge them back into the
/// settings dict before pushing the update — see issue #114.
pub async fn write_anonymous_identity(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    new_value: &str,
) -> Result<()> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let mut settings: ConnectionSettings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    let entry = settings.entry(SECTION.to_string()).or_default();
    entry.insert(
        ANONYMOUS_IDENTITY_KEY.to_string(),
        Value::from(new_value.to_string()).try_into()?,
    );
    // Pull the per-section secrets dict and graft it back onto the settings
    // dict so passwords/cert-passphrases survive the `Update` round trip. If
    // the connection has no secrets stored, NM returns an empty section and
    // `merge_secrets` is a no-op.
    let secrets: ConnectionSettings = proxy
        .get_secrets(SECTION)
        .await
        .context("calling Settings.Connection.GetSecrets(\"802-1x\")")?;
    merge_secrets(&mut settings, &secrets);
    proxy
        .update(settings)
        .await
        .context("calling Settings.Connection.Update")?;
    Ok(())
}

/// Merge a `GetSecrets` result into a settings dict in place.
///
/// NM's `GetSecrets(setting_name)` returns a dict shaped like
/// `{ "802-1x": { "password": ..., "private-key-password": ... } }` — i.e.
/// only the secret-typed keys, keyed by section. We graft each section's
/// secrets onto the matching section in `settings`, preserving any settings
/// already in place (so the caller's freshly-modified `anonymous-identity`
/// survives) and overwriting only on key collisions inside a section.
///
/// This is the merge step that fixes issue #114: without it, `Update` would
/// be called with a dict that lacks the secret keys NM already has stored,
/// and NM would interpret that as "the user removed their password".
pub fn merge_secrets(settings: &mut ConnectionSettings, secrets: &ConnectionSettings) {
    for (section_name, section_secrets) in secrets {
        let target = settings.entry(section_name.clone()).or_default();
        for (key, value) in section_secrets {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn lookup_str(
    section: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<String> {
    let v: &Value = section.get(key)?;
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_str(s: &str) -> zbus::zvariant::OwnedValue {
        Value::from(s.to_string()).try_into().unwrap()
    }

    /// Read a string-valued key from the settings dict — the test helper
    /// equivalent of `lookup_str` so the assertions stay readable.
    fn str_at(settings: &ConnectionSettings, section: &str, key: &str) -> Option<String> {
        let v: &Value = settings.get(section)?.get(key)?;
        match v {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        }
    }

    #[test]
    fn snapshot_from_empty_settings_has_no_eap_section() {
        let s = EapSnapshot::from_settings(&ConnectionSettings::new());
        assert_eq!(s, EapSnapshot::default());
        assert!(!s.has_eap_section);
        assert!(s.identity.is_none());
        assert!(s.anonymous_identity.is_none());
    }

    #[test]
    fn merge_secrets_grafts_passwords_into_existing_section() {
        // Issue #114: simulate the read-modify-write cycle where the caller
        // has already updated `anonymous-identity` in the settings dict, then
        // GetSecrets returns the (separately stored) `password`. After merge,
        // both must be present so `Update` doesn't wipe the secrets store.
        let mut settings = ConnectionSettings::new();
        let section = settings.entry(SECTION.to_string()).or_default();
        section.insert("eap".to_string(), owned_str("peap"));
        section.insert("identity".to_string(), owned_str("alice@example.edu"));
        section.insert(
            ANONYMOUS_IDENTITY_KEY.to_string(),
            owned_str("anonymous@example.edu"),
        );

        let mut secrets = ConnectionSettings::new();
        secrets
            .entry(SECTION.to_string())
            .or_default()
            .insert("password".to_string(), owned_str("hunter2"));

        merge_secrets(&mut settings, &secrets);

        assert_eq!(
            str_at(&settings, SECTION, ANONYMOUS_IDENTITY_KEY).as_deref(),
            Some("anonymous@example.edu"),
        );
        assert_eq!(
            str_at(&settings, SECTION, "identity").as_deref(),
            Some("alice@example.edu"),
        );
        assert_eq!(
            str_at(&settings, SECTION, "password").as_deref(),
            Some("hunter2"),
        );
    }

    #[test]
    fn merge_secrets_creates_section_when_missing() {
        let mut settings = ConnectionSettings::new();
        let mut secrets = ConnectionSettings::new();
        secrets
            .entry(SECTION.to_string())
            .or_default()
            .insert("password".to_string(), owned_str("hunter2"));

        merge_secrets(&mut settings, &secrets);

        assert_eq!(
            str_at(&settings, SECTION, "password").as_deref(),
            Some("hunter2"),
        );
    }

    #[test]
    fn merge_secrets_overwrites_on_key_collision() {
        // Defensive: GetSettings strips secrets, so the settings dict
        // shouldn't have stale secret values; if it does, the GetSecrets
        // value wins because NM's secrets store is the source of truth.
        let mut settings = ConnectionSettings::new();
        settings
            .entry(SECTION.to_string())
            .or_default()
            .insert("password".to_string(), owned_str("stale"));

        let mut secrets = ConnectionSettings::new();
        secrets
            .entry(SECTION.to_string())
            .or_default()
            .insert("password".to_string(), owned_str("fresh"));

        merge_secrets(&mut settings, &secrets);

        assert_eq!(
            str_at(&settings, SECTION, "password").as_deref(),
            Some("fresh"),
        );
    }

    #[test]
    fn merge_secrets_with_empty_input_is_noop() {
        let mut settings = ConnectionSettings::new();
        settings
            .entry(SECTION.to_string())
            .or_default()
            .insert("identity".to_string(), owned_str("alice@example.edu"));

        merge_secrets(&mut settings, &ConnectionSettings::new());

        assert_eq!(
            str_at(&settings, SECTION, "identity").as_deref(),
            Some("alice@example.edu"),
        );
        assert_eq!(settings.get(SECTION).map(|s| s.len()), Some(1));
    }
}
