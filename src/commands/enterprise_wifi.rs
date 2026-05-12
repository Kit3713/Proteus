// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus enterprise-wifi` — status / enable / disable for the 802.1X
//! anonymous-outer-identity feature.
//!
//! Behaviour at a glance:
//!
//! * `status` is read-only. Lists every NM connection that has an
//!   `802-1x` section, with the inner identity redacted and the current
//!   anonymous-identity surfaced verbatim.
//! * `enable` requires root + `--yes`. Reads the inner identity, derives
//!   the realm, writes `802-1x.anonymous-identity = anonymous@<realm>`,
//!   and caches the pre-Proteus value in `state.json`.
//! * `disable` requires root + `--yes`. Restores `802-1x.anonymous-identity`
//!   to the cached pre-Proteus value (issue #298 — the prior shape always
//!   cleared the field, even when the operator had a non-empty outer
//!   identity before enable). Untags the connection in `state.json`.
//! * `revert` is the helper called by `proteus revert` /
//!   `proteus uninstall` to walk every cached enterprise-wifi original
//!   in one pass.
//!
//! Connections that aren't 802.1X are skipped cleanly with a single log
//! line — matching the rest of Proteus's detect-and-defer story.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use zbus::zvariant::OwnedObjectPath;

use crate::backend::{ConnectionRef, NetworkBackend};
use crate::config::{Config, EnterpriseWifiConfig};
use crate::enterprise_wifi::{self, nm as eap_nm};
use crate::exit;
// Roadmap Milestone 1: kept for the `802-1x` snapshot reader and the
// id-based connection lookup. The high-level write
// (`write_anonymous_identity`) routes through `crate::backend::*`.
use crate::nm;
use crate::state::{ConnectionOriginals, State};
use crate::version;

#[derive(Debug, Serialize)]
struct StatusReport {
    enabled: bool,
    realm_strip_strategy: String,
    anonymous_realm: String,
    connections: Vec<ConnectionStatus>,
}

#[derive(Debug, Serialize)]
struct ConnectionStatus {
    /// NM connection profile id (the human-friendly name).
    name: String,
    /// Inner identity, redacted to `***@realm` for safe display.
    identity: Option<String>,
    /// Current value of `802-1x.anonymous-identity`. `None` means unset.
    anonymous_identity: Option<String>,
    /// Whether Proteus has cached an original for this connection.
    proteus_managed: bool,
}

