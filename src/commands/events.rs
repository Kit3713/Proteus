// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus events run` — long-lived event-driven rotation daemon.
//! Roadmap Milestone 4c.
//!
//! The daemon's job is small: build an [`EventRegistry`], register a
//! default rotation handler, spawn every available [`EventSource`],
//! and block until told to stop (Ctrl-C, the systemd unit's
//! `ExecStop`, or `--once-after-secs` / `--max-triggers` for the
//! smoke-test path). When a trigger fires, the default handler maps
//! it to the existing rotate entry point so persona / OUI shaping /
//! probe-aware collision retry all keep working.
//!
//! ## Why a separate subcommand
//!
//! The NM dispatcher script (`dist/networkmanager/dispatcher.d/01-proteus`)
//! already covers the connection-up case for hosts with NM. The
//! `events` daemon picks up the slack on hosts that don't have NM
//! (networkd, raw) and on triggers the dispatcher doesn't expose
//! (link-flap, reg-domain, portal-auth). Operators with both can
//! disable one or the other; the dispatcher is non-mutating from
//! Proteus's side, the daemon is opt-in via `[events] enabled`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::backend::NetworkBackend;
use crate::config::Config;
use crate::events::source::{
    LinkFlapSource, NmConnectionUpSource, PortalAuthSource, RegDomainChangeSource,
    SystemPortalSampler,
};
use crate::events::{EventHandler, EventRegistry, RotationTrigger};
use crate::exit;

