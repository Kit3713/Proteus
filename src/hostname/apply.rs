// SPDX-License-Identifier: GPL-3.0-or-later

//! Coherent application of the kernel/pretty/transient hostname triple.
//!
//! The three are set as a unit so an observer never sees the kernel hostname
//! (`linksys-7a3f`) disagree with the pretty hostname (`Chris's Laptop`) on
//! the same boot. Originals are captured into `state.json` on first apply and
//! never re-captured, matching the sacred-original-cache invariant.

use anyhow::{Context, Result};
use serde::Serialize;

use super::dbus::{self, HostnameSnapshot};
use crate::state::{HostnameOriginals, State};

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

/// Apply `name` to all three hostname fields and persist the originals into
/// `state` on the first apply. Caller is responsible for `state.save(path)`
/// after this returns.
pub async fn apply_hostname(
    name: &str,
    mode_label: &str,
    state: &mut State,
) -> Result<ApplyOutcome> {
    super::validate_hostname(name)?;

    let proxy = dbus::proxy().await?;
    let before = dbus::read_snapshot(&proxy).await;

    capture_originals(state, &before);

    proxy
        .set_static_hostname(name, false)
        .await
        .context("setting static hostname via hostname1.SetStaticHostname")?;
    proxy
        .set_pretty_hostname(name, false)
        .await
        .context("setting pretty hostname via hostname1.SetPrettyHostname")?;
    proxy
        .set_hostname(name, false)
        .await
        .context("setting transient hostname via hostname1.SetHostname")?;

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
    if let Some(k) = kernel {
        proxy
            .set_static_hostname(k, false)
            .await
            .context("restoring static hostname")?;
    }
    if let Some(p) = pretty {
        proxy
            .set_pretty_hostname(p, false)
            .await
            .context("restoring pretty hostname")?;
    }
    if let Some(t) = transient {
        proxy
            .set_hostname(t, false)
            .await
            .context("restoring transient hostname")?;
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
