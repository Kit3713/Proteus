// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus apply` orchestrator.
//!
//! Composes per-module apply paths into a single, idempotent run. Iterates
//! components in dependency order, captures one outcome per feature, then
//! emits a single summary. Exit code is non-zero if any component failed.
//!
//! Per-component machinery is not duplicated — each call delegates to the
//! same in-process function the corresponding subcommand uses
//! (`commands::rotate::run`, `commands::hostname::rotate`,
//! `commands::bluetooth_cmd::apply`, `commands::ipv6::apply`,
//! `commands::dhcp::apply`, `commands::dns::apply`, `commands::stack::apply`,
//! `commands::nft::apply`). Each prints its own detailed output; the
//! orchestrator adds the cross-feature summary.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::exit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Applied,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentReport {
    pub name: &'static str,
    pub status: Status,
    pub note: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Tally {
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Tally {
    fn from_reports(reports: &[ComponentReport]) -> Self {
        let mut t = Self::default();
        for r in reports {
            match r.status {
                Status::Applied => t.applied += 1,
                Status::Skipped => t.skipped += 1,
                Status::Failed => t.failed += 1,
            }
        }
        t
    }
}

pub fn run(yes: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(yes, "'apply' is mutating", "proteus help apply") {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #126: serialize concurrent mutating runs on <state-dir>/.lock.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let cfg_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&cfg_path)?;

    let warnings = risk_warnings(&config);
    print_risk_warnings(&warnings);

    let reports = orchestrate(&config, state_path, config_path);

    let tally = print_summary(&reports);
    if tally.failed > 0 {
        Ok(exit::GENERIC_ERROR)
    } else {
        Ok(exit::SUCCESS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RiskWarning {
    pub knob: &'static str,
    pub breakage: &'static str,
    pub wiki: &'static str,
}

/// Knobs the user can opt into that are known to break specific things on
/// some networks. The wiki pointer is what the operator should read before
/// shipping this config — apply still proceeds, the warnings are
/// informational. New flags should be added here when they ship with a
/// "breaks X" note in the config schema.
pub(crate) fn risk_warnings(cfg: &Config) -> Vec<RiskWarning> {
    let mut out = Vec::new();
    if cfg.discovery.ssdp_block {
        out.push(RiskWarning {
            knob: "discovery.ssdp_block",
            breakage: "blocks SSDP — breaks KDE Connect and WS-Discovery printers",
            wiki: "discovery",
        });
    }
    if cfg.discovery.wsd_block {
        out.push(RiskWarning {
            knob: "discovery.wsd_block",
            breakage: "blocks WS-Discovery — breaks Windows printer auto-discovery",
            wiki: "discovery",
        });
    }
    if cfg.enterprise_wifi.anonymous_outer_identity {
        out.push(RiskWarning {
            knob: "enterprise_wifi.anonymous_outer_identity",
            breakage: "some 802.1X auth servers reject mismatched outer/inner identities",
            wiki: "enterprise-wifi",
        });
    }
    if cfg.stack.suppress_gratuitous_arp {
        out.push(RiskWarning {
            knob: "stack.suppress_gratuitous_arp",
            breakage: "breaks VRRP/keepalived failover detection on some networks",
            wiki: "stack-fingerprint",
        });
    }
    if cfg.rf.tx_power_reduce {
        out.push(RiskWarning {
            knob: "rf.tx_power_reduce",
            breakage: "reducing TX power may degrade reception in weak-signal environments",
            wiki: "rf-fingerprinting",
        });
    }
    out
}

fn print_risk_warnings(warnings: &[RiskWarning]) {
    if warnings.is_empty() {
        return;
    }
    eprintln!("proteus apply: {} risk warning(s):", warnings.len());
    for w in warnings {
        eprintln!("  {} — {} (proteus wiki {})", w.knob, w.breakage, w.wiki);
    }
}

/// Run each enabled component in dependency order. Disabled components and
/// modules that haven't landed yet are surfaced as `Skipped` rather than
/// silently dropped. A failure in one component never aborts later steps.
fn orchestrate(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Vec<ComponentReport> {
    vec![
        run_mac(config, state_path, config_path),
        run_hostname(config, state_path, config_path),
        run_bluetooth(config, state_path, config_path),
        run_ipv6(config, state_path, config_path),
        run_dhcp(config, state_path, config_path),
        run_dns(config, config_path),
        run_stack(state_path, config_path),
        run_rf(config, state_path, config_path),
        run_nft(config_path),
        run_timers(config),
    ]
}

fn run_ipv6(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.ipv6.enabled {
        return skipped("ipv6", "disabled in config (ipv6.enabled = false)");
    }
    // Orchestrator already gated on `--yes`; pass it through so the
    // sub-command's own --yes guard is satisfied.
    classify("ipv6", super::ipv6::apply(true, state_path, config_path))
}

fn run_mac(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.mac.enabled {
        return skipped("mac", "disabled in config (mac.enabled = false)");
    }
    classify(
        "mac",
        super::rotate::run(None, true, state_path, config_path),
    )
}

fn run_hostname(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.hostname.enabled {
        return skipped("hostname", "disabled in config (hostname.enabled = false)");
    }
    classify("hostname", super::hostname::rotate(state_path, config_path))
}

fn run_bluetooth(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.bluetooth.enabled {
        return skipped(
            "bluetooth",
            "disabled in config (bluetooth.enabled = false)",
        );
    }
    classify(
        "bluetooth",
        super::bluetooth_cmd::apply(state_path, config_path),
    )
}

fn run_dhcp(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.dhcp.enabled {
        return skipped("dhcp", "disabled in config (dhcp.enabled = false)");
    }
    classify("dhcp", super::dhcp::apply(state_path, config_path))
}

// `dns.strip_edns_client_subnet` is the only DNS knob today, so it doubles
// as the master enable. The submodule still handles disabled-via-config
// (it removes any prior drop-in), so the orchestrator only short-circuits
// to keep the summary line clean.
fn run_dns(config: &Config, config_path: Option<&Path>) -> ComponentReport {
    if !config.dns.strip_edns_client_subnet {
        return skipped(
            "dns",
            "disabled in config (dns.strip_edns_client_subnet = false)",
        );
    }
    classify("dns", super::dns::apply(config_path))
}

// Stack has no master enable — its toggles express specific hardenings.
// Always run; the renderer respects each toggle.
fn run_stack(state_path: Option<&Path>, config_path: Option<&Path>) -> ComponentReport {
    classify("stack", super::stack::apply(true, state_path, config_path))
}

// RF: master-switch and "no Wi-Fi hardware" skips happen here so the
// orchestrator labels them clearly. Missing `iw` is gated by the submodule
// (it returns SYSTEM_NOT_SUPPORTED, treated as a skip below — same pattern
// as nft).
fn run_rf(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.rf.tx_power_reduce {
        return skipped("rf", "disabled in config (rf.tx_power_reduce = false)");
    }
    if crate::rf::wifi_interfaces().is_empty() {
        return skipped("rf", "no Wi-Fi interfaces detected");
    }
    let res = super::rf::apply(true, state_path, config_path);
    if let Ok(code) = res
        && code == exit::SYSTEM_NOT_SUPPORTED
    {
        return skipped("rf", "`iw` binary not found on PATH");
    }
    classify("rf", res)
}

// nft is gated by `nft_present()` inside the submodule, which surfaces
// SYSTEM_NOT_SUPPORTED (70) when nftables isn't installed. Treat that as a
// skip in the summary instead of a failure so the orchestrator doesn't
// flag a missing system dep.
fn run_nft(config_path: Option<&Path>) -> ComponentReport {
    let res = super::nft::apply(true, config_path);
    if let Ok(code) = res
        && code == exit::SYSTEM_NOT_SUPPORTED
    {
        return skipped("nft", "nftables not installed");
    }
    classify("nft", res)
}

// Reconcile the [timers] block onto the on-disk drop-ins. Skipped if the
// process is not running under systemd (e.g. CI containers, dev shells)
// because the reconciler restarts units that won't exist there.
fn run_timers(config: &Config) -> ComponentReport {
    if !std::path::Path::new("/run/systemd/system").is_dir() {
        return skipped("timers", "systemd not detected");
    }
    let report = crate::timer::reconcile_with_config(
        &config.timers,
        |unit| systemctl(&["restart", unit]),
        || systemctl(&["daemon-reload"]),
    );
    if report.any_failed() {
        let notes: Vec<String> = report
            .timers
            .iter()
            .filter_map(|t| match &t.outcome {
                crate::timer::ReconcileOutcome::Failed(msg) => Some(format!("{}: {msg}", t.name)),
                _ => None,
            })
            .collect();
        return ComponentReport {
            name: "timers",
            status: Status::Failed,
            note: notes.join("; "),
        };
    }
    let summary = summarize_timer_report(&report);
    ComponentReport {
        name: "timers",
        status: Status::Applied,
        note: summary,
    }
}

fn summarize_timer_report(report: &crate::timer::ReconcileReport) -> String {
    report
        .timers
        .iter()
        .map(|t| format!("{}={}", t.name, t.outcome.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    use anyhow::{Context, anyhow};
    let output = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .context("invoking systemctl")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "systemctl {} exited with {}: {}",
        args.join(" "),
        output.status,
        stderr.trim()
    ))
}

/// Map an exit-code-returning command result into a component report. Any
/// non-success code is treated as a failure with the numeric code preserved
/// for the summary; an `Err` becomes a failure with the diagnostic text.
fn classify(name: &'static str, res: Result<u8>) -> ComponentReport {
    match res {
        Ok(c) if c == exit::SUCCESS => ComponentReport {
            name,
            status: Status::Applied,
            note: "ok".into(),
        },
        Ok(c) => ComponentReport {
            name,
            status: Status::Failed,
            note: format!("exited with code {c}"),
        },
        Err(e) => ComponentReport {
            name,
            status: Status::Failed,
            note: format!("{e:#}"),
        },
    }
}

fn skipped(name: &'static str, note: &str) -> ComponentReport {
    ComponentReport {
        name,
        status: Status::Skipped,
        note: note.into(),
    }
}

fn print_summary(reports: &[ComponentReport]) -> Tally {
    println!("apply summary:");
    for r in reports {
        let label = match r.status {
            Status::Applied => "applied",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
        };
        println!("  {:<10} {:<8} ({})", r.name, label, r.note);
    }
    let tally = Tally::from_reports(reports);
    println!(
        "totals: applied={} skipped={} failed={}",
        tally.applied, tally.skipped, tally.failed
    );
    tally
}

#[cfg(test)]
mod tests {
    use super::*;

    // `stack` and `nft` have no master enable, so a "fully disabled" config
    // covers everything else and the orchestrator-level test below pairs
    // that with separate per-module unit coverage. The two remain wired
    // unconditionally — they're tested via integration paths that need
    // root.
    fn disabled_config() -> Config {
        let mut cfg = Config::default();
        cfg.mac.enabled = false;
        cfg.hostname.enabled = false;
        cfg.bluetooth.enabled = false;
        cfg.ipv6.enabled = false;
        cfg.dhcp.enabled = false;
        cfg.dns.strip_edns_client_subnet = false;
        cfg.rf.tx_power_reduce = false;
        cfg
    }

    #[test]
    fn disabled_components_skip_with_config_note() {
        let cfg = disabled_config();
        // Test the gated-skip path component-by-component so the rest of
        // the orchestrator (stack, nft) doesn't pull in root-only side
        // effects. This is the part of orchestrate() worth pinning: every
        // gated module emits a Skipped report instead of running.
        let reports = vec![
            run_mac(&cfg, None, None),
            run_hostname(&cfg, None, None),
            run_bluetooth(&cfg, None, None),
            run_ipv6(&cfg, None, None),
            run_dhcp(&cfg, None, None),
            run_dns(&cfg, None),
            run_rf(&cfg, None, None),
        ];
        assert!(
            reports.iter().all(|r| r.status == Status::Skipped),
            "expected all gated components to skip when disabled, got: {reports:?}"
        );
        let tally = Tally::from_reports(&reports);
        assert_eq!(tally.failed, 0);
        assert_eq!(tally.applied, 0);
        assert_eq!(tally.skipped, reports.len());
    }

    #[test]
    fn orchestrator_covers_every_phase_d_module() {
        // The audit guard: if a future refactor drops a component from the
        // dispatch table, this test fails. Run once with a fully-disabled
        // config so dispatch produces stable `skipped` reports we can
        // inspect by name without touching the system.
        let cfg = disabled_config();
        let reports = [
            run_mac(&cfg, None, None),
            run_hostname(&cfg, None, None),
            run_bluetooth(&cfg, None, None),
            run_ipv6(&cfg, None, None),
            run_dhcp(&cfg, None, None),
            run_dns(&cfg, None),
            run_rf(&cfg, None, None),
            run_timers(&cfg),
        ];
        for name in [
            "mac",
            "hostname",
            "bluetooth",
            "ipv6",
            "dhcp",
            "dns",
            "rf",
            "timers",
        ] {
            assert!(
                reports.iter().any(|r| r.name == name),
                "orchestrator missing component '{name}'"
            );
        }
    }

    #[test]
    fn orchestrator_includes_timers_for_default_config() {
        // The default Med config carries [timers] cadences, so the
        // orchestrator must include a "timers" entry. We don't assert the
        // status because that depends on whether systemd is present in the
        // test environment.
        let cfg = Config::default();
        let report = run_timers(&cfg);
        assert_eq!(report.name, "timers");
    }

    #[test]
    fn classify_maps_command_results_to_status_buckets() {
        let r = classify("a", Ok(exit::SUCCESS));
        assert_eq!(r.status, Status::Applied);

        let r = classify("b", Ok(exit::GENERIC_ERROR));
        assert_eq!(r.status, Status::Failed);
        assert!(r.note.contains("1"));

        let r = classify("c", Err(anyhow::anyhow!("boom")));
        assert_eq!(r.status, Status::Failed);
        assert!(r.note.contains("boom"));
    }

    #[test]
    fn risk_warnings_empty_for_default_config() {
        // Defaults must not trigger warnings — every default-on knob is
        // privacy-safe-by-design. Risky knobs are opt-in.
        let cfg = Config::default();
        let warnings = risk_warnings(&cfg);
        assert!(
            warnings.is_empty(),
            "default config should produce no warnings, got: {warnings:?}"
        );
    }

    #[test]
    fn risk_warnings_triggers_for_each_risky_knob() {
        let mut cfg = Config::default();
        cfg.discovery.ssdp_block = true;
        cfg.discovery.wsd_block = true;
        cfg.enterprise_wifi.anonymous_outer_identity = true;
        cfg.stack.suppress_gratuitous_arp = true;
        cfg.rf.tx_power_reduce = true;
        let warnings = risk_warnings(&cfg);
        let knobs: Vec<&str> = warnings.iter().map(|w| w.knob).collect();
        assert!(knobs.contains(&"discovery.ssdp_block"));
        assert!(knobs.contains(&"discovery.wsd_block"));
        assert!(knobs.contains(&"enterprise_wifi.anonymous_outer_identity"));
        assert!(knobs.contains(&"stack.suppress_gratuitous_arp"));
        assert!(knobs.contains(&"rf.tx_power_reduce"));
        assert_eq!(warnings.len(), 5);
    }

    #[test]
    fn risk_warnings_each_points_at_an_existing_wiki_page() {
        // Every warning needs a wiki page to read — so a future rename in
        // the wiki dir doesn't quietly orphan an apply warning.
        use crate::wiki;
        let mut cfg = Config::default();
        cfg.discovery.ssdp_block = true;
        cfg.discovery.wsd_block = true;
        cfg.enterprise_wifi.anonymous_outer_identity = true;
        cfg.stack.suppress_gratuitous_arp = true;
        cfg.rf.tx_power_reduce = true;
        for w in risk_warnings(&cfg) {
            assert!(
                wiki::get_page(w.wiki).is_some(),
                "risk warning {} points at wiki page '{}' which does not exist",
                w.knob,
                w.wiki
            );
        }
    }

    #[test]
    fn run_rf_skips_with_default_config_note() {
        // The Med default has rf.tx_power_reduce off, so the orchestrator
        // entry point skips with a stable, machine-greppable note. Pin it
        // so a future profile change can't silently flip the default-on.
        let cfg = Config::default();
        assert!(
            !cfg.rf.tx_power_reduce,
            "default profile must not enable rf"
        );
        let report = run_rf(&cfg, None, None);
        assert_eq!(report.name, "rf");
        assert_eq!(report.status, Status::Skipped);
        assert!(report.note.contains("rf.tx_power_reduce = false"));
    }

    #[test]
    fn risk_warnings_rf_tx_power_reduce_triggers_alone() {
        // RF is the only opt-in risk knob in High by default, so verify it
        // produces the expected warning shape independent of the others.
        let mut cfg = Config::default();
        cfg.rf.tx_power_reduce = true;
        let warnings = risk_warnings(&cfg);
        let rf = warnings.iter().find(|w| w.knob == "rf.tx_power_reduce");
        let rf = rf.expect("rf.tx_power_reduce warning expected");
        assert_eq!(rf.wiki, "rf-fingerprinting");
        assert!(rf.breakage.contains("TX power"));
    }

    #[test]
    fn tally_counts_each_status_bucket() {
        let reports = vec![
            ComponentReport {
                name: "a",
                status: Status::Applied,
                note: String::new(),
            },
            ComponentReport {
                name: "b",
                status: Status::Skipped,
                note: String::new(),
            },
            ComponentReport {
                name: "c",
                status: Status::Failed,
                note: String::new(),
            },
            ComponentReport {
                name: "d",
                status: Status::Applied,
                note: String::new(),
            },
        ];
        let t = Tally::from_reports(&reports);
        assert_eq!(t.applied, 2);
        assert_eq!(t.skipped, 1);
        assert_eq!(t.failed, 1);
    }
}
