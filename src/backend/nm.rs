// SPDX-License-Identifier: GPL-3.0-or-later

//! `backend::nm` — wraps the existing `crate::nm::*` zbus calls behind
//! the [`NetworkBackend`] trait. No behaviour change vs the pre-trait
//! call sites; the rest of Milestone 1 routes
//! `src/commands/{rotate,dhcp,ipv6,enterprise_wifi}.rs` through this
//! impl so non-NM backends can drop in.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

use anyhow::{Context, Result, anyhow};
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use super::{
    BackendDevice, BackendKind, BoxFuture, ConnectionRef, NetworkBackend, RenewOutcome,
    RotateOutcome,
};
use crate::ipv6::nm::Ipv6NmSettings;
use crate::mac::{Mac, factory};
use crate::nm::{self, ConnectionSettings, DeviceKind};
use crate::state::DhcpSettingsSnapshot;

/// N14: process-wide registry of per-iface async mutexes used to
/// serialise concurrent `rotate_if_needed` invocations against the same
/// interface. Different interfaces still rotate in parallel.
///
/// Why a separate per-iface mutex (instead of relying on `state_lock`):
/// the state lock's process-wide `Mutex<Option<File>>` is reentrant —
/// the first acquire in this process holds the kernel flock; every
/// subsequent acquire from the same process returns a no-op guard. That
/// keeps the orchestrator pattern (`apply::run` → `rotate::run`) safe,
/// but it also means two parallel tokio tasks on a multi-thread runtime
/// can BOTH pass through the lock acquire and proceed to the cooldown
/// read without observing each other's `last_rotated` write. The
/// in-process reentrancy that protects nested calls in the same task
/// becomes a hole for true parallel tasks: both see "no cooldown" and
/// both fall through to `rotate_hook`, double-rotating inside the
/// cooldown window.
///
/// The per-iface mutex closes that hole. We acquire it BEFORE the
/// state-lock acquire, so:
///   1. Same-iface concurrent rotates serialise on this mutex. The
///      second waiter blocks until the first releases — by which point
///      `last_rotated` has been written. The second waiter then runs
///      the cooldown read, sees the stamp, and returns
///      `SkippedCooldown`. No double rotate.
///   2. Different-iface concurrent rotates take different keys in the
///      map and run in parallel (each on its own iface's mutex).
///   3. The state lock continues to do its job: cross-process serial-
///      isation and intra-process reentrancy for nested same-task calls.
///
/// `tokio::sync::Mutex` (vs `std::sync::Mutex`) so the guard is `Send`
/// and can be held across the await points in the rotate body. The
/// registry itself is a plain `std::sync::Mutex<HashMap<...>>` because
/// we never hold it across an await — only across the
/// HashMap lookup/insert, which is sync.
fn iface_rotate_mutex(iface: &str) -> Arc<AsyncMutex<()>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = registry.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(iface.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// How long an `available()` answer stays valid before the backend
/// re-probes. NBE.2: `select_auto` and `availability_matrix` (and any
/// caller doing back-to-back `available` checks within one command
/// invocation) used to syscall `/run/systemd/netif` per call. The TTL
/// is intentionally short — long enough that two consecutive trait
/// calls share one probe, short enough that an operator who restarts
/// NetworkManager mid-command sees the new state on the next pass.
const AVAIL_CACHE_TTL: Duration = Duration::from_secs(2);

/// NetworkManager-backed implementation.
///
/// NBE.1: previously every trait method opened a fresh
/// `zbus::Connection::system()`, which re-authenticates against the
/// daemon and pays the full SASL handshake on each call. A single
/// `proteus rotate` invocation hits the backend ~6-10 times (list_devices,
/// list_connections, read_connection_id/uuid per profile, set_cloned_mac,
/// state book-keeping) so the cumulative cost was visible in the rotate
/// hot path. The cached `Arc<Connection>` is opened lazily on first use
/// and re-used by every subsequent call on the same backend value;
/// `Arc<Connection>` is the same shape `zbus` itself uses internally,
/// so cloning it is a cheap atomic refcount bump.
///
/// NBE.2: the `available()` answer is cached behind the same backend
/// value with a short TTL so `select_auto` (NM probe → networkd probe →
/// raw probe) and `availability_matrix` (every backend probed for the
/// doctor matrix) don't re-syscall on back-to-back checks.
#[derive(Default)]
pub struct NmBackend {
    /// Lazily-initialised system bus connection. `None` until the first
    /// real DBus method is called; once filled, every subsequent call
    /// shares the same connection. `tokio::sync::Mutex` because the
    /// initialisation itself is async (the `Connection::system()` call).
    conn: AsyncMutex<Option<Arc<zbus::Connection>>>,
    /// Cached `available()` decision + the wall-clock `Instant` it was
    /// taken. The TTL is `AVAIL_CACHE_TTL`; `None` means "never probed".
    avail_cache: AsyncMutex<Option<(bool, Instant)>>,
}

// Hand-written Debug because `tokio::sync::Mutex` is not `Debug` on its
// inner cell content — we just want an opaque marker so `#[derive(Debug)]`
// on consumers (like the trait-object placeholder) doesn't break.
impl std::fmt::Debug for NmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NmBackend").finish_non_exhaustive()
    }
}

impl NmBackend {
    pub fn new() -> Self {
        Self {
            conn: AsyncMutex::new(None),
            avail_cache: AsyncMutex::new(None),
        }
    }

    /// Return the cached system bus connection, opening + caching it on
    /// first use. NBE.1: every NM trait method routes through this
    /// rather than calling `zbus::Connection::system()` directly so a
    /// single `proteus rotate` invocation re-uses one authenticated
    /// connection across the ~10 trait calls it makes.
    async fn shared_conn(&self) -> Result<Arc<zbus::Connection>> {
        let mut guard = self.conn.lock().await;
        if let Some(c) = guard.as_ref() {
            return Ok(Arc::clone(c));
        }
        let c = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager)")?;
        let arc = Arc::new(c);
        *guard = Some(Arc::clone(&arc));
        Ok(arc)
    }
}

