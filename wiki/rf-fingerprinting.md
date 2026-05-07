RF fingerprinting is two problems. The hardware-baked part (analog imperfections of your radio chip) is unfixable in software — Proteus does not pretend otherwise. The OS-controllable part (TX power, probe-request behavior, scan policy, the chip inventory you can read out) is in scope and is a focus area. This page covers both halves so you know exactly where the boundary is.

## What RF fingerprinting is

Every Wi-Fi and Bluetooth radio has analog imperfections that are unique to that physical chip. They come from manufacturing tolerances and don't change over the life of the device:

- **Oscillator drift** — the local oscillator never lands exactly on its nominal frequency, and the offset is stable per device.
- **DAC nonlinearity** — the digital-to-analog converter has small, repeatable errors in how it shapes the output waveform.
- **IQ imbalance** — the in-phase and quadrature components of the modulated signal are slightly mismatched in amplitude and phase.
- **Carrier frequency offset** — the residual offset after the receiver tries to lock on.
- **Transient power spectrum at packet start** — the first few microseconds of a transmission carry a "turn-on" signature that's stable per chip.

A passive receiver that captures the raw signal — software-defined radio (SDR), USRP, sometimes a modified commodity card — can extract these features and produce a physical-layer signature that uniquely identifies the chip emitting the packet. The signature lives below the MAC, below the preamble, below anything the protocol stack can see or modify.

Academic work has demonstrated 95%+ identification accuracy across populations of dozens to hundreds of devices. Real-world deployment is rare because it requires close-range capture, per-device training, and a clean RF environment. But the technique is real, the equipment is no longer exotic, and the cost has been falling for a decade.

What this means in practice: rotating your MAC defeats every adversary who only sees the framed bits. RF fingerprinting defeats your MAC rotation. The two attacks are at different layers and one cannot answer the other.

## What Proteus does

Nothing fixes the analog characteristics. They're physically baked into the silicon. But the radio's *control surface* — what the OS asks the chip to emit, when, and at what power — is software. That part is in scope and ships now via the `proteus rf` subcommand family.

**TX power reduction.** Opt-in via `[rf] tx_power_reduce`. Reduces transmit power so passive listeners need to be physically closer to capture cleanly. The signature does not change; the audience that can read it shrinks. The `proteus rf apply` command issues `iw dev <iface> set txpower fixed <mbm>` against every Wi-Fi interface; `proteus rf revert` restores the cached original. See the dedicated section below.

**Chipset and firmware inventory.** `proteus rf status` is read-only and works as any user. It surfaces, per Wi-Fi interface: driver name (e.g. `iwlwifi`, `rtw89`, `mt7921e`), PCI/USB vendor and device IDs from sysfs, firmware version where the driver exposes it, and the current TX power as reported by `iw dev <iface> info`. It also re-uses the BlueZ adapter inventory so Bluetooth chip vendor, address type, and powered state appear in the same view. Knowing what is in the system lets the operator cross-reference RF-fingerprinting research and size exposure.

**Bluetooth radio policy.** `discoverable=off` by default, BLE Resolvable Private Address (RPA) where the controller supports it, generic device alias. The classic BR/EDR BD_ADDR rotation is chipset-specific HCI territory and stays deferred — too easy to brick across vendors. See `proteus wiki bluetooth`.

**Probe-request privacy is still planned.** Per-scan MAC randomization at the NetworkManager / wpa_supplicant layer, plus suppression of unnecessary active probes when passive scanning is enough, is on the roadmap and is *not* part of the current `[rf]` schema. A laptop searching for known SSIDs broadcasts a list of every network it remembers — that is an L2 leak future releases will address by tightening the supplicant's scan behavior, not the radio. Track this work in `docs/ROADMAP.md`.

What Proteus does *not* try to do, and won't:

- Spoof a different chipset's RF signature. No COTS firmware permits this.
- Mask oscillator drift, DAC nonlinearity, IQ imbalance, or carrier frequency offset. Physical properties of the radio.
- Defeat targeted close-range SDR analysis aimed at *your specific chip*. Only swapping the radio helps.

## What Proteus can NOT do

- Change your chipset's analog characteristics. Impossible without hardware modification.
- Mask oscillator drift, DAC nonlinearity, IQ imbalance, or frequency offset. These are physical properties of the radio.
- Spoof a different chipset's RF signature. Would require custom firmware that doesn't exist for COTS Wi-Fi or Bluetooth chips.
- Defeat targeted close-range SDR analysis. Only swapping the radio helps.

If your threat model includes someone aiming an SDR at you from across the room, Proteus's L2-L4 work is irrelevant to that threat. Read the rest of this page for what actually helps.

## What actually works

**Use a swappable USB Wi-Fi adapter.** A different physical radio emits a different RF fingerprint. Rotate adapters periodically. This is the real answer to RF fingerprinting and the only one that holds up against a determined adversary. Cheap USB dongles are sufficient — you're buying RF identity, not throughput.

**Reduce TX power.** Proteus's opt-in knob narrows the capture radius. Useful, not a cure. See the section below for what it actually buys you.

**Don't use Wi-Fi in adversarial environments.** Ethernet has no RF leak. Cellular hands RF identification off to the carrier — a different problem with different threats, but not the SDR-in-the-room one. Faraday-bagging the laptop is the nuclear option and works.

## TX power reduction

Configurable in `/etc/proteus/config.toml`:

```toml
[rf]
tx_power_reduce = false           # opt-in; default off in min/low/med, on in high/agr
tx_power_reduction_db = 6         # dB below regulatory max
```

