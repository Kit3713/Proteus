// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus kill` / `proteus resume` — emergency network shutdown.
//!
//! Three entry points:
//!
//! - `kill_run(yes, state_path)` — the destructive activation path. Iterates
//!   `/sys/class/net`, brings every real NIC down via `ip link set <iface>
//!   down`, disables NetworkManager radios via DBus, powers down BlueZ
//!   adapters, and records the snapshot under `state.kill_switch`.
//! - `kill_status(json, state_path)` — read-only. Renders the recorded
//!   kill_switch object.
//! - `resume_run(yes, state_path)` — the restoration path. Reads the
//!   recorded snapshot and reverses each step, then clears the kill_switch
//!   field.
//!
//! Idempotency:
//!
//! - `kill` while already active: prints "kill switch already active" and
//!   exits 0. We do not re-walk the bus / sysfs.
//! - `resume` while not active: prints "kill switch not active" and
//!   exits 0.
//!
//! Both are best-effort — every step records a warning rather than aborting
//! so a partially-broken install can still be operated. Pre-existing
//! patterns from `commands::revert` are mirrored on purpose.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use zbus::proxy;

use crate::bluetooth::{self, AdapterInfo};
use crate::exit;
use crate::kill_switch::{self, KillSwitchState};
use crate::state::State;
use crate::version;

/// Minimal NM proxy for the radio toggles. Pulled out of `src/nm/mod.rs`
/// so the larger device proxy is unaffected and the kill-switch knobs
/// live next to the kill code.
#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NmRadio {
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, value: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn wwan_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wwan_enabled(&self, value: bool) -> zbus::Result<()>;
}

/// Public entry point for `proteus kill [--yes]`.
pub fn kill_run(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "kill is destructive (drops all network traffic, disables radios)",
        "proteus wiki kill-switch",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if state.kill_switch.active {
        println!("kill switch already active");
        if let Some(ts) = &state.kill_switch.activated_at {
            println!("  activated at: {ts}");
        }
        if !state.kill_switch.interfaces.is_empty() {
            println!(
                "  interfaces:   {}",
                state.kill_switch.interfaces.join(", ")
            );
        }
        return Ok(exit::SUCCESS);
    }

    let interfaces = kill_switch::enumerate_managed(Path::new(kill_switch::SYSFS_NET));
    let mut warns: Vec<String> = Vec::new();
    let mut downed: Vec<String> = Vec::new();

    for iface in &interfaces {
        match kill_switch::link_down(iface) {
            Ok(true) => {
                println!("interface {iface}: down");
                downed.push(iface.clone());
            }
            Ok(false) => warns.push(format!(
                "interface {iface}: `ip` not found in PATH; install iproute2"
            )),
            Err(e) => warns.push(format!("interface {iface}: {e}")),
        }
    }

    let radio = toggle_radios(false, true, true);
    let bt = set_bluetooth_powered(false);
    for w in &radio.warns {
        warns.push(format!("nm: {w}"));
    }
    for w in &bt.warns {
        warns.push(format!("bluetooth: {w}"));
    }
    if radio.wireless_changed {
        println!("nm: wireless radio off");
    }
    if radio.wwan_changed {
        println!("nm: wwan radio off");
    }
    if bt.toggled {
        println!("bluetooth: adapters powered off");
    }

    state.kill_switch = KillSwitchState {
        active: true,
        activated_at: Some(super::now_iso8601()),
        interfaces: downed,
        nm_wireless_disabled: radio.wireless_changed,
        nm_wwan_disabled: radio.wwan_changed,
        bluetooth_disabled: bt.toggled,
    };
    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    if warns.is_empty() {
        println!("kill switch: ACTIVE");
    } else {
        eprintln!("kill switch: ACTIVE with {} warning(s):", warns.len());
        for w in &warns {
            eprintln!("  {w}");
        }
        eprintln!("see `proteus wiki kill-switch` for recovery");
    }

    if !interfaces.is_empty() && state.kill_switch.interfaces.is_empty() {
        // Every link-down attempt failed. State still records the attempt so
        // `proteus kill status` shows what the operator tried; exit non-zero
        // so wrappers (and the operator) notice.
        return Ok(exit::GENERIC_ERROR);
    }
    Ok(exit::SUCCESS)
}

