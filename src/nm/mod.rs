// SPDX-License-Identifier: GPL-3.0-or-later

pub mod apply;
pub mod dhcp;

use anyhow::{Context, Result, anyhow, bail};
use zbus::proxy;
use zbus::zvariant::OwnedObjectPath;

use crate::mac::Mac;

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
    fn get_device_by_ip_iface(&self, iface: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    /// Activate a connection profile on a device. Used by `dhcp renew`
    /// as the fallback after a forced `Disconnect` when the running NM
    /// doesn't support `Device.Reapply`.
    fn activate_connection(
        &self,
        connection: &zbus::zvariant::ObjectPath<'_>,
        device: &zbus::zvariant::ObjectPath<'_>,
        specific_object: &zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    #[zbus(property, name = "Managed")]
    fn managed(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "AvailableConnections")]
    fn available_connections(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
    /// Active connection on this device (NM ActiveConnection object path)
    /// or `/` (root path) when no connection is active. Roadmap Milestone
    /// 4c: `dhcp renew` consults this to decide between the cheap
    /// `Reapply` path and the more disruptive Disconnect+ActivateConnection
    /// fallback.
    #[zbus(property, name = "ActiveConnection")]
    fn active_connection(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    /// Issue #217: NM device state integer (100 == Activated, etc.). The
    /// connection-up event source polls this every 2s to detect the
    /// any-prior → Activated transition that the dispatcher signal
    /// surfaces synchronously elsewhere. Naming with explicit `name =
    /// "State"` keeps the Rust accessor `state()` aligned with the
    /// DBus property without colliding with anything else on the proxy.
    #[zbus(property, name = "State")]
    fn state(&self) -> zbus::Result<u32>;
    /// Re-apply the connection's current settings to the running device
    /// without bringing the link down. NM 1.2+. The empty-dict / version=0
    /// / flags=0 form (the one Proteus uses for DHCP renew) tells NM "use
    /// the stored settings as-is" which triggers a fresh DHCP exchange
    /// without changing L2.
    fn reapply(
        &self,
        connection: ConnectionSettings,
        version_id: u64,
        flags: u32,
    ) -> zbus::Result<()>;
    /// Disconnect the device. Used as the fallback when `Reapply` isn't
    /// supported by the running NM (≤1.0) or returns `NotSupported`.
    fn disconnect(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
    fn get_connection_by_uuid(&self, uuid: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

pub type ConnectionSettings = std::collections::HashMap<
    String,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
>;

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Connection {
    fn get_settings(&self) -> zbus::Result<ConnectionSettings>;
    /// Fetch the secrets dict for one setting (e.g. `"802-1x"`). NM returns
    /// the secrets keyed by setting name — the keys inside (e.g. `password`,
    /// `private-key-password`) are exactly what NM accepts back through
    /// `Update`, so the result can be merged straight into the settings
    /// dict before calling `Update` to avoid clobbering the secrets store.
    fn get_secrets(&self, setting_name: &str) -> zbus::Result<ConnectionSettings>;
    fn update(&self, settings: ConnectionSettings) -> zbus::Result<()>;
    /// NBE.7: monotonic version counter NM bumps on every `Update`. Pass
    /// the current value into `Device.Reapply` so a concurrent `nmcli
    /// connection modify` between our read and our reapply surfaces as
    /// a DBus `InvalidArguments` (NM's documented version-mismatch
    /// signal) rather than silently overwriting the in-flight edit.
    /// NM 1.20+; older NM doesn't expose the property and the Reapply
    /// fall-through with `version=0` is the only path available.
    #[zbus(property, name = "VersionId")]
    fn version_id(&self) -> zbus::Result<u64>;
}

// NetworkManager device-type integer constants (subset).
pub const DEVICE_TYPE_ETHERNET: u32 = 1;
pub const DEVICE_TYPE_WIFI: u32 = 2;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub interface: String,
    pub kind: DeviceKind,
    pub hw_address: Option<String>,
    pub path: zbus::zvariant::OwnedObjectPath,
    pub managed: bool,
    pub connections: Vec<zbus::zvariant::OwnedObjectPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Wifi,
    Ethernet,
    Other(u32),
}

impl DeviceKind {
    pub fn from_nm(i: u32) -> Self {
        match i {
            DEVICE_TYPE_ETHERNET => Self::Ethernet,
            DEVICE_TYPE_WIFI => Self::Wifi,
            other => Self::Other(other),
        }
    }

    pub fn setting_key(&self) -> Option<&'static str> {
        match self {
            Self::Wifi => Some("802-11-wireless"),
            Self::Ethernet => Some("802-3-ethernet"),
            Self::Other(_) => None,
        }
    }
}

/// N3: probe the running NM's introspected DBus interface version so
/// callers can fall back gracefully on a `Reapply`/`Disconnect` etc.
/// branch when the running daemon predates the method. NM's
/// `org.freedesktop.NetworkManager.Version` property is a string like
/// `"1.42.4"`. Returns `None` if the property read errors (older NM
/// /
///                                                       a host that
/// declines the introspection); callers should treat `None` as
/// "assume modern" but can downgrade if they want to.
pub async fn probe_version(conn: &zbus::Connection) -> Option<String> {
    // The proxy doesn't declare `Version` as a typed property, but a
    // raw `Properties.Get` works against any NM that exposes the root
    // interface (every supported version does). We dispatch through
    // the standard freedesktop properties proxy to stay version-agnostic.
    use zbus::zvariant::Value;
    let props = match zbus::fdo::PropertiesProxy::builder(conn)
        .destination("org.freedesktop.NetworkManager")
        .ok()?
        .path("/org/freedesktop/NetworkManager")
        .ok()?
        .build()
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("nm probe_version: properties proxy failed: {e}");
            return None;
        }
    };
    let owned = match props
        .get("org.freedesktop.NetworkManager".try_into().ok()?, "Version")
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("nm probe_version: Properties.Get(Version) failed: {e}");
            return None;
        }
    };
    // OwnedValue derefs into Value via the standard Deref impl; use the
    // same pattern as `extract_str` elsewhere in this module.
    let v: &Value = &owned;
    if let Value::Str(s) = v {
        Some(s.as_str().to_string())
    } else {
        None
    }
}

