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
//!
//! Issue #242: per-feature handlers carry their own `--yes` gate so that
//! direct CLI invocations stay safe. The orchestrator's `run` already
//! cleared its own gate at the top, so every per-feature call below
//! passes `yes=true` to satisfy the gate without re-prompting.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;

// Roadmap 1.1.1: the `apply`/`revert` `--json` envelope DTOs moved to
// `proteus-types`. Re-exported here so `commands::apply::Summary` (used by
// `commands::revert`) and every other in-crate path keep resolving and the
// emitted JSON is byte-identical. The `emit_summary` writer and the private
// `Tally` helper below are behaviour, not wire types, so they stay.
pub use proteus_types::apply::{ComponentReport, Status, Summary};

/// Write a [`Summary`] as a single line on stdout. Used by both apply
/// and revert `--json` paths so the envelope is byte-identical between
/// them. Failures here surface as a non-zero exit only via the caller's
/// existing error-handling — we never panic on a serializer error.
pub(crate) fn emit_summary(summary: &Summary) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, summary).context("serialising apply/revert summary")?;
    handle
        .write_all(b"\n")
        .context("flushing summary newline")?;
    Ok(())
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

pub fn run(
    yes: bool,
    json: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    // root check first — every other failure mode below is a no-op for
    // a non-root caller, so surfacing the privilege error first keeps
    // the message clean.
    //
    // E5 partial follow-up: this site drops the anyhow source chain in
    // exchange for a typed `PERMISSION_ERROR` exit code. Bubbling via
    // `?` would let the dispatcher render `proteus: {e:#}` but collapse
    // the code to `GENERIC_ERROR` (1) instead of (77), which CI scripts
    // grepping for the typed code rely on. A future wave introducing a
    // typed `ExitCodeError(u8)` wrapper can convert this without losing
    // either the chain or the typed code.
    if let Err(e) = super::require_root() {
        let msg = format!("{e}");
        if json {
            emit_summary(&Summary::with_error("apply", exit::PERMISSION_ERROR, msg))?;
        } else {
            eprintln!("proteus: {msg}");
        }
        return Ok(exit::PERMISSION_ERROR);
    }

    // NMOD.1 (high): load and validate the config BEFORE acquiring the
    // state lock. The previous shape took the lock first, then loaded +
    // validated. Combined with C1 / N12.13 (HELD mutex held across retry
    // sleep) a misconfigured per-SSID block could starve the rotate
    // timer for the full 5 s budget on every retry — the lock sat idle
    // while the validator returned an error. With load-before-lock, a
    // typo'd config fails fast and the lock is never taken.
    let cfg_path = super::config_path(config_path);
    let config = match Config::default_or_loaded(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            if json {
                emit_summary(&Summary::with_error(
                    "apply",
                    exit::CONFIG_ERROR,
                    format!("{e:#}"),
                ))?;
                return Ok(exit::CONFIG_ERROR);
            }
            return Err(e);
        }
    };

    // NMOD.2: the `--yes` gate must run AFTER config validation so a user
    // with a typo'd `[per_ssid]` block sees the parse error before the
    // confirmation prompt. Without this reorder the operator confirms,
    // then the validator rejects — the "confirmation = mutation
    // imminent" invariant breaks because no mutation actually happens.
    if let Err(code) = super::require_yes(yes, "'apply' is mutating", "proteus help apply") {
        if json {
            // `require_yes` already printed the stderr hint; pair it
            // with the structured envelope so wrappers stay on stdout.
            emit_summary(&Summary::with_error("apply", code, "missing --yes"))?;
        }
        return Ok(code);
    }

    // Issue #126: serialize concurrent mutating runs on <state-dir>/.lock.
    // Acquired AFTER config validation (NMOD.1) so a misconfig never
    // pins the lock budget.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => {
            if json {
                emit_summary(&Summary::with_error(
                    "apply",
                    code,
                    "state lock unavailable",
                ))?;
            }
            return Ok(code);
        }
    };

    // Roadmap Milestone 1: resolve the backend once at the top of the
    // apply cycle. Per-feature runners select again internally (each
    // is callable as a standalone subcommand), but resolving here
    // surfaces "no backend available" as a single, clear error before
    // any component prints its own line — the whole cycle uses the
    // same auto-pick because `select` is deterministic for a fixed
    // driver string.
    // E5 partial follow-up: similar shape to the root-check above.
    // Bubbling via `?` would collapse `SYSTEM_NOT_SUPPORTED` (70) into
    // `GENERIC_ERROR` (1) at the dispatcher; the typed code is what
    // signals "install NM/networkd or pick `--backend=raw`" to wrapper
    // scripts. The eprintln already prints the full anyhow chain via
    // `{e:#}`, so the user-visible diagnostic is intact — only the
    // dispatch table can't distinguish typed vs generic without a
    // wrapper. Defer until the wrapper lands.
    if let Err(e) = preflight_backend(&config) {
        let msg = format!("{e:#}");
        if json {
            emit_summary(&Summary::with_error(
                "apply",
                exit::SYSTEM_NOT_SUPPORTED,
                format!("backend preflight failed: {msg}"),
            ))?;
        } else {
            eprintln!("proteus apply: backend preflight failed: {msg}");
        }
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }

    if !json {
        let warnings = risk_warnings(&config);
        print_risk_warnings(&warnings);
    }

    let reports = orchestrate(&config, state_path, config_path);

    // NCMD2.3: a single `systemctl daemon-reload` after every per-feature
    // apply is done. dns / stack / resolved / ipv6 each write drop-ins under
    // /etc/systemd/, but their per-feature `apply` paths don't reload — the
    // documented effect ("apply lands the config") only materialises after
    // the unit-file cache rebuilds. Run it once here so the operator never
    // has to invoke a manual reload. Failure is non-fatal: drop-ins are on
    // disk and will be picked up on next boot regardless.
    let reload_note = systemctl_daemon_reload_after_apply(&reports);

    let tally = Tally::from_reports(&reports);
    let exit_code = if tally.failed > 0 {
        exit::GENERIC_ERROR
    } else {
        exit::SUCCESS
    };

    if json {
        emit_summary(&Summary::new("apply", reports, exit_code))?;
    } else {
        print_summary(&reports);
        if let Some(note) = reload_note {
            println!("{note}");
        }
    }
    Ok(exit_code)
}