/// Default handler — turns a `RotationTrigger` into a rotation. The
/// handler captures the state + config paths at construction so the
/// async rotate path runs against the same files the rest of the
/// CLI does.
///
/// Roadmap Milestone 3: when a `ConnectionUp` trigger carries an SSID,
/// the handler resolves the per-SSID policy via
/// [`crate::per_ssid::resolve_for_ssid`] so the persona / profile /
/// pin_mac override for that specific network is visible at trigger
/// time. The resolution is reload-on-trigger (cheap, the config is a
/// small TOML) so an operator can edit `config.toml` without restarting
/// the daemon.
struct RotateOnTriggerHandler {
    /// Issue #259/#262: shared with the daemon's main loop so the
    /// `--max-triggers` budget enforcement reads the same counter
    /// the handler increments. `Arc<AtomicU64>` is the simplest
    /// way to share a `Send + Sync` integer between the handler
    /// (boxed into the registry) and the loop (owns its own
    /// `Arc` clone).
    counter: Arc<std::sync::atomic::AtomicU64>,
    config_path: Option<PathBuf>,
    state_path: Option<PathBuf>,
    /// N1 (the most important single fix in the roadmap): when the
    /// handler is constructed via [`RotateOnTriggerHandler::with_backend`]
    /// it owns an `Arc<dyn NetworkBackend>` and a tokio runtime
    /// `Handle`. On every trigger it dispatches the existing
    /// [`crate::commands::rotate::run_with_backend`] pipeline against
    /// that backend so the trigger actually rotates the MAC instead
    /// of merely logging the trigger. The rotate work is dispatched
    /// via `Handle::spawn` so the registry's serial dispatch loop
    /// stays unblocked — handlers run synchronously, the rotate
    /// runs concurrently on the same tokio runtime that owns the
    /// source tasks. Counter increments happen at trigger observation
    /// time (not after rotate completes) so `--max-triggers` budgets
    /// the trigger rate, not the rotate-completion rate.
    backend: Option<Arc<dyn NetworkBackend>>,
    runtime: Option<tokio::runtime::Handle>,
    /// Issue #266: track in-flight rotate tasks so the daemon can wait
    /// for them at shutdown. `Arc<Mutex<Vec<JoinHandle>>>` rather than
    /// firing-and-forgetting because a hard SIGTERM landing mid-rotate
    /// could otherwise leave the backend half-written. Reaped on every
    /// dispatch so the vec doesn't grow unbounded over a long-running
    /// daemon.
    in_flight: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl RotateOnTriggerHandler {
    fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            config_path,
            state_path: None,
            backend: None,
            runtime: None,
            in_flight: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// N1 wiring: build a handler that actually rotates on trigger.
    /// The runtime handle is captured so the sync `EventHandler::handle`
    /// can dispatch async rotate work onto the daemon's tokio runtime.
    fn with_backend(
        config_path: Option<PathBuf>,
        state_path: Option<PathBuf>,
        backend: Arc<dyn NetworkBackend>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            config_path,
            state_path,
            backend: Some(backend),
            runtime: Some(runtime),
            in_flight: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn load_config(&self) -> Option<Config> {
        let cfg_path = super::config_path(self.config_path.as_deref());
        Config::default_or_loaded(&cfg_path).ok()
    }

    /// Reap completed rotate tasks. Called at the start of every
    /// `dispatch_rotate` so the vec stays bounded; under steady-state
    /// each rotate completes in well under the inter-trigger gap so
    /// the vec is typically empty.
    fn reap_completed(&self) {
        let mut guard = match self.in_flight.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.retain(|h| !h.is_finished());
    }

    /// Snapshot of currently-spawned rotate tasks. Used by tests to
    /// await completion synchronously without racing against the
    /// `tokio::spawn` future. Production callers ignore the return
    /// value — the daemon's shutdown path drains via the registered
    /// source tasks.
    fn dispatch_rotate(&self, iface: Option<String>) {
        let (Some(backend), Some(runtime)) = (self.backend.clone(), self.runtime.clone()) else {
            // No backend wired in — this is the legacy "log only"
            // path used by the test handlers and by the construction
            // below when the backend select fails. The trigger
            // counter has already been bumped by the caller.
            return;
        };
        self.reap_completed();
        let config_path = super::config_path(self.config_path.as_deref());
        let state_path = super::state_path(self.state_path.as_deref());
        let in_flight = Arc::clone(&self.in_flight);
        let join = runtime.spawn(async move {
            if let Err(e) = run_rotate_for_trigger(backend, &config_path, &state_path, iface).await
            {
                // Rotation failures are warned but not propagated —
                // the events daemon must keep running through a
                // transient backend failure. The next trigger
                // gets a fresh shot.
                tracing::warn!("events: rotate-on-trigger failed: {e:#}");
            }
        });
        if let Ok(mut guard) = in_flight.lock() {
            guard.push(join);
        } else if let Err(p) = in_flight.lock() {
            p.into_inner().push(join);
        }
    }
}

/// N1 implementation: drive `crate::commands::rotate::run_with_backend`
/// against the daemon's chosen backend. Lives outside the handler
/// struct so the `Send`-friendly `'static` bounds the spawned task
/// requires are easy to satisfy — the captures are the `Arc<dyn
/// NetworkBackend>`, the two `PathBuf`s, and the optional `iface`.
async fn run_rotate_for_trigger(
    backend: Arc<dyn NetworkBackend>,
    config_path: &Path,
    state_path: &Path,
    iface: Option<String>,
) -> Result<()> {
    use crate::mac::probe::SystemProbe;
    use crate::mac::{Mac, arp};
    use crate::state::State;

    let config = Config::default_or_loaded(config_path)?;
    let mut state = State::load_or_default(state_path)?;
    // Mirror `commands::rotate::run` — assemble the avoid set from
    // live ARP, the gateway, and the recent-neighbour ledger. The
    // events daemon doesn't get the explain/yes/iface_filter knobs;
    // a triggered rotation always rotates whatever the trigger
    // points at, falling back to "every managed iface" for triggers
    // without an iface payload (reg-domain, portal-auth).
    let arp_macs = arp::read_arp_macs();
    let recent = arp::RecentNeighbourTable::new();
    recent.record_all(arp_macs.iter().copied());
    let gateway_mac = arp::read_default_gateway_mac();
    let mut avoid: HashSet<Mac> = arp_macs;
    if let Some(gw) = gateway_mac {
        avoid.insert(gw);
    }
    for m in recent.current_macs() {
        avoid.insert(m);
    }
    let probe = SystemProbe::new();
    let _report = crate::commands::rotate::run_with_backend(
        backend.as_ref(),
        iface.as_deref(),
        &config,
        &avoid,
        &probe,
        false,
        &mut state,
        state_path,
    )
    .await?;
    state.save(state_path)?;
    Ok(())
}

impl EventHandler for RotateOnTriggerHandler {
    fn handle(&self, trigger: &RotationTrigger) -> Result<()> {
        // N1 (the highest-impact fix in the roadmap): the rotate
        // pipeline is dispatched inline (well, on the same tokio
        // runtime; see `dispatch_rotate`). The handler stays sync —
        // it kicks off the async rotate via `Handle::spawn` so the
        // registry's serial dispatch loop is not blocked, and
        // increments its trigger counter for the daemon's
        // `--max-triggers` budget. Counter increments at observation
        // time so the budget reflects how many *triggers* fired,
        // not how many *rotates completed* — flapping infrastructure
        // can keep the rate-limiter informed even when the rotates
        // pile up behind a slow backend.
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Roadmap Milestone 3 connection-up wiring: when the trigger
        // carries an SSID, resolve the per-SSID policy and log it so
        // an operator can see the right network rules took effect.
        // Other trigger kinds fall through to a plain trigger log.
        let iface = match trigger {
            RotationTrigger::ConnectionUp { iface, ssid } => {
                if let Some(ssid) = ssid
                    && let Some(cfg) = self.load_config()
                {
                    let policy = crate::per_ssid::resolve_for_ssid(&cfg, ssid);
                    // Issue #224: SSIDs are attacker-controlled. Sanitize
                    // before logging so journald renders something the
                    // operator can read without a hostile AP redrawing
                    // their terminal via the `journalctl` viewer.
                    let ssid_safe = crate::per_ssid::display_ssid(ssid);
                    // E2: demoted from info to debug. Per-trigger
                    // success-path logging fires on every connection-up,
                    // which on a busy dispatcher easily hits journald rate
                    // limits. The shutdown / startup / budget-reached
                    // lines stay at info because they're once-per-run.
                    tracing::debug!(
                        kind = trigger.kind(),
                        iface = iface.as_str(),
                        ssid = ssid_safe.as_str(),
                        persona = policy.persona.as_deref().unwrap_or("-"),
                        profile = ?policy.profile,
                        pinned = policy.pin_mac.is_some(),
                        source = ?policy.source,
                        "events: connection-up resolved per-SSID policy"
                    );
                }
                Some(iface.clone())
            }
            RotationTrigger::LinkFlap { iface } => Some(iface.clone()),
            RotationTrigger::PortalAuth { .. } | RotationTrigger::RegDomainChange { .. } => None,
        };

        // E2: demoted from info to debug — trigger observation is the
        // hot path; the operator wants to know the daemon is alive
        // (info: "events daemon started" / "trigger budget reached"
        // are kept at info because they're once-per-run). Per-trigger
        // success-path lines fire on every connection-up which on a
        // busy dispatcher easily hits journald rate limits.
        tracing::debug!(
            kind = trigger.kind(),
            iface = iface.as_deref().unwrap_or("-"),
            "events: trigger observed; rotating via backend"
        );

        // N1: actually dispatch the rotate pipeline against the
        // configured backend. When `with_backend` was not used
        // (legacy / test paths) this is a no-op.
        self.dispatch_rotate(iface);
        Ok(())
    }
}

/// Public entry point: builds the registry, registers handlers, and
/// runs the loop until the configured stop condition fires. Returns
/// an exit code (0 on clean shutdown, non-zero on misconfiguration).
pub fn run(
    force: bool,
    max_triggers: u64,
    once_after_secs: u64,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    let cfg_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&cfg_path).unwrap_or_default();

    if !config.events.enabled && !force {
        eprintln!(
            "proteus: [events] enabled = false in {}; pass --force to run anyway, \
             or `proteus config enable events` to flip the master switch",
            cfg_path.display()
        );
        return Ok(exit::CONFIG_ERROR);
    }

    // Issue #223: the daemon mutates state through the rotation handler
    // and (once Milestone 4c lands the active rotate body) needs root for
    // the netlink + DBus + nft writes. Mirroring `apply` / `rotate` /
    // `uninstall` keeps the privilege story consistent — config errors
    // still print first so a non-root operator sees the right message
    // when they forget `--force`.
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }

