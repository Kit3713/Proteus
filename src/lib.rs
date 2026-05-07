// SPDX-License-Identifier: GPL-3.0-or-later

pub mod bluetooth;
pub mod captive_portal;
pub mod cli;
pub mod commands;
pub mod config;
pub mod diff;
pub mod dns;
pub mod dry_run;
pub mod enterprise_wifi;
pub mod hostname;
pub mod ipv6;
pub mod kill_switch;
pub mod logging;
pub mod mac;
pub mod nft;
pub mod nm;
pub mod probe;
pub mod profile;
pub mod rf;
pub mod stack;
pub mod state;
pub mod timer;
pub mod version;
pub mod wiki;

pub mod exit {
    // Stable, documented exit codes. Do not renumber.
    pub const SUCCESS: u8 = 0;
    pub const GENERIC_ERROR: u8 = 1;
    // 2 is reserved for clap's invalid-args default.
    pub const NOT_IMPLEMENTED: u8 = 64;
    pub const CONFIG_ERROR: u8 = 65;
    pub const PERMISSION_ERROR: u8 = 66;
    pub const SYSTEM_NOT_SUPPORTED: u8 = 70;

    /// Mutating commands that require `--yes` use this when the flag is
    /// missing. Aliased to `CONFIG_ERROR` (65) — "you must adjust the
    /// invocation to confirm the change" — rather than `NOT_IMPLEMENTED`
    /// (64), which historically meant "the feature has not landed yet" and
    /// misled wrappers into thinking the command itself was a stub. The
    /// numeric value is unchanged so wrappers that already grep for `65`
    /// keep working; the alias just makes the intent legible at the call
    /// site.
    pub const CONFIRMATION_REQUIRED: u8 = CONFIG_ERROR;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let cfg = config::Config::default();
        let raw = cfg.to_raw_explicit();
        let s = toml::to_string_pretty(&raw).unwrap();
        let parsed: config::RawConfig = toml::from_str(&s).unwrap();
        let back = parsed.resolve();
        assert_eq!(back.probes.quorum_n, 3);
        assert_eq!(back.probes.quorum_total, 4);
        assert!(back.dns.strip_edns_client_subnet);
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit::SUCCESS, 0);
        assert_eq!(exit::NOT_IMPLEMENTED, 64);
        assert_eq!(exit::CONFIG_ERROR, 65);
        assert_eq!(exit::PERMISSION_ERROR, 66);
        assert_eq!(exit::SYSTEM_NOT_SUPPORTED, 70);
        // CONFIRMATION_REQUIRED is an intent alias; pinning the numeric
        // value documents that it's wire-compatible with CONFIG_ERROR so
        // existing wrappers don't break when callers migrate off the
        // legacy NOT_IMPLEMENTED return.
        assert_eq!(exit::CONFIRMATION_REQUIRED, 65);
    }

    #[test]
    fn version_phase_is_b() {
        assert_eq!(version::PHASE, 'B');
    }
}
