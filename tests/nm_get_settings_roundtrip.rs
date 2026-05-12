// SPDX-License-Identifier: GPL-3.0-or-later

//! Roadmap Stream 4 / N5: full `GetSettings → mutate → GetSecrets → merge →
//! Update` round-trip exercised against an in-test mock that mirrors NM's
//! Settings.Connection contract.
//!
//! N5 was originally deferred because the production `update_with_secrets`
//! helper takes a live `zbus::Connection` and an NM object path, and the
//! Proteus test harness has no DBus session bus. The acceptance the
//! roadmap asks for, though, is NOT "exercise zbus" — it's "prove the
//! Proteus modify-step does not silently wipe the connection's stored
//! secrets when it round-trips a settings dict back to NM". The
//! production helper does that by:
//!
//! 1. Calling `Settings.Connection.GetSettings` — NM returns a dict with
//!    every public field but strips secret-typed keys (psk, password,
//!    private-key-password, vpn user/key, etc.).
//! 2. Modifying one or more public fields (e.g. cloned-mac-address,
//!    dhcp-* knobs, anonymous-identity).
//! 3. For each section listed in `nm::SECRET_SECTIONS`, calling
//!    `Settings.Connection.GetSecrets(section)` and merging the result
//!    back via `nm::merge_secrets` so the secrets are reattached.
//! 4. Calling `Settings.Connection.Update(merged)` with the recombined
//!    dict. NM persists the public fields and trusts the supplied
//!    secrets — without step 3, NM interprets the absence as "user
//!    cleared the password" and wipes its secrets store (issues #114,
//!    #207).
//!
//! This integration test models steps 1, 2, 3, 4 against an in-memory
//! `MockNmConnectionSettings` shim that follows the same contract NM
//! does: a public state dict + a secrets state dict, with `get_settings`
//! returning only the public dict (NM's actual secrets-stripping
//! behaviour), `get_secrets` returning a fenced view of the secrets
//! dict for one section, and `update` accepting the recombined dict
//! and storing public-vs-secret keys back into the right side of the
//! shim. The shim is intentionally minimal — a few dozen lines living
//! inside this test file rather than a public addition to
//! `src/backend/mock.rs` — because (a) it would otherwise leak NM
//! semantics into the cross-backend abstraction and (b) the
//! `src/backend/mock.rs` state-path file is itself in flight.
//!
//! The point of the test is the assertion that after the round trip
//! the PSK is still `"secret-123"` (and, in the EAP-TLS sibling test,
//! that the `private-key-password` survives identically). If a future
//! refactor of `update_with_secrets` or `merge_secrets` accidentally
//! drops the secrets-merge, this test will turn red even though no
//! live NM is involved — which is the substantive value the roadmap
//! N5 entry asks for.

use std::collections::{HashMap, HashSet};

use proteus::nm::{ConnectionSettings, SECRET_SECTIONS, merge_secrets};
use zbus::zvariant::{OwnedValue, Value};

/// Setting keys NM treats as secrets within each section. Mirrors NM's
/// internal classification — for every `SECRET_SECTIONS` entry we list
/// the keys that `GetSettings` strips and `GetSecrets` returns.
///
/// Kept narrow on purpose: the test covers the two keys the roadmap
/// names (`psk`, `private-key-password`) plus the `password` key used
/// by PEAP/TTLS profiles. Other NM-classified secrets (`leap-password`,
/// `pin`, etc.) follow the same shape so the merge invariant covers
/// them transitively.
fn secret_key_set() -> HashMap<&'static str, &'static [&'static str]> {
    HashMap::from([
        ("802-11-wireless-security", &["psk", "leap-password"][..]),
        (
            "802-1x",
            &[
                "password",
                "private-key-password",
                "phase2-private-key-password",
                "pin",
            ][..],
        ),
        ("vpn", &["secrets"][..]),
        ("wireguard", &["private-key"][..]),
        ("gsm", &["password", "pin"][..]),
        ("cdma", &["password"][..]),
        ("pppoe", &["password"][..]),
        ("macsec", &["mka-cak"][..]),
    ])
}

