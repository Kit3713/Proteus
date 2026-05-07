// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Result, anyhow};

use crate::config::BluetoothConfig;

// Generic, non-host-derived strings. Anything host-derived (hostname, user
// name, device model) would re-leak the identifier the alias is meant to mask.
pub const GENERIC_ALIASES: &[&str] = &[
    "BT Device",
    "Bluetooth",
    "Bluetooth Device",
    "Linux BT",
    "Linux Bluetooth",
    "Wireless",
    "Wireless Device",
    "Audio Device",
    "Headset",
    "Speaker",
    "Mouse",
    "Keyboard",
    "Trackpad",
    "Controller",
    "Generic Adapter",
    "BLE Device",
    "BT Host",
    "Adapter",
    "Device",
];

pub fn select_alias(cfg: &BluetoothConfig) -> Result<String> {
    match cfg.alias_source.as_str() {
        "pinned" => cfg
            .pinned_alias
            .clone()
            .ok_or_else(|| anyhow!("alias_source = 'pinned' but pinned_alias is unset")),
        "generic" => generic(),
        other => Err(anyhow!(
            "unknown alias_source '{other}'; expected 'generic' or 'pinned'"
        )),
    }
}

fn generic() -> Result<String> {
    let mut buf = [0u8; 1];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("getrandom: {e}"))?;
    let idx = (buf[0] as usize) % GENERIC_ALIASES.len();
    Ok(GENERIC_ALIASES[idx].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(source: &str, pinned: Option<&str>) -> BluetoothConfig {
        BluetoothConfig {
            enabled: true,
            generic_alias: true,
            alias_source: source.into(),
            pinned_alias: pinned.map(str::to_string),
            discoverable: false,
            ble_rpa: true,
        }
    }

    #[test]
    fn generic_aliases_have_at_least_fifteen_entries() {
        assert!(
            GENERIC_ALIASES.len() >= 15,
            "need a decent pool to avoid trivial guess-the-alias",
        );
    }

    #[test]
    fn generic_aliases_have_no_host_strings() {
        // None of the entries should look like hostname/user-derived data.
        for a in GENERIC_ALIASES {
            assert!(
                !a.contains("'"),
                "alias '{a}' contains an apostrophe, suggests possessive"
            );
            assert!(!a.contains('@'), "alias '{a}' contains '@', suggests email");
            assert!(
                a.is_ascii(),
                "alias '{a}' has non-ascii chars (could leak locale)"
            );
        }
    }

    #[test]
    fn generic_returns_one_of_the_pool() {
        for _ in 0..50 {
            let pick = select_alias(&cfg("generic", None)).unwrap();
            assert!(
                GENERIC_ALIASES.contains(&pick.as_str()),
                "pick '{pick}' not in pool"
            );
        }
    }

    #[test]
    fn pinned_returns_pinned_value() {
        let pick = select_alias(&cfg("pinned", Some("MyBT"))).unwrap();
        assert_eq!(pick, "MyBT");
    }

    #[test]
    fn pinned_without_value_errors() {
        assert!(select_alias(&cfg("pinned", None)).is_err());
    }

    #[test]
    fn unknown_source_errors() {
        assert!(select_alias(&cfg("nonsense", None)).is_err());
    }
}