/// Public entry point for `proteus kill status [--json]`.
///
/// Issue #235: previously the docstring claimed this was readable without
/// root because `state.json` is "world-readable by design." It isn't:
/// `commands::write_atomic` stamps `state.json` at mode `0o600` and
/// `install.sh` creates `/var/lib/proteus` at `0o700`. A non-root caller
/// hits `EACCES` on the directory and gets a `Permission denied` from
/// `State::load`'s `?` operator.
///
/// Fix matches the codebase's other read commands: require root upfront
/// so the error is uniform, the docstring matches reality, and a future
/// world-readable hint file (issue's option 3, deferred) can be wired in
/// behind a clean entry point. Mutating `kill` / `resume` already require
/// root via the dispatcher, so this only tightens `status`.
pub fn kill_status(json: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(crate::exit::PERMISSION_ERROR);
    }
    let state_path = super::state_path(state_path);
    let state = State::load_or_default(&state_path)?;
    render_status(&state.kill_switch, json)
}

#[derive(Debug, Serialize)]
struct StatusReport<'a> {
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    activated_at: Option<&'a str>,
    interfaces: &'a [String],
    nm_wireless_disabled: bool,
    nm_wwan_disabled: bool,
    bluetooth_disabled: bool,
}

fn render_status(k: &KillSwitchState, json: bool) -> Result<u8> {
    let report = StatusReport {
        active: k.active,
        activated_at: k.activated_at.as_deref(),
        interfaces: &k.interfaces,
        nm_wireless_disabled: k.nm_wireless_disabled,
        nm_wwan_disabled: k.nm_wwan_disabled,
        bluetooth_disabled: k.bluetooth_disabled,
    };
    if json {
        super::print_json(&report)?;
    } else if k.active {
        println!("kill switch: ACTIVE");
        if let Some(ts) = &k.activated_at {
            println!("  activated at:           {ts}");
        }
        if k.interfaces.is_empty() {
            println!("  interfaces:             (none)");
        } else {
            println!("  interfaces:             {}", k.interfaces.join(", "));
        }
        println!("  nm wireless disabled:   {}", k.nm_wireless_disabled);
        println!("  nm wwan disabled:       {}", k.nm_wwan_disabled);
        println!("  bluetooth disabled:     {}", k.bluetooth_disabled);
        println!("\nrun `sudo proteus resume --yes` to restore.");
    } else {
        println!("kill switch: inactive");
    }
    Ok(exit::SUCCESS)
}

/// Public entry point for `proteus resume [--yes]`.
pub fn resume_run(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "resume re-enables network traffic and radios",
        "proteus wiki kill-switch",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if !state.kill_switch.active {
        println!("kill switch not active; nothing to resume");
        return Ok(exit::SUCCESS);
    }

    let mut warns: Vec<String> = Vec::new();
    for iface in &state.kill_switch.interfaces {
        match kill_switch::link_up(iface) {
            Ok(true) => println!("interface {iface}: up"),
            Ok(false) => warns.push(format!(
                "interface {iface}: `ip` not found in PATH; install iproute2"
            )),
            Err(e) => warns.push(format!("interface {iface}: {e}")),
        }
    }

    if state.kill_switch.nm_wireless_disabled || state.kill_switch.nm_wwan_disabled {
        let r = toggle_radios(
            true,
            state.kill_switch.nm_wireless_disabled,
            state.kill_switch.nm_wwan_disabled,
        );
        for w in &r.warns {
            warns.push(format!("nm: {w}"));
        }
        if r.wireless_changed {
            println!("nm: wireless radio on");
        }
        if r.wwan_changed {
            println!("nm: wwan radio on");
        }
    }
    if state.kill_switch.bluetooth_disabled {
        let bt = set_bluetooth_powered(true);
        for w in &bt.warns {
            warns.push(format!("bluetooth: {w}"));
        }
        if bt.toggled {
            println!("bluetooth: adapters powered on");
        }
    }

    state.kill_switch = KillSwitchState::default();
    state.save(&state_path)?;

    if warns.is_empty() {
        println!("kill switch: cleared");
    } else {
        eprintln!("kill switch: cleared with {} warning(s):", warns.len());
        for w in &warns {
            eprintln!("  {w}");
        }
        eprintln!("see `proteus wiki kill-switch` for manual recovery");
    }
    Ok(exit::SUCCESS)
}