impl NetworkBackend for NmBackend {
    fn name(&self) -> &'static str {
        "nm"
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, bool> {
        // /run/NetworkManager and /var/run/NetworkManager are the same
        // signal `commands::status::detect_system` uses; cheap and
        // mirrors what the user already sees in `proteus status`.
        //
        // NBE.2: cache the answer for `AVAIL_CACHE_TTL` so back-to-back
        // probes (auto-select + availability_matrix in `proteus doctor`,
        // or two trait callers in one rotate) share one syscall pair.
        // The TTL is short enough that an operator restarting NM mid-
        // run sees the change in their next command.
        Box::pin(async move {
            let mut guard = self.avail_cache.lock().await;
            if let Some((value, taken)) = *guard
                && taken.elapsed() < AVAIL_CACHE_TTL
            {
                return value;
            }
            let v = Path::new("/run/NetworkManager").exists()
                || Path::new("/var/run/NetworkManager").exists();
            *guard = Some((v, Instant::now()));
            v
        })
    }

    fn list_devices<'a>(&'a self) -> BoxFuture<'a, Result<Vec<BackendDevice>>> {
        Box::pin(async move {
            let conn = self.shared_conn().await?;
            let devs = nm::list_devices(&conn).await?;
            let out = devs
                .into_iter()
                .map(|d| BackendDevice {
                    iface: d.interface,
                    kind: kind_from_nm(d.kind),
                    hw_address: d.hw_address,
                    // `identifier` is the NM Device object path — used
                    // by `renew_lease`. The connection-keyed mutators
                    // (set_cloned_mac, set_dhcp_settings, ...) iterate
                    // `connections` instead.
                    identifier: d.path.as_str().to_string(),
                    connections: d
                        .connections
                        .iter()
                        .map(|p| ConnectionRef::new(p.as_str()))
                        .collect(),
                    managed: d.managed,
                })
                .collect();
            Ok(out)
        })
    }

    fn list_connections<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Vec<ConnectionRef>>> {
        // The NM impl has the connections cached on the device value
        // already (populated in `list_devices`); just hand them back.
        let out = device.connections.clone();
        Box::pin(async move { Ok(out) })
    }

    fn set_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
        mac: Mac,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let kind = kind_to_nm(device.kind).ok_or_else(|| {
                anyhow!(
                    "backend::nm: device kind {:?} has no cloned-MAC setting",
                    device.kind
                )
            })?;
            let conn = self.shared_conn().await?;
            // NBE.8: re-resolve the device by iface name on every
            // mutating call so the connection-profile list reflects the
            // CURRENT NM state. The `device.connections` cached on the
            // value passed in was captured by `list_devices` earlier in
            // the command and may be stale if NM added or removed a
            // profile in the interim (e.g. nmcli connection add raced
            // against the rotate). Falling back to the cached list
            // when the re-resolve cannot find the device keeps
            // operator-friendly errors on the unlikely "device removed"
            // race.
            let connections = match nm::find_device_by_iface(&conn, &device.iface).await {
                Ok(d) => d
                    .connections
                    .iter()
                    .map(|p| ConnectionRef::new(p.as_str()))
                    .collect::<Vec<_>>(),
                Err(_) => device.connections.clone(),
            };
            if connections.is_empty() {
                return Err(anyhow!(
                    "backend::nm: device {} has no NM connection profile",
                    device.iface
                ));
            }
            // Issue #122: write to every profile, not just the first.
            for cref in &connections {
                let path = parse_connection_ref(cref)?;
                nm::apply::set_cloned_mac(&conn, &path, kind, mac).await?;
            }
            Ok(())
        })
    }

    fn read_cloned_mac<'a>(
        &'a self,
        device: &'a BackendDevice,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let kind = match kind_to_nm(device.kind) {
                Some(k) => k,
                None => return Ok(None),
            };
            let conn = self.shared_conn().await?;
            // NBE.8: re-resolve by iface so a stale `device.connections`
            // list doesn't mask a connection NM has since added.
            let cref_opt = match nm::find_device_by_iface(&conn, &device.iface).await {
                Ok(d) => d
                    .connections
                    .first()
                    .map(|p| ConnectionRef::new(p.as_str())),
                Err(_) => device.connections.first().cloned(),
            };
            let Some(cref) = cref_opt else {
                return Ok(None);
            };
            let path = parse_connection_ref(&cref)?;
            nm::apply::read_cloned_mac(&conn, &path, kind).await
        })
    }

    fn read_factory_mac<'a>(&'a self, iface: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        // Same source `commands::rotate` and `commands::status` already
        // use; the trait method just routes through it so non-NM
        // backends can share the read path verbatim.
        Box::pin(async move { Ok(factory::permanent_address(iface)) })
    }

    fn rotate_if_needed<'a>(
        &'a self,
        iface: &'a str,
        cooldown: Duration,
        state_path: Option<&'a std::path::Path>,
    ) -> BoxFuture<'a, Result<RotateOutcome>> {
        // Issue #206-C: structured entry point used by the NM
        // dispatcher in place of the previous `proteus current --json | sed`
        // grep. The dispatcher path stays read-mostly for backwards
        // compatibility — the cooldown decision lives here, the actual
        // mutation is delegated to `commands::rotate::run` (no DBus
        // spelling out).
        //
        // GH#381: honor the operator-supplied `--state` path. Stream 1
        // surfaced a warn for the ignored arg; the actual fix needed
        // a backend-trait change, threaded through here so the cooldown
        // read AND the inner rotate book-keeping land on the same file.
        Box::pin(async move { rotate_if_needed_inner(iface, cooldown, state_path).await })
    }

    fn read_connection_id<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = self.shared_conn().await?;
            nm::apply::read_connection_id(&conn, &path).await
        })
    }

    fn read_connection_uuid<'a>(
        &'a self,
        connection: &'a ConnectionRef,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = self.shared_conn().await?;
            nm::apply::read_connection_uuid(&conn, &path).await
        })
    }

    fn set_dhcp_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        snapshot: DhcpSettingsSnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = self.shared_conn().await?;
            // Read current settings, write the snapshot's keys, push
            // back via the secrets-aware updater. `revert_dhcp_settings`
            // is the same routine the per-command revert path uses;
            // here it does the apply direction by writing the desired
            // snapshot directly onto the live settings.
            let mut settings: ConnectionSettings = nm::dhcp::get_settings(&conn, &path).await?;
            nm::dhcp::revert_dhcp_settings(&mut settings, &snapshot)?;
            nm::dhcp::update_connection(&conn, &path, settings).await
        })
    }

    fn set_ipv6_settings<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        settings: Ipv6NmSettings,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = self.shared_conn().await?;
            crate::ipv6::nm::apply_settings(&conn, &path, &settings).await
        })
    }

    fn renew_lease<'a>(&'a self, device: &'a BackendDevice) -> BoxFuture<'a, Result<RenewOutcome>> {
        Box::pin(async move {
            let conn = self.shared_conn().await?;
            // NBE.8: re-resolve the NM Device object path by iface name
            // rather than trusting `device.identifier`. NM can recycle a
            // device path on a `udev` add/remove (USB Wi-Fi unplug-replug,
            // a netns move) so the path captured at `list_devices` time
            // may now point to a different device or be stale. Falling
            // back to the cached identifier preserves the prior behaviour
            // when the iface lookup fails (e.g. NM is mid-restart).
            let owned: OwnedObjectPath = match nm::find_device_by_iface(&conn, &device.iface).await
            {
                Ok(d) => d.path,
                Err(_) => {
                    if device.identifier.is_empty() {
                        return Err(anyhow!(
                            "backend::nm: device {} has no NM device path",
                            device.iface
                        ));
                    }
                    let dev_path =
                        ObjectPath::try_from(device.identifier.as_str()).with_context(|| {
                            format!("parsing NM device path '{}'", device.identifier)
                        })?;
                    dev_path.into()
                }
            };
            let outcome = nm::dhcp::renew_lease(&conn, &owned).await?;
            Ok(match outcome {
                nm::dhcp::RenewOutcome::Reapplied => RenewOutcome::Reapplied,
                nm::dhcp::RenewOutcome::DisconnectActivated => RenewOutcome::DisconnectActivated,
                nm::dhcp::RenewOutcome::NoActiveConnection => RenewOutcome::NoActiveConnection,
            })
        })
    }

    fn write_anonymous_identity<'a>(
        &'a self,
        connection: &'a ConnectionRef,
        value: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let path = parse_connection_ref(connection)?;
            let conn = self.shared_conn().await?;
            crate::enterprise_wifi::nm::write_anonymous_identity(&conn, &path, value).await
        })
    }
}

