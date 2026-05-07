// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::mac::generator::{self, GenerateOptions};
use crate::mac::{Mac, arp, factory};
use crate::nm::{self, DeviceInfo, DeviceKind};
use crate::state::State;
use crate::version;

#[derive(Debug, Serialize)]
struct RotateReport {
    rotated: Vec<RotatedEntry>,
    skipped: Vec<SkippedEntry>,
}

#[derive(Debug, Serialize)]
struct RotatedEntry {
    iface: String,
    previous: Option<String>,
    new: String,
    connection: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkippedEntry {
    iface: String,
    reason: String,
}

pub fn run(
    iface_filter: Option<&str>,
    yes: bool,
    state_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8> {
    if let Err(code) = super::require_yes(
        yes,
        "'rotate' is mutating (writes new MACs to NetworkManager)",
        "proteus help rotate",
    ) {
        return Ok(code);
    }
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    // Issue #126: serialize concurrent rotates on <state-dir>/.lock.
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };

    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;
    let mut state = State::load_or_default(&state_path)?;

    let arp_macs = arp::read_arp_macs();
    let gateway_mac = arp::read_default_gateway_mac();
    let mut avoid: HashSet<Mac> = arp_macs;
    if let Some(gw) = gateway_mac {
        avoid.insert(gw);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let result: Result<RotateReport> = rt.block_on(async {
        let conn = zbus::Connection::system()
            .await
            .context("connecting to system DBus (NetworkManager required)")?;
        let devices = nm::list_devices(&conn).await?;
        rotate_devices(
            &conn,
            devices,
            iface_filter,
            &config,
            &avoid,
            &mut state,
            &state_path,
        )
        .await
    });

    let report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("proteus: rotate failed: {e:#}");
            return Ok(exit::GENERIC_ERROR);
        }
    };

    persist_capture_metadata(&mut state);
    state.save(&state_path)?;

    if report.rotated.is_empty() && report.skipped.is_empty() {
        eprintln!("proteus: no NetworkManager-managed interfaces matched");
        return Ok(exit::GENERIC_ERROR);
    }

    print_report(&report);
    Ok(exit::SUCCESS)
}