    let registry = EventRegistry::shared();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime for events daemon")?;
    // N1: build the handler with a real backend + the daemon's
    // tokio runtime handle so triggers actually rotate. The backend
    // selector talks DBus to NetworkManager, which is async, so we
    // run it on the runtime here. Failure to select a backend is a
    // soft failure — the daemon stays up but the handler falls back
    // to its log-only path. This keeps the smoke-test container
    // (which has no backend) able to run the daemon, while a real
    // host gets the rotate-on-trigger behaviour the docs promise.
    let backend_arc: Option<Arc<dyn NetworkBackend>> =
        match rt.block_on(crate::backend::select::select(&config.backend.driver)) {
            Ok(b) => Some(Arc::from(b)),
            Err(e) => {
                tracing::warn!(
                    "events daemon: backend select failed, rotate-on-trigger disabled: {e:#}"
                );
                None
            }
        };
    let handler = match backend_arc {
        Some(backend) => RotateOnTriggerHandler::with_backend(
            config_path.map(PathBuf::from),
            state_path.map(PathBuf::from),
            backend,
            rt.handle().clone(),
        ),
        None => RotateOnTriggerHandler::new(config_path.map(PathBuf::from)),
    };
    let trigger_count = Arc::clone(&handler.counter);
    let in_flight = Arc::clone(&handler.in_flight);
    registry
        .register(Box::new(handler))
        .context("registering default rotation handler")?;

    rt.block_on(async {
        let started = Instant::now();
        let mut tasks = Vec::new();

        // Connection-up — gracefully degrades when DBus isn't
        // available (typical in CI / non-NM hosts).
        if let Some(t) = NmConnectionUpSource::new()
            .spawn_into(Arc::clone(&registry))
            .await
        {
            tasks.push(t);
        }
        // Link-flap — needs CAP_NET_ADMIN to bind the netlink
        // socket; degrades to a no-op task when the bind fails.
        if let Some(t) =
            LinkFlapSource::with_window(Duration::from_secs(config.events.link_flap_window_secs))
                .spawn_into(Arc::clone(&registry))
                .await
        {
            tasks.push(t);
        }
        // Reg-domain — same CAP_NET_ADMIN gate as link-flap.
        if let Some(t) = RegDomainChangeSource::new()
            .spawn_into(Arc::clone(&registry))
            .await
        {
            tasks.push(t);
        }
        // Portal-auth — no privilege requirement; always runs.
        let sampler = Arc::new(SystemPortalSampler::new(
            config.captive_portal.detect_url.clone(),
            config.captive_portal.expected_response.clone(),
            config.captive_portal.timeout_secs,
        ));
        if let Some(t) = PortalAuthSource::new(sampler, config.events.portal_poll_secs)
            .spawn_into(Arc::clone(&registry))
            .await
        {
            tasks.push(t);
        }

        tracing::info!(
            sources = tasks.len(),
            poll_secs = config.events.portal_poll_secs,
            flap_window_secs = config.events.link_flap_window_secs,
            "events daemon started"
        );

        // Block until a stop condition fires. The shutdown loop
        // checks both the trigger budget (`--max-triggers`) and the
        // wall-clock budget (`--once-after-secs`) every 250 ms; a
        // smoke-test run with `--once-after-secs 1` exits within
        // ~1.25 s. Production sets neither and runs until SIGTERM.
        //
        // Issue #259/#262: previously the trigger-budget branch was a
        // dead body — the daemon parsed `--max-triggers` but ignored
        // it. The handler now exposes its `Arc<AtomicU64>` counter to
        // the loop so the budget is honoured: when the count meets or
        // exceeds the budget the loop breaks with a "trigger budget
        // reached" log line.
        //
        // Roadmap C4: previously the production "runs until SIGTERM"
        // path relied on the OS dropping the process — systemd's
        // SIGTERM landed as a SIGKILL-equivalent because the loop
        // body had no way to observe the signal, so the in-flight
        // rotate drain + per-source `shutdown_tasks` below were only
        // reachable from the trigger-budget / wall-clock-budget
        // exits. The select! arms below race the 250 ms tick against
        // SIGTERM (systemd `ExecStop`) and SIGINT (interactive
        // Ctrl-C); on signal the loop breaks normally so the same
        // graceful drain path runs. Manual verification:
        // `pkill -TERM proteus` — journald should show the
        // "SIGTERM received" line followed by the source-drain
        // lines; signal delivery is not exercised in `cargo test`
        // because spawning real signals from inside the test harness
        // is fragile (it would race other tests on the same pid).
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("installing SIGTERM handler for events daemon")?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .context("installing SIGINT handler for events daemon")?;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                _ = sigterm.recv() => {
                    tracing::info!(
                        "events daemon: SIGTERM received; draining and shutting down"
                    );
                    break;
                }
                _ = sigint.recv() => {
                    tracing::info!(
                        "events daemon: SIGINT received; draining and shutting down"
                    );
                    break;
                }
            }
            if max_triggers > 0
                && trigger_count.load(std::sync::atomic::Ordering::SeqCst) >= max_triggers
            {
                tracing::info!(
                    max_triggers = max_triggers,
                    fired = trigger_count.load(std::sync::atomic::Ordering::SeqCst),
                    "events daemon: trigger budget reached; shutting down"
                );
                break;
            }
            if once_after_secs > 0 && started.elapsed().as_secs() >= once_after_secs {
                tracing::info!("events daemon: --once-after-secs elapsed; shutting down");
                break;
            }
        }

        // Issue #256: drain every source's join handle within a
        // bounded deadline before aborting. Previously we sent the
        // stop signal and dropped the join handles, which leaks the
        // orchestrator's "wait for shutdown" guarantee — long-running
        // tasks could outlive the daemon. The deadline is 5 s
        // (long enough for a netlink recv to wake on its stop signal,
        // short enough that systemd's default `TimeoutStopSec=` of
        // 90 s never trips).
        shutdown_tasks(tasks, Duration::from_secs(5)).await;