/// The cooldown-check + rotate body for [`NmBackend::rotate_if_needed`],
/// pulled out so it can be unit-tested without going through the trait
/// object.
///
/// Issue #245: the previous shape was TOCTOU — concurrent dispatcher
/// events would both pass the cooldown check (each reading
/// `last_rotated` before either had updated it), then both fall through
/// to `commands::rotate::run` and double-rotate inside the cooldown
/// window. The new shape acquires the state lock at the entry, reads
/// cooldown state under the lock, and (if rotating) keeps the lock held
/// across the inner `rotate::run` call. The lock is process-reentrant
/// (`state_lock` returns a no-op guard for nested calls in the same
/// process) so the inner command's own acquire is harmless. Two
/// concurrent invocations now serialise: the first runs the rotate, the
/// second wakes up holding the lock and observes `last_rotated` already
/// updated → returns `SkippedCooldown`.
///
/// N14 (Stream 4): the #245 fix closed the cross-process race but not
/// the in-process parallel-task race. The state lock's
/// `Mutex<Option<File>>` is reentrant — two parallel tokio tasks on a
/// multi-thread runtime can both observe a populated slot and proceed
/// (one as the real holder, the other with a no-op guard) without
/// observing each other's `last_rotated` write. The follow-up wraps a
/// per-iface async mutex (`iface_rotate_mutex`) around the entire
/// cooldown + state-lock + rotate sequence so concurrent rotates
/// against the SAME interface serialise; different interfaces still
/// rotate in parallel.
///
/// Issue #250: the read-back path used to fall through to
/// `Mac::new([0; 6])` when the freshly-written `state.json` couldn't be
/// reloaded or when the parse failed. Reporting `Rotated { new_mac:
/// 00:00:00:00:00:00 }` to the dispatcher made the operator think the
/// rotation produced a known-bad MAC. The new shape returns an `Err`
/// that the caller surfaces with the trail "rotate succeeded but state
/// read-back failed" so the operator can see the rotate landed and the
/// only thing missing is the post-mutation observation.
async fn rotate_if_needed_inner(
    iface: &str,
    cooldown: Duration,
    state_path: Option<&std::path::Path>,
) -> Result<RotateOutcome> {
    // GH#381: use the operator-supplied state path when present, falling
    // back to the documented default. Pre-fix, the default was
    // hardcoded so `rotate-if-needed --state /tmp/altered.json` read
    // cooldown from the default file, ignored the operator's choice,
    // and let the inner `commands::rotate::run` write the new MAC to
    // the alternate path — leaving the two views permanently
    // out-of-sync.
    let owned_default;
    let state_path: &std::path::Path = match state_path {
        Some(p) => p,
        None => {
            owned_default = std::path::PathBuf::from(crate::commands::DEFAULT_STATE_PATH);
            owned_default.as_path()
        }
    };
    rotate_if_needed_inner_with(iface, cooldown, state_path, |iface, sp| {
        // Production rotate: the existing CLI helper. Sync — builds its
        // own current-thread runtime; called from inside our outer
        // runtime via the same shape `commands::rotate::run_if_needed`
        // already uses (the previous trait method called it the same
        // way before issue #245's fix). The inner acquire is reentrant
        // and the lock we hold here makes it a no-op.
        crate::commands::rotate::run(Some(iface), true, false, Some(sp), None)
    })
    .await
}