async fn rotate_devices(
    conn: &zbus::Connection,
    devices: Vec<DeviceInfo>,
    iface_filter: Option<&str>,
    config: &Config,
    avoid: &HashSet<Mac>,
    state: &mut State,
    state_path: &Path,
) -> Result<RotateReport> {
    let mut report = RotateReport {
        rotated: Vec::new(),
        skipped: Vec::new(),
    };
    for dev in devices {
        if let Some(f) = iface_filter
            && dev.interface != f
        {
            continue;
        }
        if !matches!(dev.kind, DeviceKind::Wifi | DeviceKind::Ethernet) {
            if iface_filter.is_some() {
                report.skipped.push(SkippedEntry {
                    iface: dev.interface.clone(),
                    reason: format!("device kind {:?} not supported", dev.kind),
                });
            }
            continue;
        }
        if !dev.managed && iface_filter.is_none() {
            // Quietly skip when iterating all devices.
            continue;
        }
        if let Some(rec) = state.managed.interfaces.get(&dev.interface)
            && let Some(pin) = &rec.pinned
        {
            report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("pinned to {pin}"),
            });
            continue;
        }
        match rotate_one(conn, &dev, config, avoid, state, state_path).await {
            Ok(entry) => report.rotated.push(entry),
            Err(e) => report.skipped.push(SkippedEntry {
                iface: dev.interface.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    Ok(report)
}

async fn rotate_one(
    conn: &zbus::Connection,
    dev: &DeviceInfo,
    config: &Config,
    avoid: &HashSet<Mac>,
    state: &mut State,
    state_path: &Path,
) -> Result<RotatedEntry> {
    if dev.connections.is_empty() {
        anyhow::bail!("no NM connection profile available");
    }

    // Capture-then-save-then-mutate: the original factory MAC must be
    // durable on disk BEFORE we ask NetworkManager to set a cloned MAC.
    // Otherwise a crash between the DBus write and the final state.save()
    // at the end of `run` would lose the factory MAC and turn `revert` into
    // a no-op (sacred-originals invariant; issue #119).
    capture_original_mac(state, &dev.interface, dev.hw_address.as_deref());
    persist_capture_metadata(state);
    state.save(state_path)?;

    let forbidden = build_forbidden(state, dev.hw_address.as_deref());
    let opts = GenerateOptions {
        pool: &config.mac.oui_pool,
        forbidden: &forbidden,
        avoid,
    };
    let new_mac = generator::generate(&opts)?;

    // Issue #122: iterate every connection profile bound to the device,
    // not just the first one. Otherwise roaming between SSIDs surfaces
    // the un-cloned factory MAC for the profiles that didn't get touched.
    // The display-id label of the first profile is reported back as the
    // "primary" so the rotated_entry keeps the existing schema. Failures
    // on later profiles are logged but don't fail the whole rotate.
    let mut primary_id: Option<String> = None;
    for connection_path in &dev.connections {
        let id = nm::apply::read_connection_id(conn, connection_path)
            .await
            .ok()
            .flatten();
        let uuid = nm::apply::read_connection_uuid(conn, connection_path)
            .await
            .ok()
            .flatten();
        if let Err(e) = nm::apply::set_cloned_mac(conn, connection_path, dev.kind, new_mac).await {
            tracing::warn!(
                profile = ?id,
                "set_cloned_mac failed for profile: {e:#}"
            );
            continue;
        }
        if primary_id.is_none() {
            primary_id = id.clone();
        }
        if let Some(uuid) = uuid {
            let crec = state.managed.connections.entry(uuid).or_default();
            crec.current_mac = Some(new_mac.to_string());
            crec.last_rotated = Some(super::now_iso8601());
            crec.rotation_count += 1;
        }
    }

    let rec = state
        .managed
        .interfaces
        .entry(dev.interface.clone())
        .or_default();
    let previous = rec.current_mac.clone().or_else(|| dev.hw_address.clone());
    rec.current_mac = Some(new_mac.to_string());
    rec.last_rotated = Some(super::now_iso8601());
    rec.rotation_count += 1;

    Ok(RotatedEntry {
        iface: dev.interface.clone(),
        previous,
        new: new_mac.to_string(),
        connection: primary_id,
    })
}

/// Issue #123 / #208: cache the BURNED-IN factory MAC, never a live (possibly
/// cloned) value.
///
/// The kernel surfaces the current netdev MAC at
/// `/sys/class/net/<iface>/address`, which after even one prior rotation is
/// the cloned value — caching that as "original" makes `proteus revert`
/// restore to a non-original. We consult `factory::permanent_address` which
/// prefers `phy80211/macaddress` (Wi-Fi) then `ethtool -P` (ethernet) and
/// only accepts the live `address` when `addr_assign_type == NET_ADDR_PERM`.
///
/// Issue #208 dropped the previous `hw_hint` fallback that consulted NM's
/// live `HwAddress`. NM surfaces whatever the kernel currently reports — on
/// a driver without phy80211 *and* without `ETHTOOL_GPERMADDR`, that's the
/// live address, which post-rotation is the cloned MAC. Caching it as
/// "factory" silently undid the #123 guard. The new contract: when
/// `factory::permanent_address` returns `None`, we leave `original_macs`
/// untouched and let `proteus status` surface "no factory MAC captured" so
/// the operator can intervene rather than the tool quietly recording a
/// known-cloned value as the restoration target.
fn capture_original_mac(state: &mut State, iface: &str, _hw_hint: Option<&str>) {
    capture_original_mac_under(state, iface, |i| factory::permanent_address(i))
}

/// Test-injectable form of [`capture_original_mac`]. The closure stands in
/// for `factory::permanent_address` so unit tests don't have to read the
/// real `/sys/class/net`. Issue #200.
fn capture_original_mac_under(
    state: &mut State,
    iface: &str,
    permanent: impl Fn(&str) -> Option<String>,
) {
    if state.original_macs.contains_key(iface) {
        return;
    }
    if let Some(mac) = permanent(iface) {
        state.original_macs.insert(iface.to_string(), mac);
    }
}

fn persist_capture_metadata(state: &mut State) {
    if state.captured_by_version.is_none() {
        state.captured_by_version = Some(version::VERSION.to_string());
    }
    if state.captured_at.is_none() {
        state.captured_at = Some(super::now_iso8601());
    }
}

fn build_forbidden(state: &State, hw: Option<&str>) -> HashSet<Mac> {
    let mut set = HashSet::new();
    for mac_str in state.original_macs.values() {
        if let Ok(m) = mac_str.parse::<Mac>() {
            set.insert(m);
        }
    }
    if let Some(h) = hw
        && let Ok(m) = h.parse::<Mac>()
    {
        set.insert(m);
    }
    for rec in state.managed.interfaces.values() {
        if let Some(m) = rec.current_mac.as_ref().and_then(|s| s.parse::<Mac>().ok()) {
            set.insert(m);
        }
    }
    set
}

fn print_report(report: &RotateReport) {
    for r in &report.rotated {
        let prev = r.previous.as_deref().unwrap_or("?");
        match &r.connection {
            Some(id) => println!("rotated {} ({}): {} -> {}", r.iface, id, prev, r.new),
            None => println!("rotated {}: {} -> {}", r.iface, prev, r.new),
        }
    }
    for s in &report.skipped {
        println!("skipped {}: {}", s.iface, s.reason);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Build a stub `permanent_address` lookup so tests don't poke real sysfs.
    /// Issue #200: the previous test read `/sys/class/net/eth0` directly which
    /// flaked on hosts that actually had an `eth0`. The injected closure is
    /// the production-equivalent of `factory::permanent_address_under`.
    fn stub_permanent(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |iface| map.get(iface).map(|s| s.to_string())
    }

    /// Issue #119 — sacred-originals invariant. `rotate_one` now saves
    /// state.json AFTER `capture_original_mac` and BEFORE the
    /// `nm::set_cloned_mac` DBus write. This test pins the round-trip half:
    /// a captured factory MAC must survive a crash between save and the
    /// DBus mutation so revert can restore it.
    #[test]
    fn captured_factory_mac_persists_to_disk() {
        let dir = crate::testing::TempRoot::new("rotate");
        let state_path = dir.path.join("state.json");

        let mut state = State::default();
        let lookup = stub_permanent(HashMap::from([
            ("wlan0", "aa:bb:cc:dd:ee:ff"),
            ("eth0", "11:22:33:44:55:66"),
        ]));
        capture_original_mac_under(&mut state, "wlan0", &lookup);
        capture_original_mac_under(&mut state, "eth0", &lookup);
        persist_capture_metadata(&mut state);

        state.save(&state_path).expect("state.save");
        drop(state);

        let loaded = State::load(&state_path).expect("load").expect("present");
        assert_eq!(
            loaded.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "wlan0 factory MAC must be on disk before any DBus mutation"
        );
        assert_eq!(
            loaded.original_macs.get("eth0").map(String::as_str),
            Some("11:22:33:44:55:66")
        );
        assert!(loaded.captured_at.is_some());
    }

    /// `capture_original_mac` is capture-once: a second call with a
    /// different MAC must not clobber the first capture.
    #[test]
    fn capture_original_mac_is_idempotent() {
        let mut state = State::default();
        let first = stub_permanent(HashMap::from([("wlan0", "aa:bb:cc:dd:ee:ff")]));
        let second = stub_permanent(HashMap::from([("wlan0", "00:00:00:00:00:00")]));
        capture_original_mac_under(&mut state, "wlan0", &first);
        capture_original_mac_under(&mut state, "wlan0", &second);
        assert_eq!(
            state.original_macs.get("wlan0").map(String::as_str),
            Some("aa:bb:cc:dd:ee:ff"),
            "second capture must not overwrite (sacred-originals)"
        );
    }

    /// Issue #208 — when no factory source produces a MAC (no phy80211, no
    /// `ethtool -P`, and the live address fails the `addr_assign_type` guard),
    /// `capture_original_mac` must leave `original_macs` empty rather than
    /// papering over with a known-cloned value. The previous behaviour fell
    /// back to NM's live `HwAddress`, silently undoing the #123 guard on
    /// drivers without phy80211 / `ETHTOOL_GPERMADDR`.
    #[test]
    fn capture_skips_when_factory_lookup_yields_none() {
        let mut state = State::default();
        let empty = stub_permanent(HashMap::new());
        capture_original_mac_under(&mut state, "eth0", &empty);
        assert!(
            state.original_macs.get("eth0").is_none(),
            "no factory source — must not cache the live (cloned) address"
        );
    }
}