/// Outcome of a single radio-toggle pass. `wireless_changed` and
/// `wwan_changed` are direction-agnostic — the caller asked for off (kill)
/// or on (resume); the flag reports "we actually flipped this property".
#[derive(Debug, Default)]
struct RadioOutcome {
    wireless_changed: bool,
    wwan_changed: bool,
    warns: Vec<String>,
}

#[derive(Debug, Default)]
struct BluetoothKillOutcome {
    /// True if at least one adapter's Powered property was actually toggled.
    toggled: bool,
    warns: Vec<String>,
}

/// Run `f` on a fresh single-thread tokio runtime. Returns `Err` only if the
/// runtime itself failed to build; callers handle the resulting warning.
fn block_on_async<F, T>(label: &str, f: F) -> std::result::Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .with_context(|| format!("starting tokio runtime for {label}"))
    {
        Ok(rt) => Ok(rt.block_on(f)),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// Toggle NetworkManager's `WirelessEnabled` / `WwanEnabled` properties.
/// `target` is the desired state (`false` for kill, `true` for resume).
/// Best-effort: a missing NM is not an error — the system may not run NM.
/// `do_wireless` / `do_wwan` mirror the asymmetry between kill (touch both
/// radios) and resume (touch only the radios the kill snapshot recorded as
/// disabled, so we never re-enable a radio the user had off pre-kill).
fn toggle_radios(target: bool, do_wireless: bool, do_wwan: bool) -> RadioOutcome {
    let label = if target {
        "NM radio re-enable"
    } else {
        "NM radio disable"
    };
    let mut out = RadioOutcome::default();
    match block_on_async(label, async move {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(_) => {
                return RadioOutcome {
                    warns: vec!["system DBus unavailable; skipped radio toggle".into()],
                    ..Default::default()
                };
            }
        };
        let proxy = match NmRadioProxy::new(&conn).await {
            Ok(p) => p,
            Err(_) => {
                return RadioOutcome {
                    warns: vec!["NetworkManager not on bus; skipped radio toggle".into()],
                    ..Default::default()
                };
            }
        };
        let mut o = RadioOutcome::default();
        if do_wireless {
            apply_radio(
                "wireless",
                target,
                proxy.wireless_enabled().await,
                || proxy.set_wireless_enabled(target),
                &mut o.wireless_changed,
                &mut o.warns,
            )
            .await;
        }
        if do_wwan {
            apply_radio(
                "wwan",
                target,
                proxy.wwan_enabled().await,
                || proxy.set_wwan_enabled(target),
                &mut o.wwan_changed,
                &mut o.warns,
            )
            .await;
        }
        o
    }) {
        Ok(o) => o,
        Err(e) => {
            out.warns.push(e);
            out
        }
    }
}