        // N1: drain any rotate tasks that are still running so a
        // SIGTERM landing mid-rotation doesn't leave the backend
        // half-written. We swap the vec out so the lock is released
        // before we await — none of the spawned tasks try to take
        // this lock back, but the swap is the obviously-correct
        // shape regardless. Bounded by the same 5 s budget the
        // source drain uses.
        //
        // C7: surface JoinError::is_panic() at tracing::error! so a
        // panicking rotate task does not disappear into the void. The
        // daemon already kept running through it (tokio task isolation),
        // but a silent discard means an operator running
        // `journalctl -u proteus-events` sees no signal that a
        // rotation failed because of a bug. Mirrors the JoinError
        // handling shape in `shutdown_tasks` below.
        let pending: Vec<tokio::task::JoinHandle<()>> = match in_flight.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        };
        for join in pending {
            match tokio::time::timeout(Duration::from_secs(5), join).await {
                Ok(Ok(())) => {}
                Ok(Err(join_err)) => {
                    if join_err.is_panic() {
                        let payload = join_err.into_panic();
                        let msg = crate::events::panic_payload_message(payload.as_ref());
                        tracing::error!(
                            panic = msg.as_str(),
                            "events daemon: rotate-on-trigger task panicked"
                        );
                    } else {
                        tracing::warn!(
                            "events daemon: rotate-on-trigger task cancelled: {join_err}"
                        );
                    }
                }
                Err(_) => {
                    tracing::warn!(
                        "events daemon: rotate-on-trigger task did not complete within 5s"
                    );
                }
            }
        }

        Ok::<u8, anyhow::Error>(exit::SUCCESS)
    })
}

/// Roadmap #283: read-only enumeration of the four event sources the
/// daemon can subscribe to, each annotated with a host-side
/// availability probe.
///
/// The probe shapes mirror the actual `spawn_into` paths the daemon
/// uses, so what `list-sources` reports here is the same gate
/// `proteus events run` would observe at startup:
///
/// - `nm-connection-up` — `/run/NetworkManager` marker check
///   (cheap path stat).
/// - `link-flap` — try-bind a `NETLINK_ROUTE` socket; degrades when
///   the bind fails (typical without `CAP_NET_ADMIN`).
/// - `reg-domain` — same on `NETLINK_GENERIC`.
/// - `portal-auth` — pure userspace HTTP poller; always available.
///
/// Read-only and non-mutating: safe to run when `[events] enabled =
/// false` and from any non-root account. The probes themselves are
/// cheap (one path check + two netlink binds); the command exits in
/// single-digit milliseconds.
pub fn list_sources(json: bool) -> Result<u8> {
    let sources = probe_sources();
    if json {
        // Single-line array — the roadmap contract so a pipe to
        // `head -1` always sees the full payload.
        let json_sources: Vec<SourceEntryJson<'_>> =
            sources.iter().map(SourceEntryJson::from).collect();
        let s = serde_json::to_string(&json_sources)
            .context("serialising events list-sources report")?;
        println!("{s}");
    } else {
        print_sources_human(&sources);
    }
    Ok(exit::SUCCESS)
}

/// Status of a probed event source. Two states only — operators
/// reading `proteus events list-sources` care about "can the daemon
/// use this source on this host" and, when not, "what's missing." A
/// third "unknown" state would just kick the can; the probe is cheap
/// enough to always produce a definite answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceStatus {
    Available,
    Degraded,
}

impl SourceStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Degraded => "degraded",
        }
    }
}

/// One row in the `list-sources` output. Field names match the JSON
/// schema callers will key off — `name`, `status`, `degraded_reason`,
/// `capability_needed`.
#[derive(Debug)]
struct SourceEntry {
    name: &'static str,
    status: SourceStatus,
    /// `None` when `status == Available`. Owned `String` so the probe
    /// can compose a host-specific reason (e.g. the libc error from a
    /// failed netlink bind) without leaking a `&'static str` for
    /// every possible failure mode.
    degraded_reason: Option<String>,
    /// Human-readable description of the kernel cap / DBus interface
    /// that gates this source. Stable across hosts — the same string
    /// for any probe outcome.
    capability_needed: &'static str,
}

/// JSON shape — borrows from `SourceEntry` so we don't double-alloc
/// for the serialiser. `degraded_reason` stays `Option<&str>` so a
/// degraded source emits the explanation and an available source
/// emits `null`, matching the documented JSON contract.
#[derive(serde::Serialize)]
struct SourceEntryJson<'a> {
    name: &'a str,
    status: &'a str,
    degraded_reason: Option<&'a str>,
    capability_needed: &'a str,
}

impl<'a> From<&'a SourceEntry> for SourceEntryJson<'a> {
    fn from(e: &'a SourceEntry) -> Self {
        Self {
            name: e.name,
            status: e.status.as_str(),
            degraded_reason: e.degraded_reason.as_deref(),
            capability_needed: e.capability_needed,
        }
    }
}

/// Build the four-entry source list. Order matches the daemon's
/// `spawn_all`: nm-connection-up, link-flap, reg-domain, portal-auth.
/// Stable so scripts grepping the table can rely on row ordering.
fn probe_sources() -> Vec<SourceEntry> {
    vec![
        probe_nm_connection_up(),
        probe_link_flap(),
        probe_reg_domain(),
        probe_portal_auth(),
    ]
}

/// NM connection-up probe — needs a running NetworkManager.
/// `/run/NetworkManager` is the same marker
/// `commands::status::detect_system` keys off, so what we report here
/// matches what an operator sees in `proteus status`. The DBus open
/// itself happens lazily inside `spawn_into`; we don't open a
/// connection in the read-only path because it would re-authenticate
/// per invocation. The marker-file probe is cheap and a sufficient
/// proxy.
fn probe_nm_connection_up() -> SourceEntry {
    let (status, reason) = if crate::events::source::probe_nm_connection_up_available() {
        (SourceStatus::Available, None)
    } else {
        (
            SourceStatus::Degraded,
            Some(String::from("NetworkManager not running")),
        )
    };
    SourceEntry {
        name: "nm-connection-up",
        status,
        degraded_reason: reason,
        capability_needed: "system DBus (org.freedesktop.NetworkManager)",
    }
}

/// Link-flap probe — try to bind a NETLINK_ROUTE socket via the
/// shared probe helper. Mirrors the daemon's actual `spawn_into` path
/// so what we report here is what `proteus events run` would observe
/// at startup.
fn probe_link_flap() -> SourceEntry {
    match crate::events::source::probe_link_flap_available() {
        Ok(()) => SourceEntry {
            name: "link-flap",
            status: SourceStatus::Available,
            degraded_reason: None,
            capability_needed: "CAP_NET_ADMIN (NETLINK_ROUTE)",
        },
        Err(e) => SourceEntry {
            name: "link-flap",
            status: SourceStatus::Degraded,
            // Probe-error renderings vary by libc (EPERM vs "Operation
            // not permitted"); pin a stable reason for the typical
            // CAP_NET_ADMIN-missing case and append the OS detail.
            degraded_reason: Some(format!(
                "netlink bind failed (likely missing CAP_NET_ADMIN): {e:#}"
            )),
            capability_needed: "CAP_NET_ADMIN (NETLINK_ROUTE)",
        },
    }
}