/// Generic form of [`rotate_if_needed_inner`] that takes the state path
/// and the rotate hook as parameters. Exists so the unit tests can
/// drive the cooldown / lock / read-back logic against a tempdir state
/// file and a synthetic rotate that just bumps `last_rotated` — no NM,
/// no DBus.
///
/// `rotate_hook` returns `Result<u8>` matching the CLI exit code
/// contract `commands::rotate::run` uses (`SUCCESS == 0` means rotated;
/// anything else is treated as a backend-unavailable signal). When the
/// hook returns `SUCCESS` the function reads `state.json` back under
/// the same lock to surface the new MAC.
async fn rotate_if_needed_inner_with<F>(
    iface: &str,
    cooldown: Duration,
    state_path: &std::path::Path,
    mut rotate_hook: F,
) -> Result<RotateOutcome>
where
    F: FnMut(&str, &std::path::Path) -> Result<u8>,
{
    // N14: per-iface serialisation. Acquire the per-iface async mutex
    // BEFORE the state lock so two concurrent rotates against the SAME
    // interface (e.g. a per-SSID policy fires via the events daemon at
    // the same moment an operator types `proteus rotate`) wait their
    // turn. The state lock alone is insufficient: it is reentrant
    // within a process, so parallel tokio tasks both observe `HELD =
    // Some(..)` for one task and `outermost = false` for the other,
    // then both fall through to the cooldown read with stale state and
    // both rotate. The per-iface mutex makes the second task wait
    // until the first commits `last_rotated`; the second then reads
    // the stamp and returns `SkippedCooldown`. Different ifaces hash
    // to different keys in the registry and rotate in parallel.
    let iface_mutex = iface_rotate_mutex(iface);
    let _iface_guard = iface_mutex.lock().await;

    // Acquire the state lock for the duration of the cooldown decision
    // AND the inner rotate. Held = no other proteus process can rotate
    // this iface; nested = the inner `rotate::run` sees the in-process
    // slot already populated and returns a no-op guard.
    //
    // We don't use `commands::acquire_state_lock_or_print` here because
    // we never want to print: the trait method bubbles a structured
    // outcome and the caller decides what the operator sees.
    let _guard = match crate::state_lock::acquire_for_state_path(state_path) {
        Ok(g) => g,
        Err(crate::state_lock::LockError::Busy { .. }) => {
            // Another rotate / apply / pin is mid-flight. The dispatcher
            // will retry on the next event; surface this as a cooldown
            // skip with a tiny remaining window so the dispatcher's
            // existing "skipped: cooldown 0s" log line covers it.
            return Ok(RotateOutcome::SkippedCooldown {
                remaining: Duration::from_secs(1),
            });
        }
        Err(_) => return Ok(RotateOutcome::BackendUnavailable),
    };

    let state = match crate::state::State::load_or_default(state_path) {
        Ok(s) => s,
        Err(_) => return Ok(RotateOutcome::BackendUnavailable),
    };
    // Cooldown check under the lock: read the per-iface `last_rotated`
    // and bail structured if the elapsed time hasn't met the budget
    // yet. The elapsed/now arithmetic is racy with wall-clock skew but
    // not with concurrent rotates anymore.
    if let Some(rec) = state.managed.interfaces.get(iface)
        && let Some(stamp) = rec.last_rotated.as_deref()
        && let Some(remaining) = remaining_cooldown(stamp, cooldown)
    {
        return Ok(RotateOutcome::SkippedCooldown { remaining });
    }
    // Factory MAC must be on file before we ever rotate; the
    // sacred-originals invariant in `commands::rotate` saves it
    // mid-run, but `rotate-if-needed` is meant to be called by
    // the dispatcher BEFORE the first rotation, so we check
    // here too. Returning `NoFactoryMac` lets the dispatcher
    // log a clear "this driver doesn't expose a factory MAC,
    // skipping" rather than a generic NM error.
    if factory::permanent_address(iface).is_none() && !state.original_macs.contains_key(iface) {
        return Ok(RotateOutcome::NoFactoryMac);
    }
    // Delegate to the rotate hook. The inner call will try to acquire
    // the same lock; the in-process reentrancy guard makes that a
    // no-op. We don't pass the result through Result<u8> because
    // that's the CLI exit code; the typed outcome here is "rotated"
    // iff the call succeeded.
    let res = rotate_hook(iface, state_path);
    match res {
        Ok(c) if c == crate::exit::SUCCESS => {
            // Read back the new MAC the rotation just wrote. The lock
            // is still held, so the file we just saved IS the file we
            // read here — no race with another mutator.
            let new_state = crate::state::State::load_or_default(state_path).map_err(|e| {
                anyhow!(
                    "rotate succeeded on {iface} but state read-back failed: {e:#}; \
                     check {} for write permissions",
                    state_path.display()
                )
            })?;
            let mac_str = new_state
                .managed
                .interfaces
                .get(iface)
                .and_then(|r| r.current_mac.as_deref())
                .ok_or_else(|| {
                    anyhow!(
                        "rotate succeeded on {iface} but state read-back found no \
                         current_mac entry under managed.interfaces.{iface} in {}",
                        state_path.display()
                    )
                })?;
            let new_mac = mac_str.parse::<Mac>().map_err(|e| {
                anyhow!(
                    "rotate succeeded on {iface} but state read-back returned an \
                     unparseable MAC '{mac_str}': {e}"
                )
            })?;
            Ok(RotateOutcome::Rotated { new_mac })
        }
        Ok(_) | Err(_) => Ok(RotateOutcome::BackendUnavailable),
    }
}

