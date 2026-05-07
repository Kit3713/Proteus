// SPDX-License-Identifier: GPL-3.0-or-later

//! NetworkManager `ipv6.*` connection settings — addr-gen-mode, dhcp-duid,
//! and dhcp-iaid. The DUID/IAID couple DHCPv6 identity to the current MAC
//! so DUID rotates whenever the MAC does.
//!
//! Sysctl drives SLAAC IID generation; these connection-level keys are an
//! explicit belt-and-braces override at the NM layer so a future NM default
//! flip can't quietly negate the kernel-level setting. Mirrors the existing
//! `nm::apply` cloned-MAC machinery.

use anyhow::{Context, Result};
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::nm::{ConnectionProxy, ConnectionSettings};

/// NM key for stable-privacy IID generation under `[ipv6]`.
pub const ADDR_GEN_MODE_STABLE_PRIVACY: &str = "stable-privacy";
/// NM key for EUI-64 IID generation. Exposed only so revert can write it back
/// when an interface's pre-Proteus value was eui64. NEVER set this from
/// `apply` — wiki page `ipv6` is loud about the leak.
pub const ADDR_GEN_MODE_EUI64: &str = "eui64";

/// `ipv6.dhcp-duid = "ll"` — link-layer DUID. Rotates with the MAC because
/// the link-layer field is the MAC.
pub const DHCP_DUID_LL: &str = "ll";

/// `ipv6.dhcp-iaid = "mac"` — derive the IAID from the link-layer MAC. Same
/// rationale as DUID — couples the DHCPv6 identity to the rotating MAC.
pub const DHCP_IAID_MAC: &str = "mac";

/// Settings written by `apply_one`. Surfaced in apply output and in the
/// per-connection block of `proteus ipv6 status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv6NmSettings {
    pub addr_gen_mode: String,
    pub dhcp_duid: String,
    pub dhcp_iaid: String,
}

impl Default for Ipv6NmSettings {
    fn default() -> Self {
        Self {
            addr_gen_mode: ADDR_GEN_MODE_STABLE_PRIVACY.into(),
            dhcp_duid: DHCP_DUID_LL.into(),
            dhcp_iaid: DHCP_IAID_MAC.into(),
        }
    }
}

/// Read the three keys we manage from a connection's settings dict, returning
/// `None` for any that aren't set so the caller can spot first-apply-vs-reapply.
pub async fn read_settings(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
) -> Result<Ipv6Snapshot> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let settings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    Ok(Ipv6Snapshot::from_settings(&settings))
}

/// Apply Proteus's `ipv6.*` settings to one NM connection. Only the three
/// keys are touched — the rest of `[ipv6]` (method, dns, etc.) is the user's.
pub async fn apply_settings(
    conn: &zbus::Connection,
    connection_path: &OwnedObjectPath,
    new: &Ipv6NmSettings,
) -> Result<()> {
    let proxy = ConnectionProxy::builder(conn)
        .path(connection_path.clone())?
        .build()
        .await?;
    let mut settings: ConnectionSettings = proxy
        .get_settings()
        .await
        .context("calling Settings.Connection.GetSettings")?;
    let entry = settings.entry("ipv6".to_string()).or_default();
    entry.insert(
        "addr-gen-mode".to_string(),
        Value::from(new.addr_gen_mode.clone()).try_into()?,
    );
    entry.insert(
        "dhcp-duid".to_string(),
        Value::from(new.dhcp_duid.clone()).try_into()?,
    );
    entry.insert(
        "dhcp-iaid".to_string(),
        Value::from(new.dhcp_iaid.clone()).try_into()?,
    );
    proxy
        .update(settings)
        .await
        .context("calling Settings.Connection.Update")?;
    Ok(())
}

/// Snapshot of the three keys we read off a connection. Carries `Option`
/// because NM omits unset keys from `GetSettings` rather than returning a
/// default; an absent key is informative for revert ("we never touched it").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ipv6Snapshot {
    pub addr_gen_mode: Option<String>,
    pub dhcp_duid: Option<String>,
    pub dhcp_iaid: Option<String>,
}

impl Ipv6Snapshot {
    fn from_settings(settings: &ConnectionSettings) -> Self {
        let mut out = Self::default();
        let Some(section) = settings.get("ipv6") else {
            return out;
        };
        out.addr_gen_mode = lookup_str(section, "addr-gen-mode");
        out.dhcp_duid = lookup_str(section, "dhcp-duid");
        out.dhcp_iaid = lookup_str(section, "dhcp-iaid");
        out
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

    #[test]
    fn defaults_match_proteus_policy() {
        let s = Ipv6NmSettings::default();
        assert_eq!(s.addr_gen_mode, ADDR_GEN_MODE_STABLE_PRIVACY);
        assert_eq!(s.dhcp_duid, DHCP_DUID_LL);
        assert_eq!(s.dhcp_iaid, DHCP_IAID_MAC);
    }

    #[test]
    fn snapshot_from_empty_settings_yields_all_none() {
        let s = Ipv6Snapshot::from_settings(&ConnectionSettings::new());
        assert_eq!(s, Ipv6Snapshot::default());
    }
}
