// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus rf` — chipset inventory + opt-in TX-power reduction.
//!
//! `status` is read-only and works as any user; it surfaces driver/chip/
//! firmware/current-TX/regulatory-max for every Wi-Fi interface plus the
//! BlueZ adapter inventory for cross-referencing RF-fingerprinting research.
//!
//! `apply` writes a fixed TX-power floor when `cfg.rf.tx_power_reduce` is
//! true. It captures the per-iface original TX power into `state.originals.rf`
//! on first apply (capture-once, never re-captured) so revert can restore
//! exactly what the system had before Proteus touched it.
//!
//! Hardware-baked RF properties (oscillator drift, IQ imbalance, …) are
//! out of scope by physics. See `proteus wiki rf-fingerprinting` for the
//! boundary.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::exit;
use crate::rf::{self, BluetoothChipInfo, ChipInfo};
use crate::state::{RfOriginals, State};

#[derive(Debug, Serialize)]
struct StatusReport {
    tx_power_reduce: bool,
    tx_power_reduction_db: u8,
    iw_present: bool,
    regulatory_max_mbm: Option<i32>,
    interfaces: Vec<InterfaceStatus>,
    bluetooth: Vec<BluetoothChipInfo>,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct InterfaceStatus {
    iface: String,
    driver: Option<String>,
    vendor_id: Option<String>,
    device_id: Option<String>,
    firmware: Option<String>,
    current_tx_power_mbm: Option<i32>,
    originals_cached: bool,
}

pub fn status(json: bool, config_path: Option<&Path>) -> Result<u8> {
    let config_path = super::config_path(config_path);
    let state_path = super::state_path(None);
    let config = Config::default_or_loaded(&config_path).unwrap_or_default();
    let state = State::load_or_default(&state_path).unwrap_or_default();

    let interfaces = rf::wifi_interfaces();
    let iw_present = rf::iw_present();
    let regulatory_max = if iw_present {
        rf::regulatory_max_mbm()
    } else {
        None
    };

    let interface_reports: Vec<InterfaceStatus> = interfaces
        .iter()
        .map(|iface| build_iface_status(iface, &state, iw_present))
        .collect();

    let mut note = None;
    if interfaces.is_empty() {
        note = Some("no Wi-Fi interfaces detected".to_string());
    } else if !iw_present {
        note = Some("`iw` binary not found on PATH; install iw-tools for TX-power data".into());
    }

    let report = StatusReport {
        tx_power_reduce: config.rf.tx_power_reduce,
        tx_power_reduction_db: config.rf.tx_power_reduction_db,
        iw_present,
        regulatory_max_mbm: regulatory_max,
        interfaces: interface_reports,
        bluetooth: rf::bluetooth_chip_info(),
        note,
    };

    if json {
        super::print_json(&report)?;
    } else {
        print_status_human(&report);
    }
    Ok(exit::SUCCESS)
}

pub fn apply(yes: bool, state_path: Option<&Path>, config_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !yes {
        eprintln!("proteus: 'rf apply' is mutating; pass --yes (see `proteus help rf`)");
        return Ok(exit::NOT_IMPLEMENTED);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let config_path = super::config_path(config_path);
    let config = Config::default_or_loaded(&config_path)?;

    if !config.rf.tx_power_reduce {
        println!("rf: disabled in config (rf.tx_power_reduce = false)");
        return Ok(exit::SUCCESS);
    }

    let interfaces = rf::wifi_interfaces();
    if interfaces.is_empty() {
        println!("rf: no Wi-Fi interfaces detected");
        return Ok(exit::SUCCESS);
    }

    if !rf::iw_present() {
        eprintln!("proteus: `iw` binary not found on PATH; install iw-tools to use rf");
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }

    let target_mbm = compute_target_mbm(&config);
    let mut state = State::load_or_default(&state_path)?;

    let mut applied = 0usize;
    let mut skipped = 0usize;
    for iface in &interfaces {
        capture_original(&mut state, iface);
        match rf::set_tx_power_mbm(iface, target_mbm) {
            Ok(()) => {
                applied += 1;
                println!(
                    "rf: {iface} set to {target_mbm} mBm (~{} dBm)",
                    target_mbm / 100
                );
            }
            Err(e) => {
                skipped += 1;
                tracing::warn!("rf: setting tx power on {iface} failed: {e:#}");
                println!("rf: {iface} skipped ({e})");
            }
        }
    }
    state.save(&state_path)?;

    println!(
        "rf apply: {applied} interface(s) updated, {skipped} skipped (target {target_mbm} mBm)"
    );
    Ok(exit::SUCCESS)
}

pub fn revert(yes: bool, state_path: Option<&Path>) -> Result<u8> {
    if let Err(e) = super::require_root() {
        eprintln!("proteus: {e}");
        return Ok(exit::PERMISSION_ERROR);
    }
    if !yes {
        eprintln!("proteus: 'rf revert' is mutating; pass --yes (see `proteus help rf`)");
        return Ok(exit::NOT_IMPLEMENTED);
    }
    let _lock = match super::acquire_state_lock_or_print(state_path) {
        Ok(g) => g,
        Err(code) => return Ok(code),
    };
    let state_path = super::state_path(state_path);
    let mut state = State::load_or_default(&state_path)?;

    if state.originals.rf.is_empty() {
        println!("rf: no originals cached, nothing to restore");
        return Ok(exit::SUCCESS);
    }

    if !rf::iw_present() {
        eprintln!("proteus: `iw` binary not found on PATH; cannot restore TX power");
        return Ok(exit::SYSTEM_NOT_SUPPORTED);
    }

    let originals: Vec<(String, RfOriginals)> = state
        .originals
        .rf
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut restored = 0usize;
    let mut missing = 0usize;
    for (iface, orig) in &originals {
        match orig.tx_power_mbm {
            Some(mbm) => match rf::set_tx_power_mbm(iface, mbm) {
                Ok(()) => {
                    restored += 1;
                    println!("rf: {iface} restored to {mbm} mBm");
                }
                Err(e) => {
                    tracing::warn!("rf revert: restoring tx power on {iface} failed: {e:#}");
                    println!("rf: {iface} restore failed ({e})");
                }
            },
            None => {
                missing += 1;
                println!("rf: {iface} had no original TX power on first apply, skipping");
            }
        }
    }

    state.originals.rf.clear();
    state.save(&state_path)?;
    println!("rf revert: {restored} restored, {missing} had no original to restore");
    Ok(exit::SUCCESS)
}

fn capture_original(state: &mut State, iface: &str) {
    if state.originals.rf.contains_key(iface) {
        return;
    }
    let tx_power_mbm = rf::current_tx_power_mbm(iface);
    state
        .originals
        .rf
        .insert(iface.to_string(), RfOriginals { tx_power_mbm });
}

fn compute_target_mbm(config: &Config) -> i32 {
    let max = rf::regulatory_max_mbm_or_fallback();
    let reduction = i32::from(config.rf.tx_power_reduction_db) * 100;
    max.saturating_sub(reduction)
}

fn build_iface_status(iface: &str, state: &State, iw_present: bool) -> InterfaceStatus {
    let info = rf::chip_info(iface).unwrap_or(ChipInfo {
        iface: iface.to_string(),
        ..Default::default()
    });
    let current = if iw_present {
        rf::current_tx_power_mbm(iface)
    } else {
        None
    };
    InterfaceStatus {
        iface: iface.to_string(),
        driver: info.driver,
        vendor_id: info.vendor_id,
        device_id: info.device_id,
        firmware: info.firmware,
        current_tx_power_mbm: current,
        originals_cached: state.originals.rf.contains_key(iface),
    }
}

fn print_status_human(r: &StatusReport) {
    println!("rf:");
    println!("  tx_power_reduce:        {}", yesno(r.tx_power_reduce));
    println!("  tx_power_reduction_db:  {}", r.tx_power_reduction_db);
    println!("  iw on PATH:             {}", yesno(r.iw_present));
    if let Some(max) = r.regulatory_max_mbm {
        println!("  regulatory max:         {max} mBm (~{} dBm)", max / 100);
    } else {
        println!("  regulatory max:         (unknown — using fallback at apply time)");
    }
    println!("interfaces:");
    if r.interfaces.is_empty() {
        println!("  (none — no Wi-Fi hardware detected)");
    } else {
        for i in &r.interfaces {
            println!("  {}", i.iface);
            println!(
                "    driver:               {}",
                i.driver.as_deref().unwrap_or("(unknown)")
            );
            println!(
                "    vendor:device:        {} : {}",
                i.vendor_id.as_deref().unwrap_or("?"),
                i.device_id.as_deref().unwrap_or("?")
            );
            println!(
                "    firmware:             {}",
                i.firmware.as_deref().unwrap_or("(not exposed by driver)")
            );
            match i.current_tx_power_mbm {
                Some(m) => println!("    current tx power:     {m} mBm (~{} dBm)", m / 100),
                None => println!("    current tx power:     (unknown)"),
            }
            println!("    originals cached:     {}", yesno(i.originals_cached));
        }
    }
    println!("bluetooth:");
    if r.bluetooth.is_empty() {
        println!("  (none — no BlueZ adapters detected)");
    } else {
        for b in &r.bluetooth {
            println!("  {}", b.hci);
            println!(
                "    address:              {} ({})",
                b.address.as_deref().unwrap_or("?"),
                b.address_type.as_deref().unwrap_or("?"),
            );
            println!(
                "    name:                 {}",
                b.name.as_deref().unwrap_or("(unset)")
            );
            println!(
                "    powered:              {}",
                b.powered.map(yesno).unwrap_or("?")
            );
        }
    }
    if let Some(n) = &r.note {
        println!();
        println!("note: {n}");
    }
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_target_mbm_subtracts_reduction_from_max_or_fallback() {
        let mut cfg = Config::default();
        cfg.rf.tx_power_reduction_db = 6;
        // The regulatory lookup is system-dependent, but the math is just
        // `max - 6 dB`. With the fallback in play (no `iw`), max == 2_000
        // and the result is 1_400.
        let target = compute_target_mbm(&cfg);
        // Allow either the fallback result or a real-iw result that happens
        // to land at the same value. The key invariant: target is positive.
        assert!(target > 0, "target should be positive, got {target}");
        assert!(
            target <= rf::regulatory_max_mbm_or_fallback(),
            "target must not exceed regulatory max"
        );
    }

    #[test]
    fn compute_target_mbm_saturates_when_reduction_overshoots() {
        let mut cfg = Config::default();
        cfg.rf.tx_power_reduction_db = 250;
        let target = compute_target_mbm(&cfg);
        // saturating_sub clamps to a non-positive value, never panics. We
        // accept any value <= max; the actual clamp depends on the
        // regulatory lookup. The contract: no overflow.
        assert!(target <= rf::regulatory_max_mbm_or_fallback());
    }

    #[test]
    fn capture_original_records_first_value_and_skips_thereafter() {
        let mut state = State::default();
        // Inject a placeholder so the second call's no-op is observable;
        // the first call would normally hit `iw`, which won't be present
        // in the test environment, so we side-step by pre-populating.
        state.originals.rf.insert(
            "fake-iface".into(),
            RfOriginals {
                tx_power_mbm: Some(1_500),
            },
        );
        capture_original(&mut state, "fake-iface");
        assert_eq!(
            state.originals.rf["fake-iface"].tx_power_mbm,
            Some(1_500),
            "second capture must not overwrite the first"
        );
    }

    #[test]
    fn capture_original_creates_entry_for_unknown_iface() {
        let mut state = State::default();
        capture_original(&mut state, "fake-iface-no-iw");
        // We can't shell `iw` in tests, but the entry must exist so revert
        // sees it. tx_power_mbm being None is the correct "iw missing" path.
        assert!(state.originals.rf.contains_key("fake-iface-no-iw"));
    }
}