fn kind_from_nm(k: DeviceKind) -> BackendKind {
    match k {
        DeviceKind::Wifi => BackendKind::Wifi,
        DeviceKind::Ethernet => BackendKind::Ethernet,
        DeviceKind::Other(_) => BackendKind::Other,
    }
}

fn kind_to_nm(k: BackendKind) -> Option<DeviceKind> {
    match k {
        BackendKind::Wifi => Some(DeviceKind::Wifi),
        BackendKind::Ethernet => Some(DeviceKind::Ethernet),
        BackendKind::Other => None,
    }
}

fn parse_connection_ref(cref: &ConnectionRef) -> Result<OwnedObjectPath> {
    let s = cref.as_str();
    if s.is_empty() {
        return Err(anyhow!(
            "backend::nm: connection has no NM dbus path (empty identifier)"
        ));
    }
    let p = ObjectPath::try_from(s).with_context(|| format!("parsing NM connection path '{s}'"))?;
    Ok(p.into())
}

/// Compute remaining cooldown given an ISO-8601 `last_rotated` stamp
/// and a budget. Returns `None` if the cooldown has expired (or the
/// stamp couldn't be parsed) — both cases mean "go ahead and rotate".
fn remaining_cooldown(stamp: &str, cooldown: Duration) -> Option<Duration> {
    let last = parse_iso8601_z(stamp)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let elapsed = now.saturating_sub(last);
    if elapsed >= cooldown.as_secs() {
        None
    } else {
        Some(Duration::from_secs(cooldown.as_secs() - elapsed))
    }
}

/// Hand-rolled inverse of `commands::now_iso8601` — accepts the
/// exact `YYYY-MM-DDTHH:MM:SSZ` shape that helper writes. Returns the
/// Unix epoch second on success, `None` on any parse failure (callers
/// treat that as "no cooldown known").
fn parse_iso8601_z(stamp: &str) -> Option<u64> {
    if stamp.len() != 20 || !stamp.ends_with('Z') {
        return None;
    }
    let y: u32 = stamp[0..4].parse().ok()?;
    let mo: u32 = stamp[5..7].parse().ok()?;
    let d: u32 = stamp[8..10].parse().ok()?;
    let h: u32 = stamp[11..13].parse().ok()?;
    let mi: u32 = stamp[14..16].parse().ok()?;
    let s: u32 = stamp[17..19].parse().ok()?;
    Some(ymdhms_to_unix(y, mo, d, h, mi, s))
}

