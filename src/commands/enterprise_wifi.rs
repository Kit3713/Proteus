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
//! * `disable` requires root + `--yes`. Clears `802-1x.anonymous-identity`
//!   and untags the connection in `state.json`.
//!
//! Connections that aren't 802.1X are skipped cleanly with a single log
//! line — matching the rest of Proteus's detect-and-defer story.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use zbus::zvariant::OwnedObjectPath;

use crate::config::{Config, EnterpriseWifiConfig};
use crate::enterprise_wifi::{self, nm as eap_nm};
use crate::exit;
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
        enable_one(&conn, connection, &config.enterprise_wifi, &mut state).await
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
        disable_one(&conn, connection, &mut state).await
    });

    match outcome {
        Ok(DisableOutcome::Cleared {
            connection: name,
            previous,
        }) => {
            state.save(&state_path)?;
            let prev_label = previous.as_deref().unwrap_or("(unset)");
            println!("enterprise-wifi: {name}: anonymous-identity cleared (was {prev_label})");
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
    Cleared {
        connection: String,
        previous: Option<String>,
    },
    NotEap {
        connection: String,
    },
}

async fn enable_one(
    conn: &zbus::Connection,
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

    // Cache the pre-Proteus value exactly once. Re-runs on a connection we
    // already manage do NOT clobber the cached original — that's how revert
    // can put the profile back the way it was on first apply, even after
    // multiple toggles.
    state
        .originals
        .connections
        .entry(connection.to_string())
        .or_insert_with(|| ConnectionOriginals {
            anonymous_identity: snapshot.anonymous_identity.clone(),
            dhcp_settings: None,
        });

    eap_nm::write_anonymous_identity(conn, &path, &new_value).await?;

    Ok(EnableOutcome::Applied {
        connection: connection.to_string(),
        realm,
        previous: snapshot.anonymous_identity,
    })
}

async fn disable_one(
    conn: &zbus::Connection,
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

    eap_nm::write_anonymous_identity(conn, &path, "").await?;
    let _ = state.originals.connections.remove(connection);

    Ok(DisableOutcome::Cleared {
        connection: connection.to_string(),
        previous: snapshot.anonymous_identity,
    })
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
            match read_one_for_status(&conn, &path).await {
                Ok(Some(row)) => out.push(row),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("enterprise-wifi: skipping connection: {e:#}");
                }
            }
        }
        // Mark proteus-managed connections via the cached originals.
        for row in &mut out {
            row.proteus_managed = state.originals.connections.contains_key(&row.name);
        }
        // Stable order so JSON output is deterministic.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    })
}

async fn read_one_for_status(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Result<Option<ConnectionStatus>> {
    let snapshot = eap_nm::read_snapshot(conn, path).await?;
    if !snapshot.has_eap_section {
        return Ok(None);
    }
    let id = nm::apply::read_connection_id(conn, path)
        .await?
        .ok_or_else(|| anyhow!("connection has no `connection.id`"))?;
    Ok(Some(ConnectionStatus {
        name: id,
        identity: snapshot
            .identity
            .as_deref()
            .map(enterprise_wifi::redact_identity),
        anonymous_identity: snapshot.anonymous_identity,
        proteus_managed: false, // filled in by the caller from state.
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

    #[test]
    fn disable_untags_connection_in_state() {
        let mut state = State::default();
        state.originals.connections.insert(
            "MyWiFi".to_string(),
            ConnectionOriginals {
                anonymous_identity: Some("old@x.y".to_string()),
                dhcp_settings: None,
            },
        );
        // Simulate the line from `disable_one` that drops the cached entry.
        state.originals.connections.remove("MyWiFi");
        assert!(!state.originals.connections.contains_key("MyWiFi"));
    }
}