pub async fn list_devices(conn: &zbus::Connection) -> Result<Vec<DeviceInfo>> {
    let nm = NetworkManagerProxy::new(conn).await.context(
        "connecting to NetworkManager DBus root proxy at \
                  /org/freedesktop/NetworkManager",
    )?;
    let paths = nm.get_devices().await.context(
        "calling NetworkManager.GetDevices on \
                  /org/freedesktop/NetworkManager",
    )?;
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let dev = DeviceProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        // Issue #248: previously every property read used `unwrap_or_default()`,
        // which silently produced ghost devices (empty iface name, kind
        // `Other(0)`, no connections) when NM raced a `Removed` signal
        // against our enumeration. The ghost would then be silently
        // skipped by every downstream consumer with no log line — the
        // operator never knew a rotation had been swallowed. Now: any
        // single property failing skips the device with a logged warning
        // that names the device path AND the property that failed, so
        // a `journalctl -t proteus` grep is enough to debug it.
        macro_rules! read_or_skip {
            ($call:expr, $prop:literal) => {
                match $call.await {
                    Ok(v) => v,
                    Err(e) => {
                        // S8: NM property-read errors can echo
                        // connection-setting values verbatim. Route
                        // through `display_string` before tracing so an
                        // attacker-controlled string in the dict can't
                        // redraw `journalctl -t proteus`.
                        let safe = crate::display::display_string(&format!("{e:#}"));
                        tracing::warn!(
                            device = %path.as_str(),
                            property = $prop,
                            "NM device property read failed; skipping device: {safe}"
                        );
                        continue;
                    }
                }
            };
        }
        let iface: String = read_or_skip!(dev.interface(), "Interface");
        let dt: u32 = read_or_skip!(dev.device_type(), "DeviceType");
        // HwAddress is allowed to be missing — virtual / non-L2 devices
        // legitimately don't expose one. We propagate `None` rather than
        // skip the device. A real DBus failure is still distinguishable
        // from "the property doesn't exist" because `.ok()` returns
        // `None` for both — but the operator path that cares (rotate)
        // surfaces the missing-MAC condition with its own clear error.
        let hw = dev.hw_address().await.ok();
        let managed: bool = read_or_skip!(dev.managed(), "Managed");
        let conns = read_or_skip!(dev.available_connections(), "AvailableConnections");
        out.push(DeviceInfo {
            interface: iface,
            kind: DeviceKind::from_nm(dt),
            hw_address: hw,
            path,
            managed,
            connections: conns,
        });
    }
    Ok(out)
}