/// Issue the radio setter only when the current value differs from the
/// target — saves a redundant DBus round-trip when the radio is already in
/// the requested state. The "already disabled" warning is kill-only;
/// resume callers filter to radios they themselves disabled, so the
/// condition is silent there.
async fn apply_radio<F, Fut>(
    name: &str,
    target: bool,
    current: zbus::Result<bool>,
    set: F,
    changed: &mut bool,
    warns: &mut Vec<String>,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = zbus::Result<()>>,
{
    match current {
        Ok(c) if c == target => {
            if !target {
                warns.push(format!("{name} already disabled"));
            }
            return;
        }
        Ok(_) => {}
        Err(e) => {
            warns.push(format!("{name} property read: {e}"));
            return;
        }
    }
    match set().await {
        Ok(()) => *changed = true,
        Err(e) => warns.push(format!("{name}: {e}")),
    }
}

fn set_bluetooth_powered(on: bool) -> BluetoothKillOutcome {
    let mut out = BluetoothKillOutcome::default();
    let res: Result<BluetoothKillOutcome> = match block_on_async("BlueZ toggle", async move {
        let Some((conn, adapters)) = bluetooth::connect_and_list().await? else {
            return Ok(BluetoothKillOutcome {
                toggled: false,
                warns: vec!["BlueZ not detected; skipped".into()],
            });
        };
        let mut o = BluetoothKillOutcome::default();
        for a in &adapters {
            match toggle_adapter(&conn, a, on).await {
                Ok(true) => o.toggled = true,
                Ok(false) => {}
                Err(e) => o.warns.push(format!("{}: {e:#}", a.hci)),
            }
        }
        Ok(o)
    }) {
        Ok(r) => r,
        Err(e) => {
            out.warns.push(e);
            return out;
        }
    };
    match res {
        Ok(o) => o,
        Err(e) => {
            out.warns.push(format!("{e:#}"));
            out
        }
    }
}

async fn toggle_adapter(
    conn: &zbus::Connection,
    info: &AdapterInfo,
    on: bool,
) -> Result<bool, anyhow::Error> {
    use crate::bluetooth::Adapter1Proxy;
    let proxy = Adapter1Proxy::builder(conn)
        .path(info.path.clone())?
        .build()
        .await?;
    // Skip the write when the property already matches — avoids noisy
    // PropertiesChanged traffic on resume when the adapter never had RF on
    // to begin with.
    let current = proxy.powered().await.unwrap_or(false);
    if current == on {
        return Ok(false);
    }
    // The Adapter1 proxy elsewhere in the codebase only exposes `Powered` as
    // a getter; rather than extend that shared trait just for this command,
    // go through the generic Properties.Set interface.
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination("org.bluez")?
        .path(info.path.clone())?
        .build()
        .await?;
    props
        .set(
            "org.bluez.Adapter1".try_into()?,
            "Powered",
            zbus::zvariant::Value::Bool(on),
        )
        .await
        .with_context(|| format!("setting Powered={on} on {}", info.hci))?;
    Ok(true)
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(super::now_iso8601());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_state_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("proteus-kill-cmd-test-{tag}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Issue #235: `kill_status` now requires root to match reality
    /// (state.json is mode 0600 in /var/lib/proteus mode 0700). The
    /// cargo-test process is non-root, so the gate returns
    /// `PERMISSION_ERROR` before the state file is even read; the
    /// test pins the gate's existence and exit code.
    #[test]
    fn status_returns_permission_error_when_not_root() {
        let path = temp_state_path("status-clean");
        let code = kill_status(true, Some(&path)).unwrap();
        assert_eq!(code, exit::PERMISSION_ERROR);
        let _ = std::fs::remove_file(&path);
    }

    /// Issue #235: even when the operator lays down a populated
    /// state.json, the non-root path returns `PERMISSION_ERROR` (the
    /// gate fires before `State::load`). State round-trip is exercised
    /// elsewhere — this test just pins the privilege check.
    #[test]
    fn status_returns_permission_error_with_populated_state() {
        let path = temp_state_path("status-active");
        let s = State {
            kill_switch: KillSwitchState {
                active: true,
                activated_at: Some("2026-05-06T00:00:00Z".to_string()),
                interfaces: vec!["wlan0".to_string()],
                nm_wireless_disabled: true,
                nm_wwan_disabled: false,
                bluetooth_disabled: true,
            },
            ..State::default()
        };
        s.save(&path).unwrap();
        let code = kill_status(true, Some(&path)).unwrap();
        assert_eq!(code, exit::PERMISSION_ERROR);

        // State round-trip is the responsibility of state.rs tests; we
        // only verify the file the production path would have read is
        // still parseable.
        let back = State::load_or_default(&path).unwrap();
        assert!(back.kill_switch.active);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn kill_without_yes_returns_confirmation_required_exit() {
        let path = temp_state_path("kill-noyes");
        let code = kill_run(false, Some(&path)).unwrap();
        // CONFIRMATION_REQUIRED (65) — the "needs --yes" sentinel shared by
        // every mutating subcommand. Issue #117 moved this off the legacy
        // NOT_IMPLEMENTED (64), which had meant "the feature has not landed
        // yet" and misled wrappers.
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resume_without_yes_returns_confirmation_required_exit() {
        let path = temp_state_path("resume-noyes");
        let code = resume_run(false, Some(&path)).unwrap();
        assert_eq!(code, exit::CONFIRMATION_REQUIRED);
        let _ = std::fs::remove_file(&path);
    }
}