/// Reg-domain probe — try to bind a NETLINK_GENERIC socket. The full
/// nl80211 family-id resolution is more involved than this probe
/// captures (an older kernel without nl80211 still lets the genetlink
/// bind succeed), but the gate the daemon actually trips on is the
/// bind, so this matches the runtime behaviour. The reason string
/// covers both the cap and the nl80211 case so an operator sees the
/// full set of possibilities.
fn probe_reg_domain() -> SourceEntry {
    match crate::events::source::probe_reg_domain_available() {
        Ok(()) => SourceEntry {
            name: "reg-domain",
            status: SourceStatus::Available,
            degraded_reason: None,
            capability_needed: "CAP_NET_ADMIN (NETLINK_GENERIC + nl80211)",
        },
        Err(e) => SourceEntry {
            name: "reg-domain",
            status: SourceStatus::Degraded,
            degraded_reason: Some(format!(
                "genetlink bind failed (missing CAP_NET_ADMIN or nl80211 absent): {e:#}"
            )),
            capability_needed: "CAP_NET_ADMIN (NETLINK_GENERIC + nl80211)",
        },
    }
}

/// Portal-auth probe — the captive-portal poller is pure userspace
/// HTTP and has no privilege requirement. The daemon's
/// `PortalAuthSource::spawn_into` always returns `Some` regardless of
/// host privilege, so we mirror that and report `available`
/// unconditionally.
fn probe_portal_auth() -> SourceEntry {
    SourceEntry {
        name: "portal-auth",
        status: SourceStatus::Available,
        degraded_reason: None,
        capability_needed: "(none — userspace HTTP poller)",
    }
}

/// Human-readable table. One header line + one row per source.
/// Columns: name (20), status (10), capability_needed (rest), with
/// degraded reasons folded onto a second indented line so a wide
/// reason doesn't push the capability column off-screen.
///
/// `display_safe` is applied to `degraded_reason` only — the other
/// columns are `&'static str` from the probe set and cannot carry
/// attacker-controlled bytes. The reason field embeds a libc error
/// string on the netlink probes, which is the only place an OS
/// translation could land a non-printable.
fn print_sources_human(sources: &[SourceEntry]) {
    println!("{:<20} {:<10} CAPABILITY_NEEDED", "NAME", "STATUS");
    for entry in sources {
        println!(
            "{:<20} {:<10} {}",
            entry.name,
            entry.status.as_str(),
            entry.capability_needed
        );
        if let Some(reason) = entry.degraded_reason.as_deref() {
            let reason_safe = crate::display::display_safe(reason);
            println!("  reason: {reason_safe}");
        }
    }
}