pub async fn find_device_by_iface(conn: &zbus::Connection, iface: &str) -> Result<DeviceInfo> {
    let devs = list_devices(conn).await?;
    devs.into_iter()
        .find(|d| d.interface == iface)
        .ok_or_else(|| anyhow!("no NetworkManager device for interface '{iface}'"))
}

/// Parse a colon/dash/bare-hex MAC string into the 6-byte vector NM expects on
/// the wire. NM's `cloned-mac-address` (and equivalent on `802-3-ethernet`) is
/// declared as `ay` in the DBus introspection XML; older NM (1.20–1.36) hard
/// rejects a string. We feed the result through `Mac::from_str` so callers get
/// the same parse behaviour and error messages they already have for
/// rotation/pin paths.
pub fn mac_string_to_bytes(s: &str) -> Result<Vec<u8>> {
    let mac: Mac = s
        .parse()
        .with_context(|| format!("parsing MAC '{s}' for NM cloned-mac-address (ay)"))?;
    Ok(mac.octets().to_vec())
}

/// Connection-setting sections that may carry NM-stored secrets. Whenever we
/// `Settings.Connection.Update` a profile, we must merge each of these
/// sections' `GetSecrets` results back in or NM will interpret the absence of
/// the keys as "user cleared their password" and wipe its secrets store.
///
/// Issue #207 (and original #114 fix): four call sites mutate connection
/// settings via `Update` — `nm::apply::set_cloned_mac`, `nm::dhcp::update_connection`,
/// `ipv6::nm::apply_settings`, and `enterprise_wifi::nm::write_anonymous_identity`.
/// Each one must round-trip through this list, not just the section it
/// directly touches: rotating a Wi-Fi MAC must preserve the WPA-PSK; updating
/// 802.1X anonymous-identity must preserve PEAP/EAP-TLS passwords; updating
/// IPv6 keys on an enterprise Wi-Fi connection must preserve both. The list
/// is the union of every secret-bearing section NM exposes that Proteus
/// could plausibly touch.
pub const SECRET_SECTIONS: &[&str] = &[
    "802-11-wireless-security",
    "802-1x",
    "vpn",
    "wireguard",
    "gsm",
    "cdma",
    "pppoe",
    "macsec",
];

