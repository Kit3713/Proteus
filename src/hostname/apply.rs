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

/// Restore the cached originals via the same DBus interface. Empty-string
/// originals (which represent "never had one set") map to `""`, which
/// hostnamed treats as "unset" — the right thing.
pub async fn revert_hostname(state: &State) -> Result<RevertOutcome> {
    let proxy = dbus::proxy().await?;
    let before = dbus::read_snapshot(&proxy).await;

    let cached = state.originals.hostname.is_some();
    if let Some(orig) = &state.originals.hostname {
        let kernel = orig.kernel.as_deref().unwrap_or_default();
        let pretty = orig.pretty.as_deref().unwrap_or_default();
        let transient = orig.transient.as_deref().unwrap_or_default();

        proxy
            .set_static_hostname(kernel, false)
            .await
            .context("restoring static hostname")?;
        proxy
            .set_pretty_hostname(pretty, false)
            .await
            .context("restoring pretty hostname")?;
        proxy
            .set_hostname(transient, false)
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