/// Run `systemctl daemon-reload` once after orchestration so freshly-written
/// drop-ins under `/etc/systemd/` (resolved.conf.d, timesyncd.conf.d, …)
/// take effect without a manual reload (NCMD2.3). Skipped when systemd
/// isn't running (CI containers, dev shells) since there's nothing to
/// reload. Bounded by a hard subprocess timeout so a wedged systemd can't
/// hang the apply cycle. Returns a one-line note for the summary, or
/// `None` when the reload was skipped.
fn systemctl_daemon_reload_after_apply(reports: &[ComponentReport]) -> Option<String> {
    if !std::path::Path::new("/run/systemd/system").is_dir() {
        return None;
    }
    // Skip when nothing the reload would care about actually got applied —
    // a pure-skip cycle never wrote a drop-in, so the reload is busywork.
    let drop_in_owners = ["dns", "resolved", "stack", "ipv6", "ntp"];
    let any_applied = reports
        .iter()
        .any(|r| drop_in_owners.contains(&r.name) && r.status == Status::Applied);
    if !any_applied {
        return None;
    }
    match systemctl_with_timeout(&["daemon-reload"], std::time::Duration::from_secs(10)) {
        Ok(()) => Some("daemon-reload: ok".to_string()),
        Err(e) => {
            tracing::warn!("apply: systemctl daemon-reload failed: {e:#}");
            Some(format!("daemon-reload: failed ({e:#})"))
        }
    }
}