/// Merge a `GetSecrets` result into a settings dict in place.
///
/// NM's `GetSecrets(setting_name)` returns a dict shaped like
/// `{ "802-1x": { "password": ..., "private-key-password": ... } }` — only
/// the secret-typed keys, keyed by section. We graft each section's secrets
/// onto the matching section in `settings`, preserving any settings already
/// in place (so the caller's freshly-modified key survives) and overwriting
/// only on key collisions inside a section.
///
/// Issue #114 / #207: without this merge, `Update` would be called with a
/// dict that lacks the secret keys NM already has stored, and NM interprets
/// that as "the user removed their password".
pub fn merge_secrets(settings: &mut ConnectionSettings, secrets: &ConnectionSettings) {
    for (section_name, section_secrets) in secrets {
        let target = settings.entry(section_name.clone()).or_default();
        for (key, value) in section_secrets {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// Classification of a single `GetSecrets` call outcome.
///
/// `Absent` covers every "no secrets to merge" path NM legitimately uses —
/// the section isn't on this profile, the section has no secret-typed keys,
/// or NM's agent returned an empty dict. `Hard` is everything else: a polkit
/// denial, a DBus disconnect, an NM crash mid-call. The split exists so
/// `update_with_secrets` can fail fast on `Hard` (and not call `Update`
/// with a stripped dict that would wipe the secrets store) while still
/// treating `Absent` as the routine no-op it is on the WPA-PSK / plain
/// ethernet / no-VPN path.
#[derive(Debug)]
pub enum GetSecretsOutcome {
    /// NM returned a populated secrets dict for the section.
    Merged(ConnectionSettings),
    /// NM has nothing to merge for this section. Safe to continue.
    Absent,
    /// NM returned an error that may indicate a real breakage. The caller
    /// must treat this as a hard failure — proceeding to `Update` would
    /// wipe the secrets store for the affected section.
    Hard(anyhow::Error),
}

/// Classify a `zbus::Error` from `Settings.Connection.GetSecrets` into the
/// "no secrets to merge" / "real failure" buckets.
///
/// Returns `true` when the error variant indicates NM legitimately has
/// nothing to hand back for this section (section absent, section without
/// secret keys, agent returned an empty result). Returns `false` when the
/// error could indicate a real failure — DBus disconnect, polkit denial,
/// arbitrary `MethodError` payload — and the caller must NOT proceed to
/// `Update` with a stripped dict.
///
/// Approach mirrors `bluetooth::apply::is_adapter_gone`: match the typed
/// `zbus::Error` / `zbus::fdo::Error` variants the runtime actually emits
/// rather than substring-sniffing the rendered error message. The
/// `MethodError` arm pins the exact NM error-name strings we accept as
/// benign; anything else propagates so the operator sees the breakage in
/// the journal.
fn get_secrets_error_is_benign(err: &zbus::Error) -> bool {
    match err {
        // FDO-typed errors. NM sometimes routes "section/property not on
        // this connection" through these rather than a MethodError —
        // accept the no-such-{property,interface,object} variants and
        // refuse everything else (AccessDenied / NoReply / IOError / …
        // remain hard failures).
        zbus::Error::FDO(boxed) => matches!(
            **boxed,
            zbus::fdo::Error::UnknownProperty(_) | zbus::fdo::Error::UnknownInterface(_)
        ),
        // NM-typed MethodErrors: the canonical "section not on this
        // profile" / "no agent could supply / had no secrets" set.
        // Anything outside this list — including `AccessDenied`
        // (polkit) and the AgentManager's `UserCanceled` /
        // `PermissionDenied` — is a hard failure.
        zbus::Error::MethodError(name, _, _) => {
            let s = name.as_str();
            matches!(
                s,
                "org.freedesktop.NetworkManager.Settings.Connection.SettingNotFound"
                    | "org.freedesktop.NetworkManager.InvalidSetting"
                    | "org.freedesktop.NetworkManager.AgentManager.NoSecrets"
            )
        }
        _ => false,
    }
}

/// Fetch the secrets dict for one section, classifying failure into the
/// "no secrets to merge" vs "real failure" buckets and emitting a tracing
/// line at the appropriate level.
///
/// E6: introduced as the single chokepoint for `GetSecrets` error handling
/// so each `update_with_secrets` callsite agrees on what counts as benign
/// (skip + `debug!`) vs hard (return `Hard` + `error!`). Operators chasing
/// a silent "auth broke after rotate" symptom now have a single grep
/// target — `GetSecrets failed with non-benign error` — that names the
/// profile, uuid, section, and underlying DBus error.
///
/// `connection_label` is whatever identifier the caller chose for the
/// log line. `update_with_secrets` passes a `"<profile> (<uuid>)"`
/// rendering pulled out of the in-flight settings dict.
async fn get_secrets_or_warn(
    proxy: &ConnectionProxy<'_>,
    section: &str,
    connection_path: &OwnedObjectPath,
    connection_label: &str,
) -> GetSecretsOutcome {
    match proxy.get_secrets(section).await {
        Ok(s) if s.is_empty() => {
            // NM returns `Ok({})` when the section exists but stores no
            // secrets — the WPA2-Enterprise-without-saved-password case,
            // among others. Distinguish it from a populated merge so
            // callers don't `merge_secrets` an empty dict (a no-op
            // today, but the distinction is useful for the trace).
            tracing::debug!(
                method = "Settings.Connection.GetSecrets",
                path = %connection_path.as_str(),
                connection = %connection_label,
                section = section,
                "GetSecrets returned an empty dict; nothing to merge"
            );
            GetSecretsOutcome::Absent
        }
        Ok(s) => GetSecretsOutcome::Merged(s),
        Err(e) => {
            // S8: NM error messages can echo connection-setting values
            // verbatim ("Could not parse value 'foo' for key …"). For
            // an enterprise-Wi-Fi connection that's attacker-influenced
            // — the AP supplies the realm / anonymous-identity, the
            // dispatcher feeds it through GetSettings, and an
            // unsanitized error message reaches journald with embedded
            // ANSI / BiDi controls. Route every dict-influenced byte
            // through `display_string` before tracing so a hostile peer
            // can't redraw the operator's terminal via
            // `journalctl -t proteus`.
            let raw = format!("{e:#}");
            let safe = crate::display::display_string(&raw);
            if get_secrets_error_is_benign(&e) {
                // Documented "nothing to merge" path — the section isn't
                // on this profile (e.g. asking for `802-1x` on a plain
                // WPA-PSK Wi-Fi, asking for `vpn` on a wired profile).
                // `debug!` keeps the journal calm on the common case.
                tracing::debug!(
                    method = "Settings.Connection.GetSecrets",
                    path = %connection_path.as_str(),
                    connection = %connection_label,
                    section = section,
                    "GetSecrets returned no secrets to merge: {safe}"
                );
                GetSecretsOutcome::Absent
            } else {
                // E6: a hard GetSecrets failure (polkit denial, DBus
                // disconnect, NM crash mid-call) was previously logged
                // and swallowed — `update_with_secrets` would proceed
                // to `Update` with a settings dict missing the secret
                // keys, which NM interprets as "the user cleared their
                // password" and wipes the secrets store. The operator
                // saw nothing alarming in the journal beyond a stray
                // log line and the next reconnect silently failed to
                // auth.
                //
                // Now: `error!` so the breakage is visible AND the
                // caller surfaces an `Err` instead of continuing to
                // Update — the secret is preserved exactly because we
                // refused to push a stripped dict. The error level is
                // `error!` (not `warn!`) because we're on the hot path
                // for a mutation that's about to land; the operator
                // needs to know it didn't.
                tracing::error!(
                    method = "Settings.Connection.GetSecrets",
                    path = %connection_path.as_str(),
                    connection = %connection_label,
                    section = section,
                    "GetSecrets failed with non-benign error; refusing Update to avoid wiping secrets: {safe}"
                );
                GetSecretsOutcome::Hard(anyhow::Error::new(e).context(format!(
                    "getting secrets for connection {connection_label} (section {section})"
                )))
            }
        }
    }
}

/// Pull `connection.id` (display name) and `connection.uuid` out of a
/// settings dict so error/warning lines can carry a human-meaningful
/// handle on the connection beyond the bare object path. Returns
/// `"<unknown> (<no-uuid>)"` if neither key is present — the trace stays
/// well-formed even on a stripped or malformed dict.
fn settings_connection_label(settings: &ConnectionSettings) -> String {
    let connection = settings.get("connection");
    let id = connection
        .and_then(|s| s.get("id"))
        .and_then(|v| {
            let v: &zbus::zvariant::Value = v;
            if let zbus::zvariant::Value::Str(s) = v {
                Some(s.as_str().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "<unknown>".to_string());
    let uuid = connection
        .and_then(|s| s.get("uuid"))
        .and_then(|v| {
            let v: &zbus::zvariant::Value = v;
            if let zbus::zvariant::Value::Str(s) = v {
                Some(s.as_str().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "<no-uuid>".to_string());
    // Route the display name through `display_string` so a profile id
    // with embedded ANSI / BiDi can't redraw `journalctl -t proteus`.
    // The uuid is well-formed hex-with-dashes and doesn't need
    // sanitisation, but the id is user-authored on the keyfile side.
    let safe_id = crate::display::display_string(&id);
    format!("{safe_id} ({uuid})")
}

/// Push a `Settings.Connection.Update` after grafting every relevant secrets
/// section back onto `settings`. Issue #207: every NM `Update` site must go
/// through this so a connection's stored PSK / EAP passwords / VPN secrets
/// survive the round trip.
///
/// `GetSettings` strips secret-typed keys, so calling `Update` with the
/// stripped dict tells NM "the user cleared this password" and wipes the
/// secrets store. We pull `GetSecrets` for each section in [`SECRET_SECTIONS`]
/// through the [`get_secrets_or_warn`] chokepoint, which classifies failure
/// into "section legitimately has no secrets" (continue) vs "DBus/agent
/// breakage" (return `Err` without touching `Update` — the secret stays
/// intact because we never push a stripped dict).
///
/// E6: pre-fix, every `GetSecrets` error was silently swallowed at
/// `debug!`. An operator with an enterprise-Wi-Fi profile and a
/// transient polkit denial mid-rotate would see no warning, the rotate
/// would land an empty-secrets dict, and the next reconnect would
/// silently fail to auth. The hard-failure split now surfaces those
/// errors at `error!` AND propagates them to the caller so the rotate
/// fails loudly instead of corrupting the secrets store.
pub async fn update_with_secrets(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    mut settings: ConnectionSettings,
) -> Result<()> {
    // Resolve a human label once so every per-section trace line carries
    // the same handle — `connection.id` + uuid pulled out of the dict
    // the caller already prepared.
    let connection_label = settings_connection_label(&settings);
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await
        .with_context(|| {
            // N4: include the connection path AND the human label in the
            // error context so an operator chasing a Settings.Connection
            // failure can see which profile (and therefore which iface)
            // tripped without grepping a separate journald line.
            format!(
                "building Settings.Connection proxy on path {} for {connection_label}",
                connection_path.as_str()
            )
        })?;
    for section in SECRET_SECTIONS {
        // E6: hard-failure outcomes return Err here so we never reach
        // `proxy.update(settings)` with a stripped secrets dict — that
        // would be the silent-secret-wipe symptom the brief calls out.
        match get_secrets_or_warn(&proxy, section, connection_path, &connection_label).await {
            GetSecretsOutcome::Merged(s) => merge_secrets(&mut settings, &s),
            GetSecretsOutcome::Absent => {
                // Logged at debug! inside `get_secrets_or_warn` — nothing
                // more to do for this section.
            }
            GetSecretsOutcome::Hard(err) => return Err(err),
        }
    }
    proxy.update(settings).await.with_context(|| {
        format!(
            "calling Settings.Connection.Update on {} for {connection_label}",
            connection_path.as_str()
        )
    })?;
    Ok(())
}

/// Map an `ipv6.addr-gen-mode` token (as it appears in our config and on the
/// wire in NM keyfile/nmcli) to the integer DBus expects. Per NM's
/// `NMSettingIP6ConfigAddrGenMode` enum:
///
/// - `default`           → `0`
/// - `eui64`             → `1`
/// - `stable-privacy`    → `2`
/// - `default-or-eui64`  → `3`
///
/// The DBus property is signature `i` (i32). NM 1.37+ tolerates a string and
/// coerces, but 1.20–1.36 rejects it, leaving the connection inconsistent.
pub fn addr_gen_mode_to_int(s: &str) -> Result<i32> {
    match s {
        "default" => Ok(0),
        "eui64" => Ok(1),
        "stable-privacy" => Ok(2),
        "default-or-eui64" => Ok(3),
        other => bail!(
            "unknown ipv6.addr-gen-mode '{other}'; expected one of \
             default, eui64, stable-privacy, default-or-eui64"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_string_to_bytes_parses_uppercase_colon_form() {
        let bytes = mac_string_to_bytes("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(bytes, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn mac_string_to_bytes_parses_lowercase_dash_form() {
        let bytes = mac_string_to_bytes("aa-bb-cc-dd-ee-ff").unwrap();
        assert_eq!(bytes, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn mac_string_to_bytes_rejects_garbage() {
        assert!(mac_string_to_bytes("not-a-mac").is_err());
        assert!(mac_string_to_bytes("AA:BB:CC:DD:EE").is_err());
        assert!(mac_string_to_bytes("").is_err());
    }

    #[test]
    fn addr_gen_mode_to_int_known_modes() {
        assert_eq!(addr_gen_mode_to_int("default").unwrap(), 0);
        assert_eq!(addr_gen_mode_to_int("eui64").unwrap(), 1);
        assert_eq!(addr_gen_mode_to_int("stable-privacy").unwrap(), 2);
        assert_eq!(addr_gen_mode_to_int("default-or-eui64").unwrap(), 3);
    }

    #[test]
    fn addr_gen_mode_to_int_rejects_unknown() {
        assert!(addr_gen_mode_to_int("garbage").is_err());
        assert!(addr_gen_mode_to_int("").is_err());
        assert!(addr_gen_mode_to_int("STABLE-PRIVACY").is_err());
    }

    // -----------------------------------------------------------------
    // E6: GetSecrets failure classification.
    //
    // We can't drive `get_secrets_or_warn` from a unit test without a
    // live NM DBus connection — but the classifier `get_secrets_error_is_benign`
    // and the label extractor are pure functions, so we exercise both
    // against the documented benign and hard error variants. The
    // assertions here are the source of truth for which NM errors the
    // production code treats as "no secrets to merge" vs "abort the
    // Update".
    // -----------------------------------------------------------------

    /// `Settings.Connection.SettingNotFound` is what NM raises when the
    /// caller asks for `GetSecrets("802-1x")` on a plain WPA-PSK
    /// connection (no 802.1X section). Must classify as benign so the
    /// merge skip doesn't bubble out as an `error!`.
    #[test]
    fn benign_classifies_nm_setting_not_found() {
        // Build the MethodError using zbus's typed constructor: we
        // can't easily fabricate a `Message` body in a unit test, so
        // we shape the error variant directly. The variant carries an
        // OwnedErrorName built from a string; the runtime equivalent
        // travels the same shape.
        let name = zbus::names::OwnedErrorName::try_from(
            "org.freedesktop.NetworkManager.Settings.Connection.SettingNotFound",
        )
        .unwrap();
        // The `Message` payload is opaque to the classifier; supply a
        // synthetic one from a method call signature so we can
        // construct the variant.
        let payload = synthetic_method_message();
        let err = zbus::Error::MethodError(name, Some("no such setting".into()), payload);
        assert!(
            get_secrets_error_is_benign(&err),
            "SettingNotFound must be benign"
        );
    }

    /// NM's `AgentManager.NoSecrets` covers the "no agent could supply
    /// secrets for this section" path. Also benign — the merge skip
    /// doesn't break anything when there were no secrets to begin with.
    #[test]
    fn benign_classifies_agent_manager_no_secrets() {
        let name = zbus::names::OwnedErrorName::try_from(
            "org.freedesktop.NetworkManager.AgentManager.NoSecrets",
        )
        .unwrap();
        let payload = synthetic_method_message();
        let err = zbus::Error::MethodError(name, None, payload);
        assert!(
            get_secrets_error_is_benign(&err),
            "AgentManager.NoSecrets must be benign"
        );
    }

    /// FDO `UnknownProperty` is what NM raises when a section is on
    /// the profile but the requested property isn't readable. The
    /// classifier treats this as benign too — there's no secret to
    /// merge.
    #[test]
    fn benign_classifies_fdo_unknown_property() {
        let inner = zbus::fdo::Error::UnknownProperty("Secrets".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(
            get_secrets_error_is_benign(&err),
            "UnknownProperty must be benign"
        );
    }

    /// Polkit denial: `AccessDenied` is a hard failure. The operator's
    /// authn flow couldn't sign the GetSecrets call — proceeding to
    /// Update would push a stripped dict and NM would wipe the secret.
    /// The classifier must refuse this so `update_with_secrets`
    /// returns `Err` and the caller (rotate, dhcp, ipv6, anonymous-id
    /// write) surfaces the failure.
    #[test]
    fn hard_classifies_fdo_access_denied() {
        let inner = zbus::fdo::Error::AccessDenied("polkit declined".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(
            !get_secrets_error_is_benign(&err),
            "FDO AccessDenied must NOT be classified benign — would silently wipe the secret"
        );
    }

    /// DBus disconnect: NM crashed mid-call, socket closed, etc. Hard
    /// failure for the same reason as AccessDenied — Update would
    /// strip the secret.
    #[test]
    fn hard_classifies_fdo_no_reply() {
        let inner = zbus::fdo::Error::NoReply("daemon went away".into());
        let err = zbus::Error::FDO(Box::new(inner));
        assert!(
            !get_secrets_error_is_benign(&err),
            "NoReply must be a hard failure"
        );
    }

    /// Address-level zbus error (couldn't even open the bus). Definitely
    /// hard — we shouldn't be running `Update` at all in this state.
    #[test]
    fn hard_classifies_bus_address_error() {
        let err = zbus::Error::Address("not a bus address".to_string());
        assert!(
            !get_secrets_error_is_benign(&err),
            "Address errors must be hard failures"
        );
    }

    /// An unrecognised NM `MethodError` name must default to "hard
    /// failure". The brief's invariant: a transient polkit / agent
    /// breakage NM signals with a new error name we haven't seen
    /// before must NOT be silently swallowed.
    #[test]
    fn hard_classifies_unknown_nm_method_error() {
        let name = zbus::names::OwnedErrorName::try_from(
            "org.freedesktop.NetworkManager.Settings.Connection.SomeNewError",
        )
        .unwrap();
        let payload = synthetic_method_message();
        let err = zbus::Error::MethodError(name, None, payload);
        assert!(
            !get_secrets_error_is_benign(&err),
            "Unknown NM MethodError names must default to hard failure"
        );
    }

    /// Pin the exact set of MethodError names we accept as benign.
    /// If a future NM rev renames one of these (or adds a new "no
    /// secrets" variant), the test forces the classifier update to
    /// happen as a deliberate code change rather than a silent runtime
    /// drift to "always log error!".
    #[test]
    fn benign_method_error_name_set_is_documented() {
        let names = [
            "org.freedesktop.NetworkManager.Settings.Connection.SettingNotFound",
            "org.freedesktop.NetworkManager.InvalidSetting",
            "org.freedesktop.NetworkManager.AgentManager.NoSecrets",
        ];
        let payload = synthetic_method_message();
        for n in names {
            let name = zbus::names::OwnedErrorName::try_from(n).unwrap();
            let err = zbus::Error::MethodError(name, None, payload.clone());
            assert!(
                get_secrets_error_is_benign(&err),
                "get_secrets_error_is_benign must accept {n}"
            );
        }
    }

    /// E6: the per-section label format `<id> (<uuid>)` is what the
    /// trace lines and the error context use. Verify the extractor
    /// pulls both fields out of a typical settings dict and sanitises
    /// the id field via `display_string` (so an attacker-controlled
    /// profile name can't redraw `journalctl -t proteus`).
    #[test]
    fn settings_connection_label_includes_id_and_uuid() {
        let mut s = ConnectionSettings::new();
        let section = s.entry("connection".to_string()).or_default();
        section.insert(
            "id".to_string(),
            zbus::zvariant::Value::from("Eduroam".to_string())
                .try_into()
                .unwrap(),
        );
        section.insert(
            "uuid".to_string(),
            zbus::zvariant::Value::from("12345678-aaaa-bbbb-cccc-1234567890ab".to_string())
                .try_into()
                .unwrap(),
        );
        let label = settings_connection_label(&s);
        assert_eq!(label, "Eduroam (12345678-aaaa-bbbb-cccc-1234567890ab)");
    }

    /// E6: a dict without a `connection` section (or with neither id
    /// nor uuid) still produces a well-formed label so the trace lines
    /// stay readable on a malformed or partially-stripped input.
    #[test]
    fn settings_connection_label_handles_missing_fields() {
        let s = ConnectionSettings::new();
        let label = settings_connection_label(&s);
        assert_eq!(label, "<unknown> (<no-uuid>)");
    }

    /// E6: the id field is routed through `display_string` so a profile
    /// id with embedded ANSI / BiDi controls can't redraw the journal.
    /// We don't sanitise the uuid because NM's uuid grammar is
    /// hex-and-dashes, but the id is user-authored on the keyfile side.
    #[test]
    fn settings_connection_label_sanitises_id() {
        let mut s = ConnectionSettings::new();
        let section = s.entry("connection".to_string()).or_default();
        // Embed a CSI (ESC `[`) — display_string must escape it.
        section.insert(
            "id".to_string(),
            zbus::zvariant::Value::from("evil\x1b[31mhack".to_string())
                .try_into()
                .unwrap(),
        );
        let label = settings_connection_label(&s);
        // The escape sequence must not appear verbatim. Exact escape
        // form is `display_string`'s choice; we just assert ESC didn't
        // pass through and the label still carries the readable prefix.
        assert!(
            !label.contains('\x1b'),
            "raw ESC must not survive the label render: {label:?}"
        );
        assert!(
            label.contains("evil"),
            "the readable prefix must survive: {label:?}"
        );
    }

    /// Build a synthetic DBus `Message` so the test can construct a
    /// `MethodError` variant without standing up a real bus. The
    /// payload is opaque to `get_secrets_error_is_benign` — only the
    /// error name string is inspected — so any well-formed message
    /// works.
    fn synthetic_method_message() -> zbus::message::Message {
        zbus::message::Message::method_call("/", "Dummy")
            .unwrap()
            .build(&())
            .unwrap()
    }
}