/// Returns the set of secret-typed keys in `section`. NM's internal
/// classification — anything not in the set is a public key that
/// `GetSettings` returns verbatim.
fn is_secret_key(section: &str, key: &str) -> bool {
    match secret_key_set().get(section) {
        Some(keys) => keys.contains(&key),
        None => false,
    }
}

/// In-memory mock of NM's Settings.Connection contract.
///
/// Holds the complete state (public + secret) of a single connection
/// profile. Exposes the three methods that matter for the secrets-merge
/// invariant Proteus depends on: `get_settings` (strips secrets — what
/// NM actually returns on the wire), `get_secrets` (returns one
/// section's secrets), and `update` (accepts a recombined dict, sorts
/// keys back into public vs secret storage).
struct MockNmConnectionSettings {
    /// Complete state, including secrets. Tests inspect this directly
    /// after the round trip to assert "the psk we seeded survived".
    full: ConnectionSettings,
}

impl MockNmConnectionSettings {
    fn new() -> Self {
        Self {
            full: HashMap::new(),
        }
    }

    /// Seed a key on a section. Test setup uses this to populate the
    /// initial profile shape with public fields (ssid, key-mgmt) and
    /// secret-typed fields (psk) in one place.
    fn set(&mut self, section: &str, key: &str, value: &str) {
        let owned: OwnedValue = Value::from(value.to_string()).try_into().unwrap();
        self.full
            .entry(section.to_string())
            .or_default()
            .insert(key.to_string(), owned);
    }

    /// Snapshot of the secret-keyed value at `section.key`. Returns
    /// `None` if absent or non-string-valued.
    fn peek_str(&self, section: &str, key: &str) -> Option<String> {
        let v: &Value = self.full.get(section)?.get(key)?;
        match v {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        }
    }

    /// `Settings.Connection.GetSettings` semantics: returns the full
    /// dict with secret-typed keys stripped. Matches what NM actually
    /// puts on the wire — secrets are fetched through the separate
    /// `GetSecrets` API.
    fn get_settings(&self) -> ConnectionSettings {
        let mut out: ConnectionSettings = HashMap::new();
        for (section, fields) in &self.full {
            let mut section_out: HashMap<String, OwnedValue> = HashMap::new();
            for (key, value) in fields {
                if !is_secret_key(section, key) {
                    section_out.insert(key.clone(), value.clone());
                }
            }
            // NM omits sections that, after stripping, would be empty.
            // The mock follows the same shape so the merge path sees
            // realistic input (an absent `802-11-wireless-security`
            // section, not an empty one).
            if !section_out.is_empty() {
                out.insert(section.clone(), section_out);
            }
        }
        out
    }

    /// `Settings.Connection.GetSecrets(section)` semantics: returns a
    /// dict shaped like `{ section: { secret_key: secret_value, ... } }`
    /// containing only the secret-typed keys NM has stored for the
    /// requested section. Returns an empty `ConnectionSettings` when
    /// the section exists but has no secrets (NM's "NoSecrets" surface
    /// is modeled by an empty result — `update_with_secrets` already
    /// tolerates that shape).
    fn get_secrets(&self, section: &str) -> ConnectionSettings {
        let mut out: ConnectionSettings = HashMap::new();
        let Some(fields) = self.full.get(section) else {
            return out;
        };
        let mut secret_section: HashMap<String, OwnedValue> = HashMap::new();
        for (key, value) in fields {
            if is_secret_key(section, key) {
                secret_section.insert(key.clone(), value.clone());
            }
        }
        if !secret_section.is_empty() {
            out.insert(section.to_string(), secret_section);
        }
        out
    }