/// Civil-from-days inverse, mirroring `commands::unix_to_ymdhms`.
fn ymdhms_to_unix(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    let y = y as i64 - if mo <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let secs = days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64;
    secs.max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trip_wifi_and_ethernet() {
        assert_eq!(kind_from_nm(DeviceKind::Wifi), BackendKind::Wifi);
        assert_eq!(kind_from_nm(DeviceKind::Ethernet), BackendKind::Ethernet);
        assert_eq!(kind_from_nm(DeviceKind::Other(7)), BackendKind::Other);

        assert_eq!(kind_to_nm(BackendKind::Wifi), Some(DeviceKind::Wifi));
        assert_eq!(
            kind_to_nm(BackendKind::Ethernet),
            Some(DeviceKind::Ethernet)
        );
        assert_eq!(kind_to_nm(BackendKind::Other), None);
    }

    #[test]
    fn parse_connection_ref_rejects_empty() {
        assert!(parse_connection_ref(&ConnectionRef::new("")).is_err());
    }

    #[test]
    fn parse_connection_ref_accepts_well_formed_path() {
        let p = parse_connection_ref(&ConnectionRef::new(
            "/org/freedesktop/NetworkManager/Settings/3",
        ))
        .expect("valid dbus path");
        assert_eq!(p.as_str(), "/org/freedesktop/NetworkManager/Settings/3");
    }

    #[test]
    fn name_is_stable_token() {
        assert_eq!(NmBackend::new().name(), "nm");
    }

    #[test]
    fn parse_iso8601_z_rejects_garbage() {
        assert!(parse_iso8601_z("").is_none());
        assert!(parse_iso8601_z("nope").is_none());
        // Wrong length / missing Z.
        assert!(parse_iso8601_z("2026-05-07T12:00:00").is_none());
        assert!(parse_iso8601_z("2026-05-07T12:00:00X").is_none());
    }

    #[test]
    fn parse_iso8601_z_round_trips_unix_to_ymdhms() {
        // Use the existing forward helper to build a stamp at a known
        // epoch, then round trip back through the inverse.
        let secs: u64 = 1_710_000_000; // 2024-03-09 16:00 UTC
        let (y, mo, d, h, mi, s) = crate::commands::unix_to_ymdhms(secs);
        let stamp = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
        let parsed = parse_iso8601_z(&stamp).unwrap();
        assert_eq!(parsed, secs);
    }

    #[test]
    fn remaining_cooldown_none_when_stamp_in_past() {
        // A stamp from year 2000 is far past any reasonable cooldown.
        let r = remaining_cooldown("2000-01-01T00:00:00Z", Duration::from_secs(60));
        assert!(r.is_none());
    }

    // ===== Issue #245 / #250 — TOCTOU + read-back regressions =====

    use crate::state::{InterfaceRecord, State};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn fresh_state_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("proteus-nm-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Issue #245: when `last_rotated` is recent, the cooldown branch
    /// must trip BEFORE the rotate hook fires. The hook would have
    /// rotated otherwise — assertion would catch a regression of the
    /// pre-fix shape that called the rotate path twice in tight succession.
    #[test]
    fn rotate_if_needed_skips_within_cooldown() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("cooldown");
        let state_path = dir.join("state.json");

        // Seed `state.json` with a recent rotation stamp.
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        let rec = InterfaceRecord {
            last_rotated: Some(crate::commands::now_iso8601()),
            ..Default::default()
        };
        state.managed.interfaces.insert("wlan0".into(), rec);
        state.save(&state_path).unwrap();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_in = calls.clone();
        let outcome = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(3600),
                &state_path,
                move |_iface, _sp| {
                    calls_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
            .unwrap()
        });
        assert!(matches!(outcome, RotateOutcome::SkippedCooldown { .. }));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "cooldown hit must not invoke the rotate hook"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #245 regression: the cooldown decision MUST happen under
    /// the state lock so a concurrent `rotate-if-needed` invocation
    /// (the dispatcher fires one per `state == "connected"` event)
    /// can't pass the cooldown check before the in-flight rotate
    /// commits its `last_rotated` stamp. Pre-fix, the lock was acquired
    /// only inside `commands::rotate::run`; the cooldown read happened
    /// before the lock, so two near-simultaneous dispatcher events
    /// both saw "no cooldown" and both rotated.
    ///
    /// The shape: assert the lock IS held during the rotate hook (so
    /// the cooldown read is also under the lock — they're collocated
    /// in the same critical section), and assert that a SECOND
    /// `rotate_if_needed` call after the first commits `last_rotated`
    /// observes the cooldown and skips. Together these prove the
    /// "decision + mutation under the lock" property the dispatcher
    /// race relies on.
    #[test]
    fn rotate_if_needed_holds_lock_across_decision_and_mutation() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("decision-mutation");
        let state_path = dir.join("state.json");

        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state
            .managed
            .interfaces
            .insert("wlan0".into(), InterfaceRecord::default());
        state.save(&state_path).unwrap();

        // First call: must rotate (no cooldown active). The hook
        // checks the in-process slot's `is_held_in_process` to confirm
        // the lock wraps the cooldown decision — we got HERE because
        // the cooldown read passed, so the lock must already be held.
        let lock_observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lock_observed_in = lock_observed.clone();
        let hook_path = state_path.clone();
        let outcome = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(3600),
                &state_path,
                move |iface, _sp| {
                    if crate::state_lock::is_held_in_process() {
                        lock_observed_in.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    let mut s = State::load_or_default(&hook_path).unwrap();
                    let rec = s.managed.interfaces.entry(iface.to_string()).or_default();
                    rec.current_mac = Some("02:00:00:00:00:01".into());
                    rec.last_rotated = Some(crate::commands::now_iso8601());
                    s.save(&hook_path).unwrap();
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
            .unwrap()
        });
        assert!(matches!(outcome, RotateOutcome::Rotated { .. }));
        assert!(
            lock_observed.load(std::sync::atomic::Ordering::SeqCst),
            "the rotate hook ran while the state lock was held — pre-fix the lock was \
             only acquired inside commands::rotate::run, after the cooldown read"
        );

        // Second call: must observe the just-committed `last_rotated`
        // and skip. This is the leg that proves the dispatcher's
        // "near-simultaneous events don't double-rotate" — the lock
        // serialised the calls, and the second one sees fresh state.
        let calls_second = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_second_in = calls_second.clone();
        let outcome2 = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(3600),
                &state_path,
                move |_iface, _sp| {
                    calls_second_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
            .unwrap()
        });
        assert!(matches!(outcome2, RotateOutcome::SkippedCooldown { .. }));
        assert_eq!(
            calls_second.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the second rotate-if-needed must skip on cooldown without \
             invoking the rotate hook a second time"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #250: the pre-fix code returned `Rotated { new_mac:
    /// 00:00:00:00:00:00 }` when `state.json` couldn't be read back
    /// after the rotate. Pin the new contract: the function returns
    /// `Err` whose message names "rotate succeeded but state read-back
    /// failed", so the dispatcher logs something an operator can act
    /// on.
    #[test]
    fn read_back_failure_after_successful_rotate_returns_meaningful_error() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("readback");
        let state_path = dir.join("state.json");

        // Seed an empty state so the cooldown branch and NoFactoryMac
        // branch don't trip — we want to reach the rotate hook.
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.save(&state_path).unwrap();

        // Hook returns SUCCESS but rewrites the state file with garbage
        // so the read-back fails.
        let hook_path = state_path.clone();
        let result = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(60),
                &state_path,
                move |_iface, _sp| {
                    std::fs::write(&hook_path, b"NOT VALID JSON {").unwrap();
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
        });
        // Pre-fix this returned `Ok(Rotated { new_mac: 00:00:00:00:00:00 })`.
        // Post-fix it returns `Err` with the read-back failure message.
        match result {
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("rotate succeeded") && msg.contains("state read-back"),
                    "error must surface the read-back failure, got: {msg}"
                );
            }
            Ok(other) => panic!("expected Err with read-back trail, got Ok({other:?})"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sibling for #250: even if the file reloads cleanly, a missing
    /// `current_mac` (rotate logic bug) must surface as a real error
    /// instead of the all-zero default.
    #[test]
    fn read_back_with_missing_current_mac_after_rotate_errors() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("readback-missing");
        let state_path = dir.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.save(&state_path).unwrap();

        let result = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(60),
                &state_path,
                move |_iface, _sp| {
                    // Pretend rotation succeeded but didn't persist
                    // `current_mac`. The function must error rather
                    // than fabricate the all-zero MAC.
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
        });
        match result {
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("no current_mac"),
                    "error must call out the missing current_mac, got: {msg}"
                );
            }
            Ok(other) => panic!("expected Err for missing current_mac, got Ok({other:?})"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Happy path: after a successful rotate that writes a valid
    /// `current_mac`, the function returns `Rotated { new_mac }` with
    /// the actual rotated value (no all-zero fabrication).
    #[test]
    fn read_back_success_returns_actual_rotated_mac() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("readback-success");
        let state_path = dir.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.save(&state_path).unwrap();

        let hook_path = state_path.clone();
        let outcome = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(60),
                &state_path,
                move |iface, _sp| {
                    let mut s = State::load_or_default(&hook_path).unwrap();
                    let rec = s.managed.interfaces.entry(iface.to_string()).or_default();
                    rec.current_mac = Some("02:11:22:33:44:55".into());
                    rec.last_rotated = Some(crate::commands::now_iso8601());
                    s.save(&hook_path).unwrap();
                    Ok(crate::exit::SUCCESS)
                },
            )
            .await
            .unwrap()
        });
        match outcome {
            RotateOutcome::Rotated { new_mac } => {
                assert_eq!(new_mac.to_string(), "02:11:22:33:44:55");
                assert!(
                    !new_mac.is_all_zero(),
                    "new_mac must reflect the actual rotation, never the all-zero default"
                );
            }
            other => panic!("expected Rotated, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #245 cross-process race: a foreign fd holds the
    /// kernel-level flock while we attempt the rotate path. The
    /// function must return `SkippedCooldown` with a tiny remaining
    /// window AND must NOT call the rotate hook — exactly the
    /// property the NM dispatcher relies on when two `proteus
    /// rotate-if-needed` processes race. Pre-fix the lock was acquired
    /// only inside the inner `rotate::run` call, AFTER the cooldown
    /// read; the rotate path would fire even while a sibling process
    /// held the lock (the inner acquire would then bail busy, but
    /// the cooldown decision was already made on a stale read).
    #[test]
    fn lock_busy_skips_without_invoking_rotate_hook() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("lock-busy");
        let state_path = dir.join("state.json");
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state.save(&state_path).unwrap();

        // Hold the on-disk lock via a foreign fd so `acquire_inner`
        // races against a real LOCK_EX, not the in-process slot.
        let lock_path = dir.join(".lock");
        let foreign = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(foreign.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "test setup: foreign flock");
        // Tighten the retry budget so the test doesn't burn 5s waiting.
        // SAFETY: tests in this module are single-threaded under
        // `test_serial_guard`, so set_var is sound here.
        unsafe {
            std::env::set_var("PROTEUS_LOCK_TIMEOUT_MS", "200");
        }

        let outcome = rt().block_on(async {
            super::rotate_if_needed_inner_with(
                "wlan0",
                Duration::from_secs(3600),
                &state_path,
                |_iface, _sp| panic!("rotate hook must not fire while the lock is busy"),
            )
            .await
            .unwrap()
        });

        unsafe {
            std::env::remove_var("PROTEUS_LOCK_TIMEOUT_MS");
        }
        // Release the foreign lock.
        unsafe {
            libc::flock(foreign.as_raw_fd(), libc::LOCK_UN);
        }
        drop(foreign);

        assert!(matches!(outcome, RotateOutcome::SkippedCooldown { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== N14 (Stream 4) — per-iface debounce for concurrent rotates =====

    /// Multi-threaded runtime helper for the N14 regression test below.
    /// The other rotate tests use `new_current_thread` because they
    /// drive one call at a time; the parallel-task race needs true
    /// multi-worker scheduling to surface.
    fn rt_mt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// N14: two `tokio::spawn`'d `rotate_if_needed` calls against the
    /// SAME iface must serialise such that exactly one Rotated +
    /// exactly one SkippedCooldown comes out. The pre-fix code would
    /// double-rotate: both tasks acquired the state lock (one as the
    /// real outermost holder, the other as a no-op reentrant guard
    /// because `Mutex<Option<File>>` is process-wide reentrant), both
    /// read `last_rotated` before either wrote it, both saw "no
    /// cooldown", both fired the rotate hook.
    ///
    /// The fix wraps the entire decision-and-mutation sequence in a
    /// per-iface async mutex (`iface_rotate_mutex`); the second task
    /// blocks at that mutex until the first releases, by which point
    /// `last_rotated` has been stamped, and the second's cooldown read
    /// trips the skip branch.
    ///
    /// The hook here uses an `AtomicUsize` to count rotates so the
    /// assertion stays robust to scheduling jitter: regardless of
    /// which task wins the race, exactly one rotate must fire.
    #[test]
    fn n14_concurrent_rotate_same_iface_serialises() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("n14-concurrent");
        let state_path = dir.join("state.json");

        // Seed: factory MAC on file, no `last_rotated` so both tasks
        // would pass the cooldown check pre-fix.
        let mut state = State::default();
        state
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:ff".into());
        state
            .managed
            .interfaces
            .insert("wlan0".into(), InterfaceRecord::default());
        state.save(&state_path).unwrap();

        // The hook simulates a real rotate: bumps `last_rotated` and
        // writes a fresh `current_mac`. It also increments a counter
        // so the test can assert exactly-once. A small sleep widens
        // the window during which the second task could observe stale
        // state if the per-iface mutex weren't doing its job.
        let rotate_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state_path_a = state_path.clone();
        let state_path_b = state_path.clone();
        let calls_a = rotate_calls.clone();
        let calls_b = rotate_calls.clone();

        let make_hook = |calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
                         hook_path: std::path::PathBuf| {
            move |iface: &str, _sp: &std::path::Path| -> Result<u8> {
                // Read-modify-write the state stamp so the SECOND
                // task's cooldown check has something to observe.
                let mut s = State::load_or_default(&hook_path).unwrap();
                let rec = s.managed.interfaces.entry(iface.to_string()).or_default();
                rec.current_mac = Some("02:00:00:00:00:01".into());
                rec.last_rotated = Some(crate::commands::now_iso8601());
                s.save(&hook_path).unwrap();
                // Brief sleep to keep the critical section open long
                // enough that a buggy implementation has every chance
                // to lose the race.
                std::thread::sleep(Duration::from_millis(50));
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(crate::exit::SUCCESS)
            }
        };

        let (out_a, out_b) = rt_mt().block_on(async move {
            let hook_a = make_hook(calls_a, state_path_a.clone());
            let hook_b = make_hook(calls_b, state_path_b.clone());
            let task_a = tokio::spawn(async move {
                super::rotate_if_needed_inner_with(
                    "wlan0",
                    Duration::from_secs(3600),
                    &state_path_a,
                    hook_a,
                )
                .await
            });
            let task_b = tokio::spawn(async move {
                super::rotate_if_needed_inner_with(
                    "wlan0",
                    Duration::from_secs(3600),
                    &state_path_b,
                    hook_b,
                )
                .await
            });
            (
                task_a.await.unwrap().unwrap(),
                task_b.await.unwrap().unwrap(),
            )
        });

        // Exactly one of the two outcomes must be Rotated and the
        // other SkippedCooldown — order doesn't matter, only the
        // multiset.
        let rotated = matches!(out_a, RotateOutcome::Rotated { .. }) as usize
            + matches!(out_b, RotateOutcome::Rotated { .. }) as usize;
        let skipped = matches!(out_a, RotateOutcome::SkippedCooldown { .. }) as usize
            + matches!(out_b, RotateOutcome::SkippedCooldown { .. }) as usize;
        assert_eq!(
            rotated, 1,
            "exactly one task must rotate; got a={out_a:?} b={out_b:?}"
        );
        assert_eq!(
            skipped, 1,
            "the other task must skip on cooldown; got a={out_a:?} b={out_b:?}"
        );
        assert_eq!(
            rotate_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the rotate hook must fire exactly once across both tasks"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// N14 corollary: two tasks against DIFFERENT ifaces must NOT
    /// serialise on the per-iface mutex — each iface has its own slot
    /// in the registry, so a rotate on `wlan0` and a concurrent rotate
    /// on `eth0` take different mutex Arcs and never block each other.
    ///
    /// We use SEPARATE state files for the two tasks so the test
    /// isolates the per-iface mutex behaviour from the (separately-
    /// tracked) intra-process state-lock reentrancy concern. With the
    /// per-iface mutex in place, both hooks fire and both calls return
    /// `Rotated`; with a coarse one-global-mutex implementation, the
    /// hooks would serialise and the test wall-clock would observe
    /// it. We don't assert on timing (flaky on CI) — we assert both
    /// hooks fired and both returned `Rotated`.
    #[test]
    fn n14_concurrent_rotate_different_ifaces_proceeds_in_parallel() {
        let _serial = crate::state_lock::test_serial_guard();
        let dir = fresh_state_dir("n14-different-ifaces");
        // Distinct state files so the two tasks don't race on the same
        // state.json (the state-lock's intra-process reentrancy is a
        // separate concern from per-iface debounce; this test pins the
        // latter only).
        let state_path_a = dir.join("a").join("state.json");
        let state_path_b = dir.join("b").join("state.json");
        std::fs::create_dir_all(state_path_a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(state_path_b.parent().unwrap()).unwrap();

        let mut state_a = State::default();
        state_a
            .original_macs
            .insert("wlan0".into(), "aa:bb:cc:dd:ee:00".into());
        state_a
            .managed
            .interfaces
            .insert("wlan0".into(), InterfaceRecord::default());
        state_a.save(&state_path_a).unwrap();

        let mut state_b = State::default();
        state_b
            .original_macs
            .insert("eth0".into(), "aa:bb:cc:dd:ee:01".into());
        state_b
            .managed
            .interfaces
            .insert("eth0".into(), InterfaceRecord::default());
        state_b.save(&state_path_b).unwrap();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_a = calls.clone();
        let calls_b = calls.clone();
        let state_path_a_t = state_path_a.clone();
        let state_path_b_t = state_path_b.clone();

        let make_hook = |counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
                         hook_path: std::path::PathBuf,
                         mac: &'static str| {
            move |iface: &str, _sp: &std::path::Path| -> Result<u8> {
                let mut s = State::load_or_default(&hook_path).unwrap();
                let rec = s.managed.interfaces.entry(iface.to_string()).or_default();
                rec.current_mac = Some(mac.to_string());
                rec.last_rotated = Some(crate::commands::now_iso8601());
                s.save(&hook_path).unwrap();
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(crate::exit::SUCCESS)
            }
        };

        let (out_a, out_b) = rt_mt().block_on(async move {
            let hook_a = make_hook(calls_a, state_path_a_t.clone(), "02:00:00:00:00:0a");
            let hook_b = make_hook(calls_b, state_path_b_t.clone(), "02:00:00:00:00:0b");
            let task_a = tokio::spawn(async move {
                super::rotate_if_needed_inner_with(
                    "wlan0",
                    Duration::from_secs(3600),
                    &state_path_a_t,
                    hook_a,
                )
                .await
            });
            let task_b = tokio::spawn(async move {
                super::rotate_if_needed_inner_with(
                    "eth0",
                    Duration::from_secs(3600),
                    &state_path_b_t,
                    hook_b,
                )
                .await
            });
            (
                task_a.await.unwrap().unwrap(),
                task_b.await.unwrap().unwrap(),
            )
        });

        // Both must rotate — separate ifaces hash to separate mutex
        // slots in the per-iface registry, so neither blocks the other.
        assert!(
            matches!(out_a, RotateOutcome::Rotated { .. }),
            "wlan0 must rotate; got {out_a:?}"
        );
        assert!(
            matches!(out_b, RotateOutcome::Rotated { .. }),
            "eth0 must rotate; got {out_b:?}"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "both rotate hooks must fire when ifaces differ"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
