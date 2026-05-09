// SPDX-License-Identifier: GPL-3.0-or-later

//! Coherent application of the kernel/pretty/transient hostname triple.
//!
//! The three are set as a unit so an observer never sees the kernel hostname
//! (`linksys-7a3f`) disagree with the pretty hostname (`Chris's Laptop`) on
//! the same boot. Originals are captured into `state.json` on first apply and
//! never re-captured, matching the sacred-original-cache invariant.
//!
//! NEV2.3: every DBus call to `org.freedesktop.hostname1` is wrapped in
//! a 5-second `tokio::time::timeout`. A stalled `systemd-hostnamed`
//! used to pin the NM dispatcher synchronously — the dispatcher is the
//! event-driven hot path, so any single hung DBus call there starved
//! the whole rotate machinery. With the timeout, a wedged hostnamed
//! surfaces as `TimedOut` (a recoverable error) and the orchestrator
//! moves on to the next component.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use super::dbus::{self, HostnameSnapshot};
use crate::state::{HostnameOriginals, State};

/// NEV2.3: per-call DBus timeout for hostnamed. The dispatcher is the
/// hot path that benefits most; 5s is generous enough that any healthy
/// hostnamed responds well under it, and short enough that a hang
/// doesn't pin a rotate cycle past its 10s timer budget.
const HOSTNAMED_DBUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Wrap a hostnamed DBus call in `tokio::time::timeout`. Surfaces
/// `TimedOut` as a structured error rather than letting the call block
/// indefinitely. The `op` label is used in the error message so the
/// operator can tell which of the three setters wedged.
async fn with_hostnamed_timeout<T, F>(op: &str, fut: F) -> Result<T>
where
    F: std::future::Future<Output = zbus::Result<T>>,
{
    match tokio::time::timeout(HOSTNAMED_DBUS_TIMEOUT, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(anyhow!(e)).with_context(|| format!("hostnamed: {op}")),
        Err(_elapsed) => Err(anyhow!(
            "hostnamed: {op} timed out after {}s — systemd-hostnamed may be wedged",
            HOSTNAMED_DBUS_TIMEOUT.as_secs()
        )),
    }
}

/// Outcome of a single `apply` run, suitable for human and JSON rendering.
/// `previous`/`current` reuse `HostnameOriginals` because the on-disk shape
/// and the on-bus shape are deliberately the same (kernel/pretty/transient).
#[derive(Debug, Serialize)]
pub struct ApplyOutcome {
    pub mode: String,
    pub previous: HostnameOriginals,
    pub current: HostnameOriginals,
}

fn snapshot_to_triple(s: HostnameSnapshot) -> HostnameOriginals {
    HostnameOriginals {
        kernel: s.static_name,
        pretty: s.pretty_name,
        transient: s.transient_name,
    }
}

/// Capture the live hostname triple into `state.originals.hostname` if not
/// already cached, and return the live snapshot. Splitting capture from
/// mutation lets the caller persist the originals to disk via
/// `state.save()` BEFORE any DBus write — so a crash between capture and
/// mutation can never strand the system without an on-disk record of what
/// to revert to (sacred-originals invariant; see issue #119).
pub async fn capture_originals_step(state: &mut State) -> Result<HostnameSnapshot> {
    let proxy = dbus::proxy().await?;
    let before = dbus::read_snapshot(&proxy).await;
    capture_originals(state, &before);
    Ok(before)
}

/// Apply `name` to all three hostname fields. Originals must already have
/// been captured + persisted via `capture_originals_step` followed by
/// `state.save()`. `before` is the pre-mutation snapshot returned by
/// `capture_originals_step` so we don't need a second DBus round-trip to
/// fill `ApplyOutcome.previous`.
pub async fn mutate_hostname(
    name: &str,
    mode_label: &str,
    before: HostnameSnapshot,
) -> Result<ApplyOutcome> {
    super::validate_hostname(name)?;

    let proxy = dbus::proxy().await?;

    // NEV2.3: each DBus setter is wrapped in a 5s timeout so a wedged
    // hostnamed surfaces as TimedOut rather than blocking the dispatcher.
    with_hostnamed_timeout("SetStaticHostname", proxy.set_static_hostname(name, false)).await?;
    with_hostnamed_timeout("SetPrettyHostname", proxy.set_pretty_hostname(name, false)).await?;
    with_hostnamed_timeout("SetHostname", proxy.set_hostname(name, false)).await?;

    let after = dbus::read_snapshot(&proxy).await;

    Ok(ApplyOutcome {
        mode: mode_label.to_string(),
        previous: snapshot_to_triple(before),
        current: snapshot_to_triple(after),
    })
}