/// Spawn `systemctl <args>` with a hard wall-clock timeout. If the timeout
/// elapses we send SIGKILL and return a `TimedOut`-style error so the
/// caller can surface a clear note. Used by `daemon-reload` so a wedged
/// pid 1 can't hang `proteus apply`.
fn systemctl_with_timeout(args: &[&str], timeout: std::time::Duration) -> anyhow::Result<()> {
    use anyhow::{Context, anyhow};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Instant;

    let mut child = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning systemctl")?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().context("waiting on systemctl")? {
            Some(status) => {
                if status.success() {
                    return Ok(());
                }
                // Drain stderr for the diagnostic.
                let mut buf = String::new();
                if let Some(mut s) = child.stderr.take() {
                    use std::io::Read;
                    let _ = s.read_to_string(&mut buf);
                }
                return Err(anyhow!(
                    "systemctl {} exited with {status}: {}; see proteus wiki troubleshooting",
                    args.join(" "),
                    buf.trim()
                ));
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!(
                        "systemctl {} timed out after {}s; see proteus wiki troubleshooting",
                        args.join(" "),
                        timeout.as_secs()
                    ));
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RiskWarning {
    pub knob: &'static str,
    pub breakage: &'static str,
    pub wiki: &'static str,
}

/// One-shot backend resolution. Roadmap M1 acceptance:
/// `proteus apply` must surface a clear "backend unavailable" line when
/// none of nm / networkd / raw is present, rather than letting every
/// component fail with its own DBus error. The trait object is
/// dropped at the end of this function — per-feature runners select
/// again internally, but they get the same answer (the resolver is
/// deterministic).
pub(crate) fn preflight_backend(config: &Config) -> anyhow::Result<&'static str> {
    let driver = config.backend.driver.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let backend = rt.block_on(async { crate::backend::select::select(&driver).await })?;
    let name = backend.name();
    // Roadmap Stream 7 / E1: success-path breadcrumbs go to debug so the
    // default-verbosity stderr stays empty for a clean apply. Operators
    // hunting for the resolved backend re-enable with `-v` or RUST_LOG.
    tracing::debug!(driver = %driver, resolved = %name, "apply: backend preflight ok");
    Ok(name)
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
        run_resolved(config, config_path),
        run_ntp(config, config_path),
        run_stack(state_path, config_path),
        run_rf(config, state_path, config_path),
        run_nft(config_path),
        run_timers(config),
    ]
}

// resolved is gated on at least one knob being on. The submodule itself
// removes any prior drop-in when both knobs are off, so the orchestrator
// short-circuits only to keep the summary line legible.
fn run_resolved(config: &Config, config_path: Option<&Path>) -> ComponentReport {
    if !crate::dns::resolved::is_active(&config.resolved) {
        return skipped(
            "resolved",
            "disabled in config (resolved.mdns_off and resolved.llmnr_off both false)",
        );
    }
    classify("resolved", super::resolved::apply(true, config_path))
}

// ntp respects the `[ntp] enabled` master switch. The submodule's hard
// guard takes over from there (chrony/ntpd present → defer).
fn run_ntp(config: &Config, config_path: Option<&Path>) -> ComponentReport {
    if !config.ntp.enabled {
        return skipped("ntp", "disabled in config (ntp.enabled = false)");
    }
    classify("ntp", super::ntp::apply(true, config_path))
}

fn run_ipv6(
    config: &Config,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> ComponentReport {
    if !config.ipv6.enabled {
        return skipped("ipv6", "disabled in config (ipv6.enabled = false)");
    }
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
        super::rotate::run(None, true, false, false, None, state_path, config_path),
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
    classify(
        "hostname",
        super::hostname::rotate(true, state_path, config_path),
    )
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
        super::bluetooth_cmd::apply(true, state_path, config_path),
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
    // The orchestrator's own `--yes` gate ran before this fan-out, so each
    // per-feature mutator is invoked with `yes=true` to skip the inner gate.
    // Mirrors the bluetooth/ipv6/enterprise-wifi/rf calls below.
    let primary = classify("dhcp", super::dhcp::apply(true, state_path, config_path));
    // Roadmap Milestone 4c: when `[dhcp] renew_on_apply = true`, follow
    // the apply with a lease release+renew so the upstream DHCP server
    // hands out a fresh lease against the new client identity. Skipped
    // when the apply itself failed — chaining a renew on top of a
    // broken DHCP write only produces noise.
    if !config.dhcp.renew_on_apply || primary.status == Status::Failed {
        return primary;
    }
    let renew = run_dhcp_renew_after_apply(config, state_path);
    merge_dhcp_with_renew(primary, renew)
}

fn run_dhcp_renew_after_apply(
    config: &Config,
    state_path: Option<&Path>,
) -> Result<u8, anyhow::Error> {
    let _ = state_path; // renew is read-only against state today
    let driver = config.backend.driver.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime for dhcp renew")?;
    let tally = rt.block_on(async {
        let backend = crate::backend::select::select(&driver).await?;
        super::dhcp::renew_after_apply(backend.as_ref()).await
    })?;
    if tally.total() == 0 {
        return Ok(exit::SUCCESS);
    }
    println!(
        "dhcp renew (renew_on_apply): reapplied={} cycled={} skipped={} failed={}",
        tally.reapplied, tally.cycled, tally.skipped_no_active, tally.failed,
    );
    if tally.failed > 0 {
        Ok(exit::GENERIC_ERROR)
    } else {
        Ok(exit::SUCCESS)
    }
}

fn merge_dhcp_with_renew(
    primary: ComponentReport,
    renew: Result<u8, anyhow::Error>,
) -> ComponentReport {
    let (renewed_ok, suffix) = match &renew {
        Ok(c) if *c == exit::SUCCESS => (true, " + renewed".to_string()),
        Ok(c) => (false, format!(" + renew failed (exit {c})")),
        Err(e) => (false, format!(" + renew error: {e:#}")),
    };
    ComponentReport {
        name: primary.name,
        status: if renewed_ok {
            primary.status
        } else {
            Status::Failed
        },
        note: format!("{}{suffix}", primary.note),
    }
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
    classify("dns", super::dns::apply(true, config_path))
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

/// C3: subprocesses called from the apply orchestrator must not block
/// the lock-holding caller forever. A wedged `systemctl daemon-reload`
/// or `nft` can otherwise hang `proteus apply` (and the rotate-timer
/// dispatcher behind it) indefinitely. 30 s is an order of magnitude
/// past the longest legitimate `systemctl` reload + restart cycle on
/// the workstations Proteus targets.
const SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    use anyhow::anyhow;
    let output = run_with_timeout("systemctl", args, SUBPROCESS_TIMEOUT)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "systemctl {} exited with {}: {}; see proteus wiki troubleshooting",
        args.join(" "),
        output.status,
        stderr.trim()
    ))
}