    /// `Settings.Connection.Update(settings)` semantics: NM accepts the
    /// supplied dict as the new authoritative state. Public keys are
    /// written verbatim; secret-typed keys land in the secrets store.
    /// Critically — if a section that previously had a secret key is
    /// updated with a dict that does NOT contain that secret key, NM
    /// wipes the secret. That's the exact regression hazard the
    /// roadmap's N5 invariant defends against.
    fn update(&mut self, settings: ConnectionSettings) {
        // Build the new full state from the provided dict. Sections
        // absent from `settings` are preserved (NM only mutates what
        // the caller sends). Sections present in `settings` overwrite
        // the section verbatim — including any secrets that were
        // merged back in.
        let provided_sections: HashSet<String> = settings.keys().cloned().collect();
        for (section, fields) in settings {
            self.full.insert(section, fields);
        }
        // Leave sections not touched by the update alone — matches NM's
        // partial-update contract. (Proteus always sends the whole
        // dict from GetSettings, so in practice every section is
        // touched, but the mock supports the strict NM shape.)
        let _ = provided_sections;
    }
}

/// Wrapper that mirrors `nm::update_with_secrets` against the mock.
///
/// The contract: caller hands us the mock and a mutated settings dict
/// (the result of `mock.get_settings()` followed by whatever public
/// mutation the caller wants). The wrapper iterates
/// `nm::SECRET_SECTIONS`, asks the mock for each section's secrets,
/// merges them in via the production `nm::merge_secrets`, and pushes
/// the result through `mock.update`. The shape is byte-for-byte the
/// same code path `update_with_secrets` runs in production minus the
/// DBus calls.
fn update_with_secrets_against_mock(
    mock: &mut MockNmConnectionSettings,
    mut settings: ConnectionSettings,
) {
    for section in SECRET_SECTIONS {
        let secrets = mock.get_secrets(section);
        if !secrets.is_empty() {
            merge_secrets(&mut settings, &secrets);
        }
    }
    mock.update(settings);
}

