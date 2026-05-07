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
    }

    #[test]
    fn version_phase_is_b() {
        assert_eq!(version::PHASE, 'B');
    }

    /// Issue #136: passing `--skipchecksums` plus an unverified remote
    /// tarball lets an MITM swap the source. Comment lines are stripped so
    /// explanatory prose can still mention the flag without false-positive.
    #[test]
    fn release_workflow_does_not_skip_makepkg_checksums() {
        const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release.yml");
        for (lineno, line) in RELEASE_WORKFLOW.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            assert!(
                !line.contains("--skipchecksums"),
                "release.yml line {} passes --skipchecksums to makepkg (issue #136): {}",
                lineno + 1,
                line,
            );
        }
    }

    /// Issue #133: `auth_admin_keep` caches elevation for ~5 minutes,
    /// leaving a walk-away replay window. Mutating actions must use the
    /// one-shot `auth_admin`.
    #[test]
    fn polkit_mutating_actions_do_not_cache_auth() {
        const POLKIT_POLICY: &str = include_str!("../dist/polkit/com.kit3713.proteus.policy");
        assert!(
            !POLKIT_POLICY.contains("auth_admin_keep"),
            "polkit policy uses auth_admin_keep on a mutating action (issue #133)"
        );
    }
}