The `[rf]` schema has exactly two fields. `tx_power_reduce` is a boolean master switch. `tx_power_reduction_db` is a `u8` count of dB below the regulatory ceiling.

Profile defaults:

| Profile | `tx_power_reduce` |
|---|---|
| `off` | false (every feature off) |
| `min` | false |
| `low` | false |
| `med` | false |
| `high` | **true** |
| `agr` | **true** |

A user-set value (`[rf] tx_power_reduce = false` or `= true` in `/etc/proteus/config.toml`) overrides the profile baseline. Switching from `med` to `high` does not flip an explicit `false` back on; switching from `high` back to `med` does not silently clear an explicit `true`. See `proteus wiki profiles`.

When enabled, Proteus issues `iw dev <iface> set txpower fixed <mbm>` against every Wi-Fi interface at apply time, computing the target as the regulatory maximum (`iw reg get`, with a 20 dBm fallback if the lookup fails) minus `tx_power_reduction_db × 100` mBm.

Default reduction is 6 dB below the regulatory maximum. That's roughly a quarter of the radiated power, halving the effective range of a passive listener under typical free-space assumptions.

Tradeoff is real: reduced range may degrade reception in weak-signal environments. `proteus apply` prints a one-line risk warning whenever the knob is on. If reception suffers, `sudo proteus rf revert --yes` restores the cached pre-Proteus TX power exactly.

## CLI surface

```sh
# Read-only inventory: chip + firmware + current TX + regulatory max + BT adapters.
# Works without root; --json is GUI-friendly.
proteus rf status
proteus rf status --json

# Apply the configured TX-power floor. Requires root + --yes. Idempotent;
# captures the original TX power once on first apply.
sudo proteus rf apply --yes

# Restore the cached pre-Proteus TX power. Requires root + --yes. Idempotent.
sudo proteus rf revert --yes
```

Skip behavior — each path lands as a clear note instead of an error:

- `rf.tx_power_reduce = false` → `apply` exits 0 with `disabled in config (rf.tx_power_reduce = false)`.
- No Wi-Fi interfaces detected → exits 0 with `no Wi-Fi interfaces detected`.
- `iw` binary missing (no iw-tools package) → exits 70 (`SYSTEM_NOT_SUPPORTED`).
- Already at the configured target → re-apply is a no-op write the kernel accepts and the originals remain captured.

The orchestrated `proteus apply` runs `rf` after `stack` and before `nft`, so the same gate applies in the all-in-one command. Failures on one interface never abort the rest of the apply pass.

## Chipset reporting

`proteus rf status` includes the radio inventory in one read-only call:

- **Wi-Fi** — driver name (e.g. `iwlwifi`, `rtw89`, `mt7921e`), chip ID from sysfs (`/sys/class/net/<iface>/device/{vendor,device}`), firmware version where the driver exposes it (`/sys/class/net/<iface>/device/firmware_version` and friends).
- **Bluetooth** — chip vendor (Intel, Broadcom, Realtek, CSR, Qualcomm), address type, powered state.

Take the chipset family and search IEEE Xplore, ACM Digital Library, USENIX, or the academic search engine of your choice for "RF fingerprinting" plus the chip name. If hardware shows up in published research, the operator has a calibrated sense of exposure.

## Threat model

**Casual passive Wi-Fi tracking.** Mall analytics boxes, advertising platforms, roadside sniffers. These do L2 (MAC) tracking — Proteus is highly effective against them. RF fingerprinting at this scale is too expensive; nobody deploys it for ad-tech.

**Targeted surveillance with SDR proximity.** A specific adversary aiming a receiver at a specific person. Here RF fingerprinting becomes a real concern, and Proteus's L2-L4 changes don't help. Hardware swap (different USB adapter, or no Wi-Fi at all) is the answer.

**Bulk RF collection at backbone or carrier scale.** Out of scope for any host-side tool. If this is your threat model you have problems Proteus cannot reason about.

The honest summary: Proteus reduces every fingerprint the OS can control. RF has two halves — the hardware-baked half is a physical leak Proteus cannot touch, and the software-controlled half (TX power, probe behavior, scan policy, chip inventory) is in scope and a focus area. This page exists so you don't accidentally trust the wrong half.

See `proteus wiki threat-model` for the full picture and the line between in-scope and out-of-scope identifiers.

## What an SDR attack looks like

So you can size the threat. A practical RF-fingerprinting attack against a target laptop typically requires:

- An SDR receiver (HackRF, USRP, BladeRF — low thousands of dollars at the high end, low hundreds at the low end).
- Physical proximity. Tens of meters at the upper bound, single meters for clean captures, depending on antenna and environment.
- A reference capture of your specific device, or a database of capture samples for your chipset family, to train the classifier against.
- Time. Real-time identification is harder than offline classification of stored captures.

This adds up to a targeted attack, not mass surveillance. But targeted is exactly the case where it matters most.

## Useful reading

- Brik, Banerjee, Gruteser, Oh — "Wireless Device Identification with Radiometric Signatures." MobiCom 2008. The foundational paper.
- Danev, Capkun — "Physical-layer Identification of UHF RFID Tags." MobiCom 2010.
- Vo-Huu, Vo-Huu, Noubir — "Fingerprinting Wi-Fi Devices Using Software Defined Radios." WiSec 2016.
- Search IEEE Xplore and ACM DL for "RF fingerprinting" plus your chipset family.

## Cross-refs

- `proteus wiki threat-model` — full out-of-scope discussion and what to layer on top of Proteus.
- `proteus wiki concepts` — what Proteus addresses at L2-L4 and what it deliberately does not touch.
- `proteus wiki bluetooth` — separate Bluetooth RF concerns and the BLE Resolvable Private Address story.
