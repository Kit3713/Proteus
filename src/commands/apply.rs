// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus apply` orchestrator.
//!
//! Composes per-module apply paths into a single, idempotent run. Iterates
//! components in dependency order, captures one outcome per feature, then
//! emits a single summary. Exit code is non-zero if any component failed.
//!
//! Modules that haven't landed yet (DHCP/DNS/stack/nft) are surfaced as
//! `not yet implemented` so the orchestrator stays forward-compatible while
//! phase-D work lands incrementally.
//!
//! Per-component machinery is not duplicated — each call delegates to the
//! same in-process function the corresponding subcommand uses
//! (`commands::rotate::run`, `commands::hostname::rotate`,
//! `commands::bluetooth_cmd::apply`). Each prints its own detailed output;
//! the orchestrator adds the cross-feature summary.

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
    if !yes {
        eprintln!("proteus: 'apply' is mutating; pass --yes to confirm (see `proteus help apply`)");
        return Ok(exit::NOT_IMPLEMENTED);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }

    let cfg_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&cfg_path)?;
    let reports = orchestrate(&config, state_path, config_path);

    let tally = print_summary(&reports);
    if tally.failed > 0 {
        Ok(exit::GENERIC_ERROR)
    } else {
        Ok(exit::SUCCESS)
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
        // Phase-D-pending modules: surface a stable note so a future enable
        // on a configured system isn't a silent no-op.
        not_yet_implemented("dhcp"),
        not_yet_implemented("dns"),
        not_yet_implemented("stack"),
        not_yet_implemented("nft"),
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

fn not_yet_implemented(name: &'static str) -> ComponentReport {
    ComponentReport {
        name,
        status: Status::Skipped,
        note: "not yet implemented".into(),
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

    fn disabled_config() -> Config {
        let mut cfg = Config::default();
        cfg.mac.enabled = false;
        cfg.hostname.enabled = false;
        cfg.bluetooth.enabled = false;
        cfg.ipv6.enabled = false;
        cfg
    }

    #[test]
    fn all_disabled_yields_only_skipped_outcomes() {
        let cfg = disabled_config();
        let reports = orchestrate(&cfg, None, None);
        assert!(reports.iter().all(|r| r.status == Status::Skipped));
        let tally = Tally::from_reports(&reports);
        assert_eq!(tally.failed, 0);
        assert_eq!(tally.applied, 0);
        assert_eq!(tally.skipped, reports.len());
    }

    #[test]
    fn unimplemented_components_have_stable_note() {
        let cfg = disabled_config();
        let reports = orchestrate(&cfg, None, None);
        for name in ["dhcp", "dns", "stack", "nft"] {
            let r = reports.iter().find(|r| r.name == name).unwrap_or_else(|| {
                panic!("missing report for component '{name}'");
            });
            assert_eq!(r.status, Status::Skipped);
            assert_eq!(r.note, "not yet implemented");
        }
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
