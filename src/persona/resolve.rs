// SPDX-License-Identifier: GPL-3.0-or-later

//! Single-source-of-truth persona resolver for the apply / rotate paths
//! (roadmap Milestone 2 "Integration").
//!
//! Every integration site — MAC OUI shaping in `mac::generator`, hostname
//! template rendering in `hostname`, DHCP-fingerprint write in
//! `nm::dhcp`, Bluetooth alias in `bluetooth::apply` — funnels through
//! [`active_for`]. Centralising the precedence rules here means future
//! layers (e.g. interface-scoped overrides) only need to grow this
//! function, not every consumer.
//!
//! Precedence chain (highest first):
//!
//!   1. `[per_ssid."<ssid>"].persona` when an `ssid` is supplied.
//!   2. `[persona].active`.
//!
//! When neither layer carries a persona id, [`active_for`] returns
//! `None` so consumers fall through to the v0.2.x `Profile`-slider path.
//! When an id is set but the file can't be loaded, the function
//! warn-logs and returns `None` rather than failing the apply: the
//! profile slider is the safe default and we surface the load error in
//! `proteus persona current`.

use std::path::Path;

use super::{Persona, load};
use crate::config::Config;

/// Resolve the persona that should shape this apply / rotate.
///
/// `ssid` is `Some(...)` on the connection-up path (Milestone 3 wiring)
/// and `None` for global apply/rotate. `user_root` points at
/// `/etc/proteus/personas/` in production; tests pass a `TempDir`.
pub fn active_for(config: &Config, ssid: Option<&str>, user_root: &Path) -> Option<Persona> {
    let id = pick_id(config, ssid)?;
    match load::load(&id, user_root) {
        Ok(Some((p, _src))) => Some(p),
        Ok(None) => {
            // Active id set but the file isn't on disk and isn't an
            // embedded built-in. Don't fail apply — just fall through.
            tracing::warn!(
                persona_id = %id,
                "persona '{id}' set in config but not found on disk or in built-in catalogue; \
                 falling back to plain randomizer mode"
            );
            None
        }
        Err(e) => {
            // Parse / schema error. Same treatment: warn-log + fall through
            // so a hand-edited bad file doesn't brick `proteus apply`.
            tracing::warn!(
                persona_id = %id,
                error = %format!("{e:#}"),
                "failed to load persona '{id}'; falling back to plain randomizer mode"
            );
            None
        }
    }
}

/// Pick the persona id to use, honouring per-SSID > global precedence.
/// Lifted out of `active_for` so the resolution logic is testable
/// without touching the loader.
fn pick_id(config: &Config, ssid: Option<&str>) -> Option<String> {
    if let Some(s) = ssid
        && let Some(per) = config.per_ssid.get(s)
        && let Some(id) = &per.persona
    {
        return Some(id.clone());
    }
    config.persona.active.clone()
}