/// Restore the cached originals via the same DBus interface. Only the names
/// that were actually captured at apply time get pushed back; un-captured
/// fields (where the apply-time snapshot had `None`) are left as-is so we
/// don't collapse a name the user later set out-of-band to `""`.
pub async fn revert_hostname(state: &State) -> Result<RevertOutcome> {
    let proxy = dbus::proxy().await?;
    let before = dbus::read_snapshot(&proxy).await;

    let cached = state.originals.hostname.is_some();
    let (kernel, pretty, transient) = revert_targets(state.originals.hostname.as_ref());
    // NEV2.3: same 5s timeout as the apply path so revert can't
    // deadlock against a wedged hostnamed.
    if let Some(k) = kernel {
        with_hostnamed_timeout(
            "SetStaticHostname (revert)",
            proxy.set_static_hostname(k, false),
        )
        .await?;
    }
    if let Some(p) = pretty {
        with_hostnamed_timeout(
            "SetPrettyHostname (revert)",
            proxy.set_pretty_hostname(p, false),
        )
        .await?;
    }
    if let Some(t) = transient {
        with_hostnamed_timeout("SetHostname (revert)", proxy.set_hostname(t, false)).await?;
    }

    let after = dbus::read_snapshot(&proxy).await;

    Ok(RevertOutcome {
        restored: cached,
        previous: snapshot_to_triple(before),
        current: snapshot_to_triple(after),
    })
}

/// Pure helper for `revert_hostname`: returns the (kernel, pretty,
/// transient) tuple of *Some* values that should actually be pushed back to
/// hostnamed. Anything that wasn't captured stays `None` so the caller
/// skips it. Split out so the partial-capture behavior is unit-testable
/// without a DBus connection.
pub(crate) fn revert_targets(
    cached: Option<&HostnameOriginals>,
) -> (Option<&str>, Option<&str>, Option<&str>) {
    match cached {
        Some(orig) => (
            orig.kernel.as_deref(),
            orig.pretty.as_deref(),
            orig.transient.as_deref(),
        ),
        None => (None, None, None),
    }
}

#[derive(Debug, Serialize)]
pub struct RevertOutcome {
    pub restored: bool,
    pub previous: HostnameOriginals,
    pub current: HostnameOriginals,
}

fn capture_originals(state: &mut State, snap: &HostnameSnapshot) {
    if state.originals.hostname.is_some() {
        return;
    }
    state.originals.hostname = Some(HostnameOriginals {
        kernel: snap.static_name.clone(),
        pretty: snap.pretty_name.clone(),
        transient: snap.transient_name.clone(),
    });
    // Keep the legacy single-field cache aligned with the static value so
    // older `proteus original` formatting still works.
    if state.original_hostname.is_none() {
        state.original_hostname.clone_from(&snap.static_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_targets_skips_uncaptured_fields() {
        // Every field set: revert pushes all three.
        let full = HostnameOriginals {
            kernel: Some("k".into()),
            pretty: Some("p".into()),
            transient: Some("t".into()),
        };
        assert_eq!(
            revert_targets(Some(&full)),
            (Some("k"), Some("p"), Some("t"))
        );

        // Partial capture: only kernel was set at apply-time. The other two
        // fields must stay `None` so the caller skips them rather than
        // collapsing them to "" (issues #139/#144).
        let partial = HostnameOriginals {
            kernel: Some("k".into()),
            pretty: None,
            transient: None,
        };
        assert_eq!(revert_targets(Some(&partial)), (Some("k"), None, None));

        // No cache at all: revert is a no-op.
        assert_eq!(revert_targets(None), (None, None, None));
    }

    #[test]
    fn capture_records_first_snapshot_only() {
        let mut state = State::default();
        let snap1 = HostnameSnapshot {
            static_name: Some("first".into()),
            pretty_name: None,
            transient_name: Some("first-transient".into()),
        };
        capture_originals(&mut state, &snap1);

        let snap2 = HostnameSnapshot {
            static_name: Some("second".into()),
            pretty_name: Some("Second".into()),
            transient_name: None,
        };
        capture_originals(&mut state, &snap2);

        let stored = state.originals.hostname.expect("hostname originals saved");
        assert_eq!(stored.kernel.as_deref(), Some("first"));
        assert_eq!(stored.pretty, None);
        assert_eq!(stored.transient.as_deref(), Some("first-transient"));
        assert_eq!(state.original_hostname.as_deref(), Some("first"));
    }
}