/// Per-source graceful shutdown. Signals every source to stop, waits
/// up to `deadline` for each `JoinHandle` to complete, then aborts
/// any stragglers. Issue #256.
///
/// The drain happens sequentially — sources are independent so the
/// total wall-clock budget is `deadline` per source, not aggregated.
/// In practice every well-behaved source returns the moment it sees
/// its `stop` channel close, which is microseconds; the deadline is
/// the ceiling for "the source is wedged in a syscall and we have to
/// abort." Logged per-source so an operator triaging a slow shutdown
/// can pinpoint the offender.
async fn shutdown_tasks(tasks: Vec<crate::events::source::SourceTask>, deadline: Duration) {
    for task in tasks {
        let name = task.name;
        // `stop()` consumes the StopHandle; signal first so the inner
        // task observes the close before we begin waiting.
        task.stop.stop();
        let mut join = task.join;
        // Pin the borrow so the JoinHandle survives a timeout — if
        // the task overruns its deadline we still hold the handle and
        // can call `.abort()` on it.
        match tokio::time::timeout(deadline, &mut join).await {
            Ok(Ok(())) => {
                tracing::debug!(source = name, "events daemon: source shut down cleanly");
            }
            Ok(Err(join_err)) => {
                // C7: differentiate panic from cancellation so an
                // operator running `journalctl -u proteus-events` can
                // tell a "the source's loop unwound on a bug" event
                // apart from a "the source was cancelled at shutdown"
                // one. Panic logs at `error!` with a downcast of the
                // payload to `&str` / `String` so the message is
                // legible. Cancellation stays at `warn!` because it's
                // expected during shutdown if a source's stop arm
                // races the join.
                if join_err.is_panic() {
                    let payload = join_err.into_panic();
                    let msg = crate::events::panic_payload_message(payload.as_ref());
                    tracing::error!(
                        source = name,
                        panic = msg.as_str(),
                        "events daemon: source task panicked"
                    );
                } else {
                    tracing::warn!(
                        source = name,
                        "events daemon: source task cancelled at shutdown: {join_err}"
                    );
                }
            }
            Err(_) => {
                tracing::warn!(
                    source = name,
                    deadline_secs = deadline.as_secs(),
                    "events daemon: source did not shut down within deadline; aborting"
                );
                // Abort explicitly; dropping a JoinHandle only
                // detaches the task. After `abort()` we wait briefly
                // for the cancellation to propagate so the abort is
                // synchronous from the caller's perspective.
                join.abort();
                let _ = tokio::time::timeout(Duration::from_millis(250), &mut join).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::events::source::MockNmConnectionUpSource;
    use crate::events::source::nm_connection_up::NM_DEVICE_STATE_ACTIVATED;

    /// Headline acceptance: register a counter handler, fire a
    /// `RotationTrigger::ConnectionUp` on a mock NM source, assert
    /// the handler observed exactly one event.
    #[test]
    fn mock_connection_up_invokes_handler_exactly_once() {
        struct Counter {
            n: Arc<AtomicUsize>,
            last: Arc<Mutex<Option<RotationTrigger>>>,
        }
        impl EventHandler for Counter {
            fn handle(&self, t: &RotationTrigger) -> Result<()> {
                self.n.fetch_add(1, Ordering::SeqCst);
                *self.last.lock().unwrap() = Some(t.clone());
                Ok(())
            }
        }

        let n = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let registry = EventRegistry::new();
        registry
            .register(Box::new(Counter {
                n: Arc::clone(&n),
                last: Arc::clone(&last),
            }))
            .unwrap();

        let src = MockNmConnectionUpSource::new();
        src.push("wlan0", NM_DEVICE_STATE_ACTIVATED, Some("home".into()));
        crate::events::source::EventSource::start(&src, &registry).unwrap();

        assert_eq!(
            n.load(Ordering::SeqCst),
            1,
            "handler must fire exactly once"
        );
        match last.lock().unwrap().as_ref().unwrap() {
            RotationTrigger::ConnectionUp { iface, ssid } => {
                assert_eq!(iface, "wlan0");
                assert_eq!(ssid.as_deref(), Some("home"));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    /// `--force false` + `[events] enabled = false` returns the
    /// CONFIG_ERROR code. Pin so the systemd unit's "off by
    /// default" guarantee is enforced even if the operator forgets
    /// `--force`.
    #[test]
    fn run_without_force_when_disabled_returns_config_error() {
        let dir =
            std::env::temp_dir().join(format!("proteus-events-disabled-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "profile = \"med\"\n[events]\nenabled = false\n").unwrap();
        let rc = run(false, 0, 0, None, Some(&cfg_path)).unwrap();
        assert_eq!(rc, exit::CONFIG_ERROR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #223: `--force true` clears the config gate. As non-root
    /// the next gate (the new `require_root` check) returns
    /// `PERMISSION_ERROR`. Together with the disabled-config test above
    /// this pins the gate-ordering: config errors print before privilege
    /// errors so `--force` users see the right message.
    ///
    /// Skipped when EUID=0 (CI containers run as root) — the gate
    /// branch we're pinning is unreachable there.
    #[test]
    fn run_with_force_clears_config_gate_then_hits_root_gate() {
        if super::super::read_uid() == Some(0) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("proteus-events-force-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "profile = \"med\"\n[events]\nenabled = false\n").unwrap();
        let rc = run(true, 0, 1, None, Some(&cfg_path)).unwrap();
        // The cargo-test process is non-root; once `--force` clears the
        // config gate, `require_root` returns PERMISSION_ERROR.
        assert_eq!(rc, exit::PERMISSION_ERROR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #223 mirror: `[events] enabled = true` clears the config
    /// gate without `--force` and lands on the same root gate. Pins the
    /// opt-in flag's semantics through the new privilege check.
    ///
    /// Skipped when EUID=0 (CI containers run as root) — the gate
    /// branch we're pinning is unreachable there.
    #[test]
    fn run_with_enabled_true_clears_config_gate_then_hits_root_gate() {
        if super::super::read_uid() == Some(0) {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("proteus-events-enabled-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "profile = \"med\"\n[events]\nenabled = true\n").unwrap();
        let rc = run(false, 0, 1, None, Some(&cfg_path)).unwrap();
        assert_eq!(rc, exit::PERMISSION_ERROR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `RotateOnTriggerHandler::handle` increments its counter for
    /// every kind of trigger it sees. Surface so a future filter
    /// (e.g. "ignore reg-domain when no Wi-Fi iface present") can be
    /// added with a regression test alongside.
    #[test]
    fn default_handler_counts_every_trigger_kind() {
        let h = RotateOnTriggerHandler::new(None);
        h.handle(&RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: None,
        })
        .unwrap();
        h.handle(&RotationTrigger::LinkFlap {
            iface: "wlan0".into(),
        })
        .unwrap();
        h.handle(&RotationTrigger::RegDomainChange {
            from: "00".into(),
            to: "US".into(),
        })
        .unwrap();
        h.handle(&RotationTrigger::PortalAuth {
            ssid: "Cafe".into(),
        })
        .unwrap();
        assert_eq!(
            h.counter.load(Ordering::SeqCst),
            4,
            "handler must observe every kind"
        );
    }

    /// Roadmap Milestone 3: when a `ConnectionUp` trigger carries an
    /// SSID and the loaded config has a `[per_ssid."<ssid>"]` block,
    /// the handler resolves the policy at trigger time. The handler
    /// itself is observable via its counter (the resolved policy goes
    /// to tracing); the test pins the contract that the load+resolve
    /// path runs without panic against a real on-disk config.
    #[test]
    fn handler_resolves_per_ssid_policy_on_connection_up() {
        let dir =
            std::env::temp_dir().join(format!("proteus-events-per-ssid-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("config.toml");
        std::fs::write(
            &cfg_path,
            "profile = \"med\"\n\
             [per_ssid.\"home\"]\n\
             aggressiveness_profile = \"agr\"\n\
             pin_mac = \"aa:bb:cc:dd:ee:ff\"\n",
        )
        .unwrap();

        let h = RotateOnTriggerHandler::new(Some(cfg_path.clone()));
        h.handle(&RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: Some("home".into()),
        })
        .unwrap();
        assert_eq!(h.counter.load(Ordering::SeqCst), 1);

        // Sanity: the resolver returns the per-SSID policy when called
        // directly with the same config the handler loaded.
        let cfg = crate::config::Config::default_or_loaded(&cfg_path).unwrap();
        let policy = crate::per_ssid::resolve_for_ssid(&cfg, "home");
        assert_eq!(policy.profile, crate::profile::Profile::Agr);
        assert_eq!(policy.pin_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `ConnectionUp` without an SSID falls through to the plain
    /// trigger log path (no per-SSID resolution). Pin so a future
    /// refactor can't accidentally panic on `ssid = None`.
    #[test]
    fn handler_handles_connection_up_without_ssid() {
        let h = RotateOnTriggerHandler::new(None);
        h.handle(&RotationTrigger::ConnectionUp {
            iface: "eth0".into(),
            ssid: None,
        })
        .unwrap();
        assert_eq!(h.counter.load(Ordering::SeqCst), 1);
    }

    /// Issue #259/#262: `shutdown_tasks` drains a clean source
    /// within the deadline — the source's `stop` channel receiver
    /// resolves immediately so the spawned task exits and the
    /// drain returns well under the configured deadline.
    #[test]
    fn shutdown_tasks_drains_a_clean_source_within_deadline() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (stop, mut stop_rx) = crate::events::source::StopHandle::channel();
            let join = tokio::spawn(async move {
                let _ = (&mut stop_rx).await;
            });
            let task = crate::events::source::SourceTask {
                join,
                stop,
                name: "test-source",
            };
            let started = std::time::Instant::now();
            shutdown_tasks(vec![task], Duration::from_secs(5)).await;
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_millis(500),
                "clean shutdown took {elapsed:?}, expected < 500ms"
            );
        });
    }

    /// Issue #259/#262: `shutdown_tasks` aborts a wedged source
    /// past the deadline — a source that ignores its stop channel
    /// must not block the daemon's shutdown indefinitely. We pin
    /// the deadline at 200ms and assert the drain returns inside
    /// 600ms (deadline + 250ms post-abort grace).
    #[test]
    fn shutdown_tasks_aborts_wedged_source_after_deadline() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (stop, _stop_rx) = crate::events::source::StopHandle::channel();
            // Spawn a task that ignores its stop channel and sleeps
            // forever. The drain must abort it after the deadline.
            let join = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            });
            let task = crate::events::source::SourceTask {
                join,
                stop,
                name: "wedged-source",
            };
            let started = std::time::Instant::now();
            shutdown_tasks(vec![task], Duration::from_millis(200)).await;
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_millis(600),
                "wedged-source shutdown took {elapsed:?}, expected < 600ms"
            );
        });
    }

    /// Issue #259/#262: `shutdown_tasks` handles an empty task
    /// list as a no-op — important because `spawn_all` may return
    /// zero tasks on a host without any of the source backends.
    #[test]
    fn shutdown_tasks_with_empty_input_is_a_clean_noop() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            shutdown_tasks(Vec::new(), Duration::from_secs(5)).await;
        });
    }

    /// Issue #259/#262: when `--max-triggers` is 0 (the
    /// production-default), the daemon does not exit on the
    /// trigger-budget branch even after a flood of triggers. Pin
    /// the "0 means run forever" semantics so the systemd unit
    /// stays a long-lived service.
    ///
    /// We exercise the loop predicate directly because spinning
    /// up `run()` requires root + tokio + DBus.
    #[test]
    fn max_triggers_zero_disables_the_budget() {
        let counter: u64 = 1_000_000;
        let max_triggers: u64 = 0;
        let should_exit = max_triggers > 0 && counter >= max_triggers;
        assert!(
            !should_exit,
            "max_triggers=0 must disable the trigger-budget gate"
        );
    }

    /// Issue #259/#262: when `--max-triggers > 0` and the counter
    /// has reached or exceeded it, the budget gate fires.
    #[test]
    fn max_triggers_at_or_above_budget_fires_the_gate() {
        for (max, fired, expected) in [
            (1u64, 0u64, false),
            (1u64, 1u64, true),
            (1u64, 2u64, true),
            (5u64, 4u64, false),
            (5u64, 5u64, true),
            (5u64, 6u64, true),
        ] {
            let should_exit = max > 0 && fired >= max;
            assert_eq!(
                should_exit, expected,
                "max={max}, fired={fired}: expected {expected}"
            );
        }
    }

    /// **N1 acceptance regression test** — the headline of Roadmap
    /// Stream 4. Before this fix the events daemon's default
    /// rotation handler logged the trigger and bumped a counter but
    /// never invoked the rotate pipeline. This test wires a
    /// `RotateOnTriggerHandler` to a `MockBackend`, fires a
    /// `ConnectionUp`, and asserts the backend observed a
    /// `set_cloned_mac` call. Today (post-fix) this passes; pre-fix
    /// it would have hung waiting for the call that never came.
    ///
    /// This is the regression scaffolding the maintainer asked for
    /// (`events_rotate_actually_rotates`-style scenario) lifted into
    /// a unit test so it runs in `cargo test` rather than only
    /// inside the integration container.
    #[test]
    fn rotate_on_trigger_handler_actually_rotates_the_mock_backend() {
        use crate::backend::mock::{MockBackend, MockCall};
        use crate::backend::{BackendDevice, BackendKind, ConnectionRef};
        use crate::state::State;

        // Single-threaded tokio runtime that the handler captures via
        // `Handle::clone()` — the same shape `run()` uses in
        // production.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // Mock backend with one wlan0 device. Seed a connection so the
        // rotate pipeline finds a profile to write the cloned MAC to.
        // Keep a typed `Arc<MockBackend>` for the call-log assertion;
        // pass the same backend to the handler as `Arc<dyn NetworkBackend>`.
        let backend = Arc::new(MockBackend::new());
        let cref = ConnectionRef::new("mock://wlan0/0");
        let device = BackendDevice {
            iface: "wlan0".into(),
            kind: BackendKind::Wifi,
            hw_address: Some("aa:bb:cc:dd:ee:ff".into()),
            identifier: "mock://wlan0".into(),
            connections: vec![cref.clone()],
            managed: true,
        };
        backend.insert_device(device, Some("aa:bb:cc:dd:ee:ff".into()));
        backend.insert_connection(&cref, Some("Home Wi-Fi"), Some("uuid-1"));
        let backend_for_assert = Arc::clone(&backend);
        let backend_arc: Arc<dyn NetworkBackend> = backend;

        // Persisted config + state in a tempdir. The handler reads
        // both via the same paths the production code uses.
        let dir = crate::testing::TempRoot::new("events-rotate");
        let cfg_path = dir.path.join("config.toml");
        let state_path = dir.path.join("state.json");
        std::fs::write(&cfg_path, "profile = \"med\"\n[events]\nenabled = true\n").unwrap();
        // Seed `original_macs` so the rotate path's capture-once
        // guard doesn't try to read sysfs (which on a build host
        // does not have a wlan0).
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.save(&state_path).unwrap();

        // Wire the handler with backend + the test runtime's handle.
        let handler = RotateOnTriggerHandler::with_backend(
            Some(cfg_path.clone()),
            Some(state_path.clone()),
            Arc::clone(&backend_arc),
            rt.handle().clone(),
        );
        let in_flight = Arc::clone(&handler.in_flight);
        let registry = EventRegistry::new();
        registry.register(Box::new(handler)).unwrap();

        // Fire the ConnectionUp from the runtime so `Handle::spawn`
        // inside the handler sees an active reactor. Then drain the
        // spawned rotate task so the assertion runs after rotate
        // completes (not racily).
        rt.block_on(async {
            registry
                .fire(RotationTrigger::ConnectionUp {
                    iface: "wlan0".into(),
                    ssid: Some("home".into()),
                })
                .unwrap();

            // Drain the in-flight rotate task. There should be exactly
            // one — the handler dispatched it on the same runtime.
            let pending: Vec<tokio::task::JoinHandle<()>> = match in_flight.lock() {
                Ok(mut g) => std::mem::take(&mut *g),
                Err(p) => std::mem::take(&mut *p.into_inner()),
            };
            assert!(
                !pending.is_empty(),
                "rotate task should have been spawned by the handler"
            );
            for join in pending {
                tokio::time::timeout(Duration::from_secs(5), join)
                    .await
                    .expect("rotate task did not complete within 5s")
                    .expect("rotate task panicked");
            }
        });

        // Acceptance: the mock backend saw `set_cloned_mac` for
        // wlan0. This is the exact assertion the roadmap calls out.
        let log: Vec<MockCall> = backend_for_assert.call_log();
        assert!(
            log.iter()
                .any(|c| matches!(c, MockCall::SetClonedMac { iface, .. } if iface == "wlan0")),
            "rotate-on-trigger must invoke set_cloned_mac on the mock backend; \
             observed call log = {log:?}"
        );
        assert!(
            backend_for_assert.cloned_mac_for("wlan0").is_some(),
            "the mock backend persisted the new cloned MAC"
        );
    }

    /// N1 negative-shape: a handler built without `with_backend`
    /// (legacy path / sources without a backend wired in) is a
    /// no-op rotation but still increments the trigger counter.
    /// Pins the "log-only" fallback so a future refactor can't
    /// accidentally panic on `backend.is_none()`.
    #[test]
    fn rotate_on_trigger_handler_without_backend_is_log_only() {
        let h = RotateOnTriggerHandler::new(None);
        h.handle(&RotationTrigger::ConnectionUp {
            iface: "wlan0".into(),
            ssid: None,
        })
        .unwrap();
        assert_eq!(h.counter.load(Ordering::SeqCst), 1);
        // No backend wired — the in-flight vec must stay empty.
        assert!(h.in_flight.lock().unwrap().is_empty());
    }

    /// Roadmap #283: `list-sources` reports all four known sources in
    /// a stable order. Off-system the underlying DBus / netlink may
    /// well be missing — the contract is that every kind shows up
    /// regardless, with a `degraded` status + reason when the probe
    /// fails. Pin both row count and the per-row `name` tokens so a
    /// downstream JSON consumer can rely on the ordering.
    #[test]
    fn list_sources_reports_all_four_kinds_in_stable_order() {
        let sources = probe_sources();
        let names: Vec<&'static str> = sources.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec!["nm-connection-up", "link-flap", "reg-domain", "portal-auth"],
            "list-sources must emit the four kinds in spawn_all order"
        );
    }

    /// Every entry has a non-empty `capability_needed` token. Pin so
    /// a future probe addition doesn't accidentally surface a blank
    /// column on the human renderer.
    #[test]
    fn list_sources_entries_each_have_a_capability_needed() {
        for entry in probe_sources() {
            assert!(
                !entry.capability_needed.is_empty(),
                "{}: capability_needed must be non-empty",
                entry.name
            );
        }
    }

    /// Available entries report `degraded_reason: None`; degraded
    /// entries report `Some(reason)`. The invariant matches the JSON
    /// shape callers serialise against.
    #[test]
    fn list_sources_degraded_reason_matches_status() {
        for entry in probe_sources() {
            match entry.status {
                SourceStatus::Available => assert!(
                    entry.degraded_reason.is_none(),
                    "{}: available source must not carry a degraded_reason",
                    entry.name
                ),
                SourceStatus::Degraded => assert!(
                    entry.degraded_reason.is_some(),
                    "{}: degraded source must carry a degraded_reason",
                    entry.name
                ),
            }
        }
    }

    /// Portal-auth always reports `available` — the captive-portal
    /// poller has no privilege gate. Pin so a future refactor can't
    /// silently flip the always-on source off.
    #[test]
    fn list_sources_portal_auth_is_always_available() {
        let sources = probe_sources();
        let entry = sources.iter().find(|s| s.name == "portal-auth").unwrap();
        assert_eq!(entry.status, SourceStatus::Available);
        assert!(entry.degraded_reason.is_none());
    }

    /// JSON shape pins the four fields callers key off:
    /// `name`, `status`, `degraded_reason`, `capability_needed`.
    /// `degraded_reason` is `null` for available sources, a string
    /// for degraded ones — the `serde_json::Value` variant check
    /// makes the contract explicit. Also pins the single-line
    /// invariant the roadmap calls out.
    #[test]
    fn list_sources_json_shape_is_stable() {
        let sources = probe_sources();
        let json_entries: Vec<SourceEntryJson<'_>> =
            sources.iter().map(SourceEntryJson::from).collect();
        let serialised = serde_json::to_string(&json_entries).unwrap();
        assert!(
            !serialised.contains('\n'),
            "list-sources JSON must be single-line; got: {serialised}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&serialised).unwrap();
        let arr = parsed.as_array().expect("top-level JSON must be an array");
        assert_eq!(arr.len(), 4);
        for entry in arr {
            let obj = entry.as_object().unwrap();
            assert!(obj.contains_key("name"));
            assert!(obj.contains_key("status"));
            assert!(obj.contains_key("degraded_reason"));
            assert!(obj.contains_key("capability_needed"));
            let status = obj["status"].as_str().unwrap();
            assert!(
                status == "available" || status == "degraded",
                "status must be one of available|degraded; got {status}"
            );
            match status {
                "available" => assert!(obj["degraded_reason"].is_null()),
                "degraded" => assert!(obj["degraded_reason"].is_string()),
                _ => unreachable!(),
            }
        }
    }

    /// `list_sources` returns SUCCESS in both renderers — the call
    /// is read-only and must not require root or `[events] enabled`.
    /// Pin both renderers so a future refactor can't accidentally
    /// flip one to a non-zero exit.
    #[test]
    fn list_sources_returns_success_in_both_renderers() {
        assert_eq!(list_sources(false).unwrap(), exit::SUCCESS);
        assert_eq!(list_sources(true).unwrap(), exit::SUCCESS);
    }
}