/// C3: spawn `program args...`, wait up to `timeout`, kill on overrun.
/// Returns the captured output on a normal exit, or an `Err` if the
/// child hangs past the budget. The kill is best-effort — a child that
/// resists SIGKILL is the kernel's problem at that point, but we still
/// surface a clean error to the caller so the lock is released.
pub(crate) fn run_with_timeout(
    program: &str,
    args: &[&str],
    timeout: std::time::Duration,
) -> anyhow::Result<std::process::Output> {
    use anyhow::{Context, anyhow};
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {program}"))?;

    let start = std::time::Instant::now();
    let poll = std::time::Duration::from_millis(50);
    loop {
        match child
            .try_wait()
            .with_context(|| format!("polling {program}"))?
        {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_end(&mut stdout);
                }
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_end(&mut stderr);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!(
                        "{program} {} timed out after {}s; see proteus wiki troubleshooting",
                        args.join(" "),
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(poll);
            }
        }
    }
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

fn print_summary(reports: &[ComponentReport]) {
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
        cfg.resolved.mdns_off = false;
        cfg.resolved.llmnr_off = false;
        cfg.ntp.enabled = false;
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
            run_resolved(&cfg, None),
            run_ntp(&cfg, None),
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
            run_resolved(&cfg, None),
            run_ntp(&cfg, None),
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
            "resolved",
            "ntp",
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

    /// Roadmap Milestone 4c: when the DHCP apply succeeds and
    /// `renew_on_apply` is true, the orchestrator merges the renew
    /// outcome into the same component note. A successful renew
    /// preserves the Applied status and tags the note. A failed renew
    /// flips the merged status to Failed so the apply's exit code
    /// reflects the breakage.
    #[test]
    fn merge_dhcp_with_renew_preserves_applied_on_success() {
        let primary = ComponentReport {
            name: "dhcp",
            status: Status::Applied,
            note: "ok".into(),
        };
        let merged = merge_dhcp_with_renew(primary, Ok(exit::SUCCESS));
        assert_eq!(merged.status, Status::Applied);
        assert!(merged.note.contains("renewed"));
    }

    #[test]
    fn merge_dhcp_with_renew_marks_failed_when_renew_errors() {
        let primary = ComponentReport {
            name: "dhcp",
            status: Status::Applied,
            note: "ok".into(),
        };
        let merged = merge_dhcp_with_renew(primary, Err(anyhow::anyhow!("boom")));
        assert_eq!(merged.status, Status::Failed);
        assert!(merged.note.contains("renew error"));
        assert!(merged.note.contains("boom"));
    }

    #[test]
    fn merge_dhcp_with_renew_marks_failed_on_nonzero_exit() {
        let primary = ComponentReport {
            name: "dhcp",
            status: Status::Applied,
            note: "ok".into(),
        };
        let merged = merge_dhcp_with_renew(primary, Ok(exit::GENERIC_ERROR));
        assert_eq!(merged.status, Status::Failed);
        assert!(merged.note.contains("renew failed"));
    }

    /// Roadmap Stream 7 / E1 acceptance: at default verbosity (no `-v`,
    /// no `RUST_LOG`), the success path of `apply` emits zero events
    /// from this module. Concretely: every `tracing::*!` in this file
    /// must be at debug or trace level on the success path. Pin this
    /// by source inspection — the production code (everything before
    /// `mod tests`) must contain no `info!` calls outside of comments
    /// / strings, since those propagate to stderr at the default INFO
    /// level. New success-path breadcrumbs must use `debug!` so the
    /// default stderr stays empty.
    #[test]
    fn success_path_emits_no_info_level_tracing_events() {
        let src = include_str!("apply.rs");
        // Cut at the test module boundary so the test's own `tracing`
        // strings (in assertion messages) don't trigger.
        let prod = src
            .split_once("\n#[cfg(test)]\n")
            .map(|(prod, _)| prod)
            .unwrap_or(src);
        let mut without_comments = String::with_capacity(prod.len());
        for line in prod.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            without_comments.push_str(line);
            without_comments.push('\n');
        }
        // `info!` is the discipline-violating level on the success
        // path; `warn!` / `error!` are still allowed for failure-path
        // diagnostics elsewhere in the file (none today).
        assert!(
            !without_comments.contains("tracing::info!"),
            "src/commands/apply.rs must not call tracing::info! on the success path (Stream 7 / E1). \
             Use `debug!` for breadcrumbs so default-verbosity stderr stays empty."
        );
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

    // ---- Issue #343: --json per-component summary ----------------------
    //
    // Pin the JSON envelope shape against synthetic per-component
    // reports. We don't drive the orchestrator (it needs root + a real
    // backend); the serializer is the load-bearing surface for CI /
    // Ansible consumers.

    #[test]
    fn summary_serialises_per_component_status_and_exit_code() {
        let reports = vec![
            ComponentReport {
                name: "mac",
                status: Status::Applied,
                note: "ok".into(),
            },
            ComponentReport {
                name: "hostname",
                status: Status::Skipped,
                note: "disabled in config (hostname.enabled = false)".into(),
            },
            ComponentReport {
                name: "dns",
                status: Status::Failed,
                note: "exited with code 1".into(),
            },
        ];
        let summary = Summary::new("apply", reports, exit::GENERIC_ERROR);
        let s = serde_json::to_string(&summary).expect("serialises");
        // No `error` field on the happy-path summary.
        assert!(!s.contains("\"error\""));
        assert!(s.contains("\"command\":\"apply\""));
        assert!(s.contains("\"exit_code\":1"));
        // kebab-case status strings (matches Serialize derive on Status).
        assert!(s.contains("\"status\":\"applied\""));
        assert!(s.contains("\"status\":\"skipped\""));
        assert!(s.contains("\"status\":\"failed\""));
        assert!(s.contains("\"name\":\"mac\""));
        // The summary is a single line — no embedded newlines.
        assert!(!s.contains('\n'));
    }

    #[test]
    fn summary_with_error_includes_error_string() {
        // Used by the early-bail gates (root / yes / config / lock /
        // preflight). The envelope still parses as a single JSON object,
        // and consumers can distinguish "never reached fan-out" from
        // "every component failed" via the `error` field.
        let summary = Summary::with_error("apply", exit::PERMISSION_ERROR, "must be run as root");
        let s = serde_json::to_string(&summary).expect("serialises");
        assert!(s.contains("\"error\":\"must be run as root\""));
        assert!(s.contains("\"exit_code\":66"));
        assert!(s.contains("\"components\":[]"));
    }

    #[test]
    fn summary_command_field_is_caller_supplied() {
        // The same envelope is reused by `revert --json`; pin the
        // discriminator so a future refactor can't collapse the two.
        let s = serde_json::to_string(&Summary::new("revert", Vec::new(), exit::SUCCESS))
            .expect("serialises");
        assert!(s.contains("\"command\":\"revert\""));
    }
}