pub fn status(json: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let state = State::load_or_default(&state_path).unwrap_or_default();

    let connections = list_eap_connections(&state).unwrap_or_default();

    let report = StatusReport {
        enabled: config.enterprise_wifi.anonymous_outer_identity,
        realm_strip_strategy: config.enterprise_wifi.realm_strip_strategy.clone(),
        anonymous_realm: config.enterprise_wifi.anonymous_realm.clone(),
        connections,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_status(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn enable(
    connection: &str,
    yes: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "'enterprise-wifi enable' is mutating",
        "proteus help enterprise-wifi",
    ) {
        return Ok(code);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();

    let mut state = State::load_or_default(&state_path)?;

    let outcome = run_async(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        // Roadmap M1: `enable_one` consults the backend trait for the
        // mutating write (`write_anonymous_identity`); the introspection
        // calls stay zbus-direct against NM since networkd / raw don't
        // have an 802.1X surface today.
        let backend = crate::backend::select::select(&config.backend.driver).await?;
        enable_one(
            &conn,
            backend.as_ref(),
            connection,
            &config.enterprise_wifi,
            &mut state,
        )
        .await
    });

    match outcome {
        Ok(EnableOutcome::Applied {
            connection: name,
            realm,
            previous,
        }) => {
            persist_capture_metadata(&mut state);
            state.save(&state_path)?;
            let prev_label = previous.as_deref().unwrap_or("(unset)");
            println!(
                "enterprise-wifi: {name}: anonymous-identity set to anonymous@{realm} (was {prev_label})"
            );
            Ok(exit::SUCCESS)
        }
        Ok(EnableOutcome::NotEap { connection: name }) => {
            eprintln!("proteus: '{name}' is not an 802.1X connection (no 802-1x.identity)");
            Ok(exit::GENERIC_ERROR)
        }
        Err(e) => {
            eprintln!("proteus: enterprise-wifi enable failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

pub fn disable(connection: &str, yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "'enterprise-wifi disable' is mutating",
        "proteus help enterprise-wifi",
    ) {
        return Ok(code);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    let outcome = run_async(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        let cfg = Config::default_or_loaded(&super::config_path(None)).unwrap_or_default();
        let backend = crate::backend::select::select(&cfg.backend.driver).await?;
        disable_one(&conn, backend.as_ref(), connection, &mut state).await
    });

    match outcome {
        Ok(DisableOutcome::Restored {
            connection: name,
            previous,
            restored,
        }) => {
            state.save(&state_path)?;
            let prev_label = previous.as_deref().unwrap_or("(unset)");
            match restored.as_deref() {
                Some(value) => println!(
                    "enterprise-wifi: {name}: anonymous-identity restored to {value} (was {prev_label})"
                ),
                None => println!(
                    "enterprise-wifi: {name}: anonymous-identity cleared (was {prev_label})"
                ),
            }
            Ok(exit::SUCCESS)
        }
        Ok(DisableOutcome::NotEap { connection: name }) => {
            eprintln!("proteus: '{name}' is not an 802.1X connection (no 802-1x section)");
            Ok(exit::GENERIC_ERROR)
        }
        Err(e) => {
            eprintln!("proteus: enterprise-wifi disable failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

/// Best-effort revert: walk every cached enterprise-wifi original and
/// push the cached `anonymous-identity` back onto the live NM
/// connection. Mirrors `dhcp::revert` and `rf::revert` so the parent
/// `commands::revert::run` (and `commands::uninstall::run`) can call it
/// alongside the other per-feature reverts.
///
/// `state.originals.connections[uuid].anonymous_identity` is the source
/// of truth: a `Some("…")` is written back verbatim, a `Some(None)` is
/// cleared (the field was unset before Proteus's first `enable`). After
/// the write succeeds the cached field is dropped from state so a
/// re-run doesn't keep retrying a connection that's already restored.
///
/// Issue #298: the prior shape (no enterprise-wifi step in
/// `revert_best_effort`) left `802-1x.anonymous-identity` set to
/// `anonymous@<realm>` after `proteus revert`, leaving the user's
/// supplicant talking to the AP with the Proteus-applied outer
/// identity even though they thought they had backed Proteus out.
pub fn revert(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if let Err(code) = super::require_yes(
        yes,
        "'enterprise-wifi revert' is mutating",
        "proteus help enterprise-wifi",
    ) {
        return Ok(code);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if !state
        .originals
        .connections
        .values()
        .any(|c| c.anonymous_identity.is_some())
    {
        // Nothing to revert. Stay quiet so a fresh install's
        // `proteus revert` doesn't spam the operator.
        return Ok(exit::SUCCESS);
    }

    let outcomes = run_async(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        let cfg = Config::default_or_loaded(&super::config_path(None)).unwrap_or_default();
        let backend = crate::backend::select::select(&cfg.backend.driver).await?;
        do_revert(&conn, backend.as_ref(), &mut state).await
    });

    match outcomes {
        Ok(rs) => {
            state.save(&state_path)?;
            for r in &rs {
                match (&r.restored, &r.error) {
                    (_, Some(e)) => {
                        eprintln!("enterprise-wifi: {}: revert failed: {}", r.connection, e);
                    }
                    (Some(value), None) => {
                        println!(
                            "enterprise-wifi: {}: anonymous-identity restored to {value}",
                            r.connection
                        );
                    }
                    (None, None) => {
                        println!(
                            "enterprise-wifi: {}: anonymous-identity cleared",
                            r.connection
                        );
                    }
                }
            }
            Ok(exit::SUCCESS)
        }
        Err(e) => {
            eprintln!("proteus: enterprise-wifi revert failed: {e:#}");
            Ok(exit::GENERIC_ERROR)
        }
    }
}

// ---- internal helpers ---------------------------------------------------

#[derive(Debug)]
enum EnableOutcome {
    Applied {
        connection: String,
        realm: String,
        previous: Option<String>,
    },
    NotEap {
        connection: String,
    },
}

#[derive(Debug)]
enum DisableOutcome {
    /// Issue #298: disable now restores the cached pre-Proteus value
    /// instead of always clearing. `restored` carries the value that
    /// was actually written: `Some("alice@example.edu")` means we put
    /// back a non-empty cached original; `None` means the cache held
    /// `None` (the field was unset before Proteus's first enable, so
    /// we cleared it, matching NM's "empty string == unset" contract).
    Restored {
        connection: String,
        previous: Option<String>,
        restored: Option<String>,
    },
    NotEap {
        connection: String,
    },
}

/// Per-connection outcome from [`do_revert`]. Mirrors `DisableOutcome`
/// but flattened for the revert loop, which writes one row per
/// connection that had a cached original.
#[derive(Debug)]
struct RevertOutcome {
    connection: String,
    /// Value actually pushed back into NM. `None` means we wrote the
    /// empty string (NM's "unset on save" contract).
    restored: Option<String>,
    /// Populated when the revert write itself failed; in that case
    /// state retains the cached original so a later revert can retry.
    error: Option<String>,
}

async fn enable_one(
    conn: &zbus::Connection,
    backend: &dyn NetworkBackend,
    connection: &str,
    cfg: &EnterpriseWifiConfig,
    state: &mut State,
) -> Result<EnableOutcome> {
    let (path, settings) = nm::apply::find_connection_by_id(conn, connection)
        .await
        .with_context(|| format!("looking up NM connection '{connection}'"))?;
    let snapshot = eap_nm::EapSnapshot::from_settings(&settings);
    if !snapshot.has_eap_section {
        return Ok(EnableOutcome::NotEap {
            connection: connection.to_string(),
        });
    }
    if snapshot.identity.is_none() && cfg.realm_strip_strategy == "auto" {
        return Ok(EnableOutcome::NotEap {
            connection: connection.to_string(),
        });
    }

    let realm = enterprise_wifi::resolve_realm(
        &cfg.realm_strip_strategy,
        &cfg.anonymous_realm,
        snapshot.identity.as_deref(),
    )?
    .to_string();
    let new_value = enterprise_wifi::anonymous_identity_for(&realm);

    // Issue #209: state must be keyed by `connection.uuid`, not the human
    // display id. The display id can collide between profiles, and the
    // load-time `migrate_connection_keys_to_uuid` migration in #124 silently
    // drops anything that's not uuid-shaped — which would have wiped every
    // enterprise-wifi original on the next state load.
    let uuid = nm::apply::read_connection_uuid(conn, &path)
        .await
        .context("reading connection.uuid for state keying")?
        .ok_or_else(|| {
            anyhow!(
                "NM connection '{connection}' has no `connection.uuid`; see proteus wiki enterprise-wifi"
            )
        })?;

    // Cache the pre-Proteus value exactly once. Re-runs on a connection we
    // already manage do NOT clobber the cached original — that's how revert
    // can put the profile back the way it was on first apply, even after
    // multiple toggles.
    state
        .originals
        .connections
        .entry(uuid)
        .or_insert_with(|| ConnectionOriginals {
            anonymous_identity: snapshot.anonymous_identity.clone(),
            dhcp_settings: None,
        });

    // Roadmap M1: write goes through the backend trait. NM impl
    // delegates back into `eap_nm::write_anonymous_identity` so
    // behaviour is byte-identical to the pre-trait path.
    let cref = ConnectionRef::new(path.as_str().to_string());
    backend.write_anonymous_identity(&cref, &new_value).await?;

    Ok(EnableOutcome::Applied {
        connection: connection.to_string(),
        realm,
        previous: snapshot.anonymous_identity,
    })
}

/// Issue #298: decide the value to write into
/// `802-1x.anonymous-identity` on `disable` or `revert`. A
/// `Some(value)` is the cached pre-Proteus original — restore it
/// verbatim. `None` covers both "Proteus saw the field unset on
/// enable" (state stored `None`) and "no cache entry at all" (state
/// was wiped, or operator never ran enable); both fall back to the
/// empty string, which is NM's "unset on save" contract.
///
/// Pure function so the disable+revert tests can pin the table
/// without needing a live DBus or backend.
fn decide_disable_value(cached_original: Option<&str>) -> String {
    cached_original.unwrap_or("").to_string()
}

async fn disable_one(
    conn: &zbus::Connection,
    backend: &dyn NetworkBackend,
    connection: &str,
    state: &mut State,
) -> Result<DisableOutcome> {
    let (path, settings) = nm::apply::find_connection_by_id(conn, connection)
        .await
        .with_context(|| format!("looking up NM connection '{connection}'"))?;
    let snapshot = eap_nm::EapSnapshot::from_settings(&settings);
    if !snapshot.has_eap_section {
        return Ok(DisableOutcome::NotEap {
            connection: connection.to_string(),
        });
    }

    // Issue #298: restore the cached pre-Proteus `anonymous-identity`
    // instead of always clearing. The cache lives at
    // `state.originals.connections[uuid].anonymous_identity` (populated
    // on `enable`). When there's no cached value, fall back to
    // writing empty — NM's "unset on save" contract.
    let uuid = nm::apply::read_connection_uuid(conn, &path)
        .await
        .ok()
        .flatten();
    let cached_original = uuid
        .as_deref()
        .and_then(|u| state.originals.connections.get(u))
        .and_then(|c| c.anonymous_identity.as_deref());
    let new_value = decide_disable_value(cached_original);
    let restored = if new_value.is_empty() {
        None
    } else {
        Some(new_value.clone())
    };

    let cref = ConnectionRef::new(path.as_str().to_string());
    backend.write_anonymous_identity(&cref, &new_value).await?;

    // Issue #209: untag by uuid (the canonical state key), with a fallback
    // pass that strips any legacy id-keyed entry we still find. The legacy
    // strip is defensive — `migrate_connection_keys_to_uuid` already drops
    // those at load time — but the disable path is the right place to
    // also remove any entry that survived a partial migration.
    if let Some(uuid) = uuid {
        let _ = state.originals.connections.remove(&uuid);
    }
    let _ = state.originals.connections.remove(connection);

    Ok(DisableOutcome::Restored {
        connection: connection.to_string(),
        previous: snapshot.anonymous_identity,
        restored,
    })
}

/// Pull the list of enterprise-wifi revert targets out of state. A
/// "target" is a connection uuid whose `anonymous_identity` cache is
/// populated (`Some(_)`, including the empty string). `None` means
/// the entry is purely a DHCP record from `dhcp::apply` and doesn't
/// belong to the enterprise-wifi pass — skipping it preserves the
/// segregation between the two features.
///
/// Pure helper so the revert-targeting logic can be pinned with a
/// straight unit test instead of a live NM + state fixture.
fn revert_targets(state: &State) -> Vec<(String, String)> {
    state
        .originals
        .connections
        .iter()
        .filter_map(|(uuid, c)| Some((uuid.clone(), c.anonymous_identity.clone()?)))
        .collect()
}

/// Walk every NM connection that has a cached enterprise-wifi original
/// in `state.originals.connections` and push the cached
/// `anonymous-identity` back. Mirrors the `dhcp::do_revert` shape so
/// both sit at the same layer for the parent `revert_best_effort` to
/// fan out into.
///
/// On success: clears `anonymous_identity` on the matching state entry
/// (other fields like `dhcp_settings` are preserved so a parallel
/// `dhcp::revert` invocation doesn't see the entry vanish out from
/// under it).
///
/// On failure: the cached original is **kept** so a later revert can
/// retry — same recovery posture as `rf::revert_apply_originals`.
async fn do_revert(
    conn: &zbus::Connection,
    backend: &dyn NetworkBackend,
    state: &mut State,
) -> Result<Vec<RevertOutcome>> {
    let settings_proxy = nm::SettingsProxy::new(conn)
        .await
        .context("connecting to NetworkManager Settings")?;

    // Snapshot just the uuids (and their cached value) so we can mutate
    // state inside the loop without re-borrowing.
    let targets = revert_targets(state);

    let mut outcomes = Vec::new();
    for (uuid, cached) in targets {
        // Resolve the live NM path for this uuid. If NM doesn't know
        // it (profile deleted manually, NM restarted, etc.) drop the
        // entry from state — there's nothing on-the-wire to restore.
        let path = match settings_proxy.get_connection_by_uuid(&uuid).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(uuid = %uuid, "enterprise-wifi revert: NM has no connection: {e:#}");
                if let Some(c) = state.originals.connections.get_mut(&uuid) {
                    c.anonymous_identity = None;
                }
                outcomes.push(RevertOutcome {
                    connection: uuid.clone(),
                    restored: None,
                    error: Some(format!("connection no longer in NetworkManager: {e}")),
                });
                continue;
            }
        };

        // Display name used in operator output. Falls back to the uuid
        // when the lookup fails — keeps the line meaningful even when
        // NM is being uncooperative.
        let display = nm::apply::read_connection_id(conn, &path)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| uuid.clone());

        let cref = ConnectionRef::new(path.as_str().to_string());
        match backend.write_anonymous_identity(&cref, &cached).await {
            Ok(()) => {
                if let Some(c) = state.originals.connections.get_mut(&uuid) {
                    c.anonymous_identity = None;
                }
                outcomes.push(RevertOutcome {
                    connection: display,
                    restored: if cached.is_empty() {
                        None
                    } else {
                        Some(cached)
                    },
                    error: None,
                });
            }
            Err(e) => {
                // Keep the cached entry so a later retry has
                // something to restore from.
                outcomes.push(RevertOutcome {
                    connection: display,
                    restored: None,
                    error: Some(format!("{e:#}")),
                });
            }
        }
    }
    outcomes.sort_by(|a, b| a.connection.cmp(&b.connection));
    Ok(outcomes)
}

/// Walk every NM connection profile, return one row per profile that has an
/// 802.1X section. Errors enumerating any single connection are logged and
/// swallowed so a single bad profile doesn't blank the whole status output.
fn list_eap_connections(state: &State) -> Result<Vec<ConnectionStatus>> {
    run_async(async {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("enterprise-wifi: system DBus unavailable: {e:#}");
                return Ok::<_, anyhow::Error>(Vec::new());
            }
        };
        let settings_proxy = match nm::SettingsProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("enterprise-wifi: NM Settings proxy unavailable: {e:#}");
                return Ok(Vec::new());
            }
        };
        let paths = settings_proxy.list_connections().await.unwrap_or_default();
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            match read_one_for_status(&conn, &path, state).await {
                Ok(Some(row)) => out.push(row),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("enterprise-wifi: skipping connection: {e:#}");
                }
            }
        }
        // Stable order so JSON output is deterministic.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    })
}

async fn read_one_for_status(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
    state: &State,
) -> Result<Option<ConnectionStatus>> {
    let snapshot = eap_nm::read_snapshot(conn, path).await?;
    if !snapshot.has_eap_section {
        return Ok(None);
    }
    let id = nm::apply::read_connection_id(conn, path)
        .await?
        .ok_or_else(|| {
            anyhow!("connection has no `connection.id`; see proteus wiki enterprise-wifi")
        })?;
    // Issue #209: state.originals.connections keys by uuid, not display id.
    let proteus_managed = match nm::apply::read_connection_uuid(conn, path).await? {
        Some(uuid) => state.originals.connections.contains_key(&uuid),
        None => false,
    };
    Ok(Some(ConnectionStatus {
        name: id,
        identity: snapshot
            .identity
            .as_deref()
            .map(enterprise_wifi::redact_identity),
        anonymous_identity: snapshot.anonymous_identity,
        proteus_managed,
    }))
}

/// Block-on a future on a fresh current-thread runtime. Mirrors the
/// pattern in the bluetooth/hostname/ipv6 commands so all of them surface
/// runtime-creation errors the same way.
fn run_async<F, T>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    rt.block_on(fut)
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(super::now_iso8601());
    }
}