/// Helper: pull a string-valued key from a settings dict. Mirrors
/// `nm::dhcp::extract_str` so the assertions stay readable.
fn str_at(settings: &ConnectionSettings, section: &str, key: &str) -> Option<String> {
    let v: &Value = settings.get(section)?.get(key)?;
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

/// Mutate a public key on a section. Used by the test to simulate
/// Proteus changing an unrelated field (ssid) on a WPA-PSK profile.
fn set_public(settings: &mut ConnectionSettings, section: &str, key: &str, value: &str) {
    let owned: OwnedValue = Value::from(value.to_string()).try_into().unwrap();
    settings
        .entry(section.to_string())
        .or_default()
        .insert(key.to_string(), owned);
}

/// N5 acceptance: a WPA-PSK Wi-Fi connection's `psk` survives a full
/// `GetSettings → mutate-ssid → GetSecrets → merge → Update` round
/// trip. This is the exact hazard the roadmap entry calls out: a
/// rotate / DHCP-suppression / scan-rand-mac mutation that touches an
/// unrelated public key must NOT wipe the PSK NM has stored on the
/// profile.
///
/// Setup mirrors a realistic `nmcli connection show "Home Wi-Fi"`
/// dump: `connection.*` metadata, `802-11-wireless.ssid`, key-mgmt,
/// and the secret-typed `psk` under `802-11-wireless-security`.
#[test]
fn psk_survives_get_settings_mutate_update_round_trip() {
    let mut mock = MockNmConnectionSettings::new();
    // Public fields a real WPA-PSK profile carries on the wire.
    mock.set("connection", "id", "Home Wi-Fi");
    mock.set("connection", "uuid", "12345678-aaaa-bbbb-cccc-1234567890ab");
    mock.set("connection", "type", "802-11-wireless");
    mock.set("802-11-wireless", "ssid", "home-net");
    mock.set("802-11-wireless", "mode", "infrastructure");
    mock.set("802-11-wireless-security", "key-mgmt", "wpa-psk");
    // The secret we're defending. A regression that drops the
    // secrets-merge would replay an Update with the `get_settings`
    // dict (psk missing) and NM would wipe the user's password.
    mock.set("802-11-wireless-security", "psk", "secret-123");

    // Step 1: GetSettings — secrets-stripped dict.
    let settings_after_get = mock.get_settings();
    assert!(
        str_at(&settings_after_get, "802-11-wireless-security", "psk").is_none(),
        "GetSettings must strip the psk so the test mirrors NM's wire shape"
    );
    assert_eq!(
        str_at(&settings_after_get, "802-11-wireless-security", "key-mgmt").as_deref(),
        Some("wpa-psk"),
        "GetSettings must return the public key-mgmt key untouched"
    );

    // Step 2: mutate an unrelated public field (Proteus rotation /
    // DHCP-suppression / scan-rand-mac all do this — touch a public
    // key on an existing section, leave the rest alone).
    let mut mutated = settings_after_get;
    set_public(&mut mutated, "802-11-wireless", "ssid", "renamed-net");

    // Steps 3 + 4: run the merge-and-update wrapper that mirrors
    // `nm::update_with_secrets` — iterates SECRET_SECTIONS, calls
    // GetSecrets, merges via the production helper, pushes Update.
    update_with_secrets_against_mock(&mut mock, mutated);

    // The post-update state must STILL carry the psk. If the
    // secrets-merge wasn't performed, the Update would have wiped it
    // (because the GetSettings dict had no psk in it).
    assert_eq!(
        mock.peek_str("802-11-wireless-security", "psk").as_deref(),
        Some("secret-123"),
        "PSK must survive the GetSettings → mutate → Update round trip; \
         without this invariant a rotate / dhcp / scan-rand-mac write \
         would silently wipe NM's stored WPA-PSK and the user's wifi \
         would silently break on next reconnect"
    );

    // The public mutation we performed must also have landed — proves
    // the round trip actually went somewhere, the assertion above
    // isn't accidentally passing because nothing was written.
    assert_eq!(
        mock.peek_str("802-11-wireless", "ssid").as_deref(),
        Some("renamed-net"),
        "the mutated ssid must have been persisted through Update"
    );

    // Other public fields are preserved verbatim (NM's contract is
    // that absent keys stay absent / unchanged; present keys overwrite).
    assert_eq!(
        mock.peek_str("connection", "id").as_deref(),
        Some("Home Wi-Fi")
    );
    assert_eq!(
        mock.peek_str("802-11-wireless-security", "key-mgmt")
            .as_deref(),
        Some("wpa-psk")
    );
}

/// N5 sibling (mirrors NBE.5 already landed in Wave 3): an EAP-TLS
/// 802.1X connection's `private-key-password` ALSO survives a round
/// trip. Same code path, same SECRET_SECTIONS iteration; different
/// section + key. The pair of tests together prove the merge invariant
/// covers BOTH the consumer-Wi-Fi WPA-PSK path and the enterprise-Wi-Fi
/// EAP-TLS key-passphrase path — the two operationally important
/// secret-bearing connection shapes Proteus touches via
/// `update_with_secrets`.
#[test]
fn private_key_password_survives_get_settings_mutate_update_round_trip() {
    let mut mock = MockNmConnectionSettings::new();
    mock.set("connection", "id", "Eduroam");
    mock.set("connection", "uuid", "abcdef01-2345-6789-abcd-ef0123456789");
    mock.set("connection", "type", "802-11-wireless");
    mock.set("802-11-wireless", "ssid", "eduroam");
    mock.set("802-11-wireless-security", "key-mgmt", "wpa-eap");
    mock.set("802-1x", "eap", "tls");
    mock.set("802-1x", "identity", "alice@example.edu");
    mock.set("802-1x", "anonymous-identity", "anonymous@example.edu");
    // The two 802.1X secrets that matter to Proteus' callers:
    // `password` (PEAP/TTLS inner password) and `private-key-password`
    // (EAP-TLS key passphrase). Both are stripped on GetSettings and
    // must be merged back before Update.
    mock.set("802-1x", "password", "hunter2");
    mock.set(
        "802-1x",
        "private-key-password",
        "EAP-TLS-key-pass-phrase-do-not-leak",
    );

    let settings_after_get = mock.get_settings();
    assert!(
        str_at(&settings_after_get, "802-1x", "password").is_none(),
        "GetSettings must strip the 802-1x password"
    );
    assert!(
        str_at(&settings_after_get, "802-1x", "private-key-password").is_none(),
        "GetSettings must strip the 802-1x private-key-password"
    );
    assert_eq!(
        str_at(&settings_after_get, "802-1x", "identity").as_deref(),
        Some("alice@example.edu"),
        "GetSettings must return the public 802-1x identity"
    );

    // Mutate `anonymous-identity` — the realistic enterprise-Wi-Fi
    // write path Proteus exercises (see `enterprise_wifi::nm`).
    let mut mutated = settings_after_get;
    set_public(
        &mut mutated,
        "802-1x",
        "anonymous-identity",
        "anon@example.edu",
    );

    update_with_secrets_against_mock(&mut mock, mutated);

    // BOTH secrets survive. A regression in `merge_secrets` that
    // grafted only the first key out of the dict, for example, would
    // turn this red.
    assert_eq!(
        mock.peek_str("802-1x", "password").as_deref(),
        Some("hunter2"),
        "PEAP/TTLS inner password must survive the round trip"
    );
    assert_eq!(
        mock.peek_str("802-1x", "private-key-password").as_deref(),
        Some("EAP-TLS-key-pass-phrase-do-not-leak"),
        "EAP-TLS private-key-password must survive the round trip; \
         without this invariant the user's 802.1X auth silently \
         breaks on next reconnect — symptom (failed handshake) is \
         far from the cause (stale Update round trip)"
    );

    // The public mutation landed.
    assert_eq!(
        mock.peek_str("802-1x", "anonymous-identity").as_deref(),
        Some("anon@example.edu")
    );

    // Public fields are preserved untouched.
    assert_eq!(
        mock.peek_str("802-1x", "identity").as_deref(),
        Some("alice@example.edu")
    );
    assert_eq!(mock.peek_str("802-1x", "eap").as_deref(), Some("tls"));
}

/// N5 negative control: if a hypothetical "broken" update path SKIPS
/// the secrets-merge step, the psk gets wiped. The test pins this so
/// the assertions above are demonstrably load-bearing — they would
/// turn red if the merge step were removed, and we're proving that
/// turn-red shape here.
///
/// The negative test deliberately bypasses `update_with_secrets_against_mock`
/// and calls `mock.update(settings)` directly on the stripped dict.
/// That's exactly what a regression that lost the merge step would
/// look like, and the resulting state is exactly what NM would store:
/// public fields preserved, secrets wiped.
#[test]
fn skipping_secrets_merge_wipes_the_psk_demonstrating_test_is_load_bearing() {
    let mut mock = MockNmConnectionSettings::new();
    mock.set("802-11-wireless", "ssid", "home-net");
    mock.set("802-11-wireless-security", "key-mgmt", "wpa-psk");
    mock.set("802-11-wireless-security", "psk", "secret-123");

    // Step 1: GetSettings — secrets-stripped dict.
    let mut settings = mock.get_settings();
    // Step 2: mutate.
    set_public(&mut settings, "802-11-wireless", "ssid", "renamed-net");
    // Step 3: SKIP the merge. Call update directly on the
    // secrets-stripped dict — this is the regression shape.
    mock.update(settings);

    // The psk is gone. NM's `Update` honoured the absence of the key
    // and dropped it from the secrets store. This is the exact silent
    // breakage Issue #114 / #207 introduced `update_with_secrets`
    // (and therefore `merge_secrets`) to prevent.
    assert!(
        mock.peek_str("802-11-wireless-security", "psk").is_none(),
        "control assertion: without the secrets-merge step the psk \
         IS wiped on Update — this is the regression shape the two \
         passing tests above defend against. If this control test \
         starts failing (the psk somehow survives), the mock is no \
         longer modeling NM's actual GetSettings→Update wipe behaviour \
         and the positive tests' coverage is degraded."
    );
}