/// Default user-persona root, exposed so call sites that don't already
/// import `persona::load` can grab it from one place.
pub fn default_user_root() -> &'static Path {
    Path::new(load::DEFAULT_USER_ROOT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PerSsidPolicy;
    use crate::profile::Profile;
    use std::path::PathBuf;

    fn cfg() -> Config {
        let mut c = Profile::Med.baseline();
        c.per_ssid.clear();
        c.persona.active = None;
        c
    }

    #[test]
    fn pick_id_returns_none_when_no_layer_sets_one() {
        let c = cfg();
        assert_eq!(pick_id(&c, None), None);
        assert_eq!(pick_id(&c, Some("home")), None);
    }

    #[test]
    fn pick_id_returns_global_when_no_per_ssid_match() {
        let mut c = cfg();
        c.persona.active = Some("iphone-15".into());
        assert_eq!(pick_id(&c, None), Some("iphone-15".into()));
        assert_eq!(pick_id(&c, Some("any-ssid")), Some("iphone-15".into()));
    }

    #[test]
    fn pick_id_per_ssid_beats_global() {
        let mut c = cfg();
        c.persona.active = Some("iphone-15".into());
        c.per_ssid.insert(
            "coffee".into(),
            PerSsidPolicy {
                persona: Some("pixel-8".into()),
                ..PerSsidPolicy::default()
            },
        );
        assert_eq!(pick_id(&c, Some("coffee")), Some("pixel-8".into()));
        // SSID match is exact — unrelated SSIDs still see the global.
        assert_eq!(pick_id(&c, Some("home")), Some("iphone-15".into()));
    }

    #[test]
    fn pick_id_per_ssid_without_persona_falls_back_to_global() {
        // A per-SSID block can override pin_mac without overriding persona.
        let mut c = cfg();
        c.persona.active = Some("iphone-15".into());
        c.per_ssid.insert(
            "home".into(),
            PerSsidPolicy {
                pin_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                ..PerSsidPolicy::default()
            },
        );
        assert_eq!(pick_id(&c, Some("home")), Some("iphone-15".into()));
    }

    #[test]
    fn active_for_with_no_persona_returns_none() {
        let c = cfg();
        let nothing = PathBuf::from("/nope");
        assert!(active_for(&c, None, &nothing).is_none());
    }

    #[test]
    fn active_for_loads_a_known_builtin_persona() {
        let mut c = cfg();
        c.persona.active = Some("iphone-15".into());
        let nothing = PathBuf::from("/nope/proteus-personas-x");
        let p = active_for(&c, None, &nothing).expect("iphone-15 must load");
        assert_eq!(p.id, "iphone-15");
    }

    #[test]
    fn active_for_unknown_id_warns_and_returns_none() {
        let mut c = cfg();
        c.persona.active = Some("definitely-not-a-real-persona-xyz".into());
        let nothing = PathBuf::from("/nope/proteus-personas-x");
        assert!(active_for(&c, None, &nothing).is_none());
    }

    /// Acceptance test from the M2 integration spec: load a tempdir
    /// config that pins `[persona] active = "iphone-15"`, drive the
    /// probe-aware generator with a `MockProbe`, and assert the chosen
    /// MAC's OUI is one of the Apple ranges.
    #[test]
    fn iphone_15_persona_drives_generator_to_apple_oui_e2e() {
        use crate::config::{Config, RawConfig};
        use crate::mac::generator::{self, GenerateOptions, ProbeOptions};
        use crate::mac::oui::APPLE;
        use crate::mac::probe::MockProbe;
        use std::collections::HashSet;

        let dir = std::env::temp_dir().join(format!("proteus-persona-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(
            &cfg_path,
            "profile = \"med\"\n[persona]\nactive = \"iphone-15\"\n",
        )
        .unwrap();

        // Re-load through the config pipeline so this exercises the
        // end-to-end path.
        let raw_str = std::fs::read_to_string(&cfg_path).unwrap();
        let raw: RawConfig = toml::from_str(&raw_str).unwrap();
        let cfg: Config = raw.resolve();
        assert_eq!(cfg.persona.active.as_deref(), Some("iphone-15"));

        let p = active_for(&cfg, None, default_user_root()).expect("iphone-15 must load");
        assert_eq!(p.id, "iphone-15");
        assert_eq!(p.oui_pool, vec!["apple".to_string()]);

        // Drive the probe-aware generator with the persona pool. The
        // MockProbe defaults to "Free", so every candidate is accepted
        // immediately — what we're pinning is the OUI choice, not the
        // collision-handling.
        let pool = p.oui_pool.clone();
        let forbidden: HashSet<crate::mac::Mac> = HashSet::new();
        let avoid: HashSet<crate::mac::Mac> = HashSet::new();
        let probe = MockProbe::responds(false);
        let opts = GenerateOptions {
            pool: &pool,
            forbidden: &forbidden,
            avoid: &avoid,
            suffix_pattern: None,
        };
        let mut probe_opts = ProbeOptions::for_iface("wlan0");
        probe_opts.run_nd_probe = false;
        let outcome = generator::generate_with_probe(&opts, &probe, &probe_opts).expect("ok");
        let oui = &outcome.chosen.octets()[..3];
        assert!(
            APPLE.iter().any(|p| p.as_slice() == oui),
            "iphone-15 persona must yield an Apple-OUI MAC; got {}",
            outcome.chosen
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_for_per_ssid_override_picks_pixel_8_on_coffee_ssid() {
        let mut c = cfg();
        c.persona.active = Some("iphone-15".into());
        c.per_ssid.insert(
            "coffee".into(),
            PerSsidPolicy {
                persona: Some("pixel-8".into()),
                ..PerSsidPolicy::default()
            },
        );
        let nothing = PathBuf::from("/nope/proteus-personas-x");
        let p = active_for(&c, Some("coffee"), &nothing).expect("pixel-8");
        assert_eq!(p.id, "pixel-8");
    }
}
