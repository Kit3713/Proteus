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
pub mod state_lock;
#[cfg(test)]
pub mod testing;
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
        assert_eq!(exit::GENERIC_ERROR, 1);
        assert_eq!(exit::NOT_IMPLEMENTED, 64);
        assert_eq!(exit::CONFIG_ERROR, 65);
        assert_eq!(exit::PERMISSION_ERROR, 66);
        assert_eq!(exit::SYSTEM_NOT_SUPPORTED, 70);
        assert_eq!(exit::CONFIRMATION_REQUIRED, 65);
    }

    #[test]
    fn version_phase_is_b() {
        assert_eq!(version::PHASE, 'B');
    }

    /// Polkit `exec.path` must point at `/usr/bin/proteus`. Issue #120.
    #[test]
    fn polkit_policy_targets_usr_bin_proteus() {
        let policy = include_str!("../dist/polkit/com.kit3713.proteus.policy");
        assert!(
            policy.contains(
                "<annotate key=\"org.freedesktop.policykit.exec.path\">/usr/bin/proteus</annotate>"
            ),
            "polkit policy must annotate exec.path=/usr/bin/proteus (issue #120)"
        );
        assert!(
            !policy.contains("/usr/local/bin/proteus"),
            "polkit policy must not hardcode /usr/local/bin/proteus (issue #120)"
        );
    }

    /// Issue #134: services with After=network-online.target need matching Wants=.
    #[test]
    fn systemd_services_with_after_network_online_also_wants_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/systemd");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&dir).expect("dist/systemd should exist") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("service") {
                continue;
            }
            let body = std::fs::read_to_string(&path).unwrap();
            let has_after = body
                .lines()
                .any(|l| l.starts_with("After=") && l.contains("network-online.target"));
            if !has_after {
                continue;
            }
            let has_wants = body
                .lines()
                .any(|l| l.starts_with("Wants=") && l.contains("network-online.target"));
            assert!(
                has_wants,
                "{}: has After=network-online.target but no matching Wants= (issue #134)",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "no .service files checked under dist/systemd");
    }

    /// Issue #135: CI must use the pinned toolchain.
    #[test]
    fn ci_workflow_does_not_use_floating_stable_toolchain() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
        let body = std::fs::read_to_string(&path).expect("ci.yml should exist");
        assert!(
            !body.contains("dtolnay/rust-toolchain"),
            "ci.yml still references dtolnay/rust-toolchain (issue #135)"
        );
        assert!(
            body.contains("actions-rust-lang/setup-rust-toolchain@v1"),
            "ci.yml should use actions-rust-lang/setup-rust-toolchain@v1"
        );
    }

    /// Issue #136: makepkg must not run with --skipchecksums.
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

    /// Issue #133: mutating polkit actions must use one-shot auth_admin.
    #[test]
    fn polkit_mutating_actions_do_not_cache_auth() {
        const POLKIT_POLICY: &str = include_str!("../dist/polkit/com.kit3713.proteus.policy");
        assert!(
            !POLKIT_POLICY.contains("auth_admin_keep"),
            "polkit policy uses auth_admin_keep on a mutating action (issue #133)"
        );
    }
}