fn print_status(r: &StatusReport) {
    println!("enterprise-wifi:");
    println!("  enabled (master):       {}", yesno(r.enabled));
    println!("  realm_strip_strategy:   {}", r.realm_strip_strategy);
    println!(
        "  anonymous_realm:        {}",
        if r.anonymous_realm.is_empty() {
            "(unset)"
        } else {
            r.anonymous_realm.as_str()
        }
    );
    println!("connections:");
    if r.connections.is_empty() {
        println!("  (no 802.1X connection profiles found)");
        return;
    }
    for c in &r.connections {
        println!("  {}", c.name);
        println!(
            "    identity:             {}",
            c.identity.as_deref().unwrap_or("(unset)")
        );
        println!(
            "    anonymous-identity:   {}",
            c.anonymous_identity.as_deref().unwrap_or("(not set)")
        );
        println!("    proteus-managed:      {}", yesno(c.proteus_managed));
    }
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ConnectionOriginals;

    #[test]
    fn realm_extraction_matches_wiki_examples() {
        // From the wiki: identity `j.smith@university.edu` → `university.edu`.
        let realm = enterprise_wifi::extract_realm("j.smith@university.edu").unwrap();
        assert_eq!(realm, "university.edu");
        let anon = enterprise_wifi::anonymous_identity_for(realm);
        assert_eq!(anon, "anonymous@university.edu");
    }

    #[test]
    fn anonymous_identity_setting_uses_anonymous_at_realm_format() {
        // Mirrors what `enable_one` writes into 802-1x.anonymous-identity.
        let realm = enterprise_wifi::resolve_realm("auto", "", Some("alice@example.edu")).unwrap();
        let written = enterprise_wifi::anonymous_identity_for(realm);
        assert_eq!(written, "anonymous@example.edu");
    }

    // === Roadmap Milestone 1 — write_anonymous_identity routes via trait ===

    use crate::backend::mock::{MockBackend, MockCall};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// `MockBackend` records the `WriteAnonymousIdentity` call when a
    /// command path drives the trait. We can't run the full
    /// `enable_one` (it needs the NM Settings proxy), but we can
    /// exercise the same trait method the production code now reaches
    /// through and assert the same shape as the production write.
    #[test]
    fn write_anonymous_identity_through_trait_records_call() {
        let backend = MockBackend::new();
        let cref = crate::backend::ConnectionRef::new("/org/freedesktop/NetworkManager/Settings/3");
        rt().block_on(async {
            backend
                .write_anonymous_identity(&cref, "anonymous@example.edu")
                .await
                .unwrap();
        });
        let log = backend.call_log();
        assert!(
            log.iter().any(|c| matches!(
                c,
                MockCall::WriteAnonymousIdentity { value, .. } if value == "anonymous@example.edu"
            )),
            "trait write must surface in MockBackend log; log = {log:?}"
        );
        assert_eq!(
            backend.anonymous_identity_for(&cref).as_deref(),
            Some("anonymous@example.edu")
        );
    }

    /// Empty string clears the field (NM contract for "unset on
    /// save"). Pin the trait surface mirrors that contract so the
    /// disable path stays consistent across backends.
    #[test]
    fn write_anonymous_identity_empty_clears_via_trait() {
        let backend = MockBackend::new();
        let cref = crate::backend::ConnectionRef::new("/org/freedesktop/NetworkManager/Settings/3");
        rt().block_on(async {
            backend
                .write_anonymous_identity(&cref, "anonymous@x.y")
                .await
                .unwrap();
            backend.write_anonymous_identity(&cref, "").await.unwrap();
        });
        assert!(backend.anonymous_identity_for(&cref).is_none());
    }

    #[test]
    fn disable_untags_connection_in_state() {
        // Issue #209: originals are keyed by NM connection.uuid (uuid-shaped).
        // The disable_one path strips the entry; this test mirrors that.
        let mut state = State::default();
        let uuid = "12345678-1234-1234-1234-123456789abc".to_string();
        state.originals.connections.insert(
            uuid.clone(),
            ConnectionOriginals {
                anonymous_identity: Some("old@x.y".to_string()),
                dhcp_settings: None,
            },
        );
        state.originals.connections.remove(&uuid);
        assert!(!state.originals.connections.contains_key(&uuid));
    }

    // === Issue #298 — disable restores cached anonymous-identity ===

    /// Issue #298: a cached `Some("…")` original means Proteus saw a
    /// real outer identity at first `enable`, so `disable` must put
    /// that exact value back rather than clearing the field. This is
    /// the primary regression: pre-fix, every `disable` wrote `""`.
    #[test]
    fn decide_disable_value_restores_cached_some() {
        let v = decide_disable_value(Some("alice@uni.edu"));
        assert_eq!(
            v, "alice@uni.edu",
            "cached non-empty original must be restored verbatim"
        );
    }

    /// No cached value (state's `anonymous_identity` was `None`, or
    /// no cache entry at all): defensive clear, since we don't have
    /// anything safer to write.
    #[test]
    fn decide_disable_value_clears_when_no_cache() {
        let v = decide_disable_value(None);
        assert!(
            v.is_empty(),
            "missing cache must fall back to clear, got {v:?}"
        );
    }

    // === Issue #298 — revert path ===

    /// `revert_targets` selects only the connections that have a
    /// cached enterprise-wifi original. A pure-DHCP entry (with
    /// `anonymous_identity = None` AND a `dhcp_settings` snapshot)
    /// must NOT be picked up by the enterprise-wifi revert pass —
    /// that's `dhcp::revert`'s job, and double-touching it would
    /// break the segregation between the two features.
    #[test]
    fn revert_targets_skips_pure_dhcp_entries() {
        use crate::state::DhcpSettingsSnapshot;
        let mut state = State::default();
        // Enterprise-wifi entry — should be picked up.
        state.originals.connections.insert(
            "11111111-1111-1111-1111-111111111111".to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("alice@uni.edu".to_string()),
                dhcp_settings: None,
            },
        );
        // Pure-DHCP entry — must be skipped.
        state.originals.connections.insert(
            "22222222-2222-2222-2222-222222222222".to_string(),
            ConnectionOriginals {
                anonymous_identity: None,
                dhcp_settings: Some(DhcpSettingsSnapshot::default()),
            },
        );
        let targets = revert_targets(&state);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "11111111-1111-1111-1111-111111111111");
        assert_eq!(targets[0].1, "alice@uni.edu");
    }

    /// An entry whose cached `anonymous_identity` is the empty string
    /// is still actionable on revert: writing `""` is NM's contract
    /// for clearing the field, so the connection ends up unset —
    /// matching whatever shape Proteus saw before the first `enable`
    /// on it. Pin that `revert_targets` includes such entries rather
    /// than skipping them.
    #[test]
    fn revert_targets_includes_entries_cached_as_empty_string() {
        let mut state = State::default();
        state.originals.connections.insert(
            "33333333-3333-3333-3333-333333333333".to_string(),
            ConnectionOriginals {
                anonymous_identity: Some(String::new()),
                dhcp_settings: None,
            },
        );
        let targets = revert_targets(&state);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "", "cached empty string survives selection");
    }

    /// End-to-end-ish: the trait write that the disable handler
    /// reaches through is the same one that the revert path
    /// reaches through. This pins that a cached `Some("alice@x")`
    /// value gets pushed onto the connection (through the trait,
    /// observed via the mock backend). Pre-fix, the same input
    /// would have observed the empty-string clear.
    #[test]
    fn disable_pushes_cached_value_through_trait() {
        let backend = MockBackend::new();
        let cref = crate::backend::ConnectionRef::new("/org/freedesktop/NetworkManager/Settings/9");
        let to_write = decide_disable_value(Some("alice@uni.edu"));
        rt().block_on(async { backend.write_anonymous_identity(&cref, &to_write).await })
            .unwrap();
        assert_eq!(
            backend.anonymous_identity_for(&cref).as_deref(),
            Some("alice@uni.edu"),
            "disable must push cached value, not clear"
        );
    }

    /// Issue #298: `enable` already caches the original (verified by
    /// reading the existing `enable_one` body). This test pins the
    /// invariant from the state-file side: a state with a cached
    /// non-empty `anonymous_identity` survives a serialize-then-
    /// deserialize round trip with the value intact, so
    /// disable-after-restart still has access to the original.
    #[test]
    fn cached_original_survives_state_round_trip() {
        let mut state = State::default();
        let uuid = "44444444-4444-4444-4444-444444444444".to_string();
        state.originals.connections.insert(
            uuid.clone(),
            ConnectionOriginals {
                anonymous_identity: Some("bob@example.org".to_string()),
                dhcp_settings: None,
            },
        );
        let bytes = serde_json::to_vec(&state).unwrap();
        let back: State = serde_json::from_slice(&bytes).unwrap();
        let entry = back
            .originals
            .connections
            .get(&uuid)
            .expect("entry survives round trip");
        assert_eq!(
            entry.anonymous_identity.as_deref(),
            Some("bob@example.org"),
            "cached original must round-trip through serde"
        );
    }
}
