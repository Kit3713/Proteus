# Profiles

The `profile` field at the top of `/etc/proteus/config.toml` selects a coherent baseline of feature toggles tuned to a deployment scenario. Six profiles ship: `off`, `min`, `low`, `med`, `high`, `agr`. The default is `med`. Profiles cover the boolean toggles only; numeric tunables and string fields keep their built-in defaults across every profile.

## Quick reference

| Profile | Use case | Rotation | Discovery silenced | Breaking knobs |
|---|---|---|---|---|
| `off` | hard kill switch | none | none | none — every feature off, per-knob overrides ignored |
| `min` | trusted home LAN | none | none | none |
| `low` | privacy-curious home | scheduled | none | none |
| `med` | public Wi-Fi default | scheduled | mDNS, LLMNR | none |
| `high` | hostile network | scheduled | mDNS, LLMNR | TX power reduction |
| `agr` | conference / border | scheduled, fresh-MAC-per-portal | mDNS, LLMNR, SSDP, WSD | TX power reduction, anonymous outer 802.1X, gratuitous ARP suppression, per-visit MAC rotation |

## Timer cadences per profile

Each profile sets a baseline cadence for `proteus-rotate.timer` (scheduled MAC rotation) and `proteus-check.timer` (probe-driven rotation check). Switching profile via `sudo proteus config set-profile <name> --yes` then `sudo proteus apply --yes` reconciles the on-disk drop-ins under `/etc/systemd/system/proteus-*.timer.d/` so the new cadence takes effect on the next timer cycle.

| Profile | `timers.rotate.interval` | `timers.check.interval` |
|---|---|---|
| `off` | `never` | `never` |
| `min` | `never` | `never` |
| `low` | `4h` | `5m` |
| `med` | `2h` | `5m` |
| `high` | `30m` | `2m` |
| `agr` | `15m` | `1m` |

The sentinel value `never` means "do not run this timer." When a profile resolves to `interval = "never"` for a timer, `proteus apply` removes the corresponding drop-in; the unit-file default (the cadence baked into the unit file under `/usr/lib/systemd/system/`) takes over only if the unit itself is enabled. `Off` and `Min` both resolve to `never` because neither profile schedules rotation in a trusted environment.

User overrides win on a per-timer basis. `[timers.rotate].interval = "1h"` survives a switch from `med` to `high`; the `check` timer still follows the new profile's baseline. `Off` short-circuits this rule: while `off` is active the timer overrides on disk are ignored and both timers resolve to `never`.

## How resolution works

Configuration loads in two passes. The first pass parses the file into the `RawConfig` shape where every field is `Option<T>`. The second pass overlays the user's per-knob overrides on top of the active profile's baseline.

- Per-knob overrides take precedence. A user-set `mac.enabled = false` survives any profile change to `low`, `med`, `high`, or `agr`.
- The `off` profile short-circuits resolution. While `off` is active every feature is forced disabled and per-knob overrides are ignored. The overrides remain on disk untouched, so switching back to a non-`off` profile restores them.
- Numeric and string overrides (for example `mac.rotation_interval = "30m"`) are also preserved across profile changes.

The override-only-if-present model means the difference between "this knob takes the profile baseline" and "this knob is explicitly set to that value" is preserved across reads and writes. `proteus config show` annotates each value with its origin so the operator can tell at a glance which knobs are profile defaults and which were explicitly overridden.

## Choosing a profile

`min` — the system is on a network it owns or trusts. Rotation does not help when the local environment already knows the device. Discovery silencing breaks home printers and casts. Bluetooth alias rotation is unnecessary on a trusted network.

`low` — privacy-curious user on a trusted home network. Scheduled rotation, hostname rotation, IPv6 stable-privacy, DHCP option suppression, and the non-breaking subset of stack hardening. Discovery stays unsilenced because home printers and Chromecasts still need it.

`med` — recommended public Wi-Fi default. Adds mDNS and LLMNR silencing on top of `low`. The system stops broadcasting its hostname and capabilities to anyone on the LAN, but does not enable any knob that could break a service the user cares about.

`high` — the network is actively hostile. Same as `med` plus TX power reduction (`[rf] tx_power_reduce = true`) so the passive-capture radius for an SDR-equipped observer shrinks. SSDP and WSD remain unblocked so KDE Connect and Windows printer discovery still work for the user's own devices. The reduction may degrade range from APs — `proteus apply` surfaces a one-line risk warning, and `sudo proteus rf revert --yes` restores the original TX power exactly.

`agr` — conference, border, or any environment where every breaking knob is acceptable. Carries the `high` TX power reduction forward and adds SSDP and WSD blocks (breaks KDE Connect, Windows printer discovery), anonymous outer identity for 802.1X (some auth servers reject mismatched outer/inner identities), gratuitous ARP suppression (breaks VRRP/keepalived failover detection), and per-visit MAC rotation for known captive portals. Each enabled breaking knob is surfaced by the `proteus apply` risk-warning banner.

`off` — temporary panic disable. Every feature off until the profile is changed back. Useful for debugging, comparing system behavior with and without Proteus, or rapidly disabling everything during an incident. Overrides remain on disk and resume effect when the profile changes back.

## Changing profile

Pick a profile from the CLI:

```
sudo proteus config set-profile high --yes
```

The command rewrites the `profile = "..."` line at the top of `/etc/proteus/config.toml`. Per-knob overrides already in the file are not touched. After changing profile, `sudo proteus apply --yes` re-runs the orchestrator with the new baseline.

To override a single knob without leaving the profile, use the existing setter:

```
sudo proteus config set discovery.ssdp_block true --yes
```

That writes `[discovery] ssdp_block = true` to the file. From now on, regardless of profile, SSDP is blocked. Switching profile to `med` will not turn it back off; the override wins.

To clear all per-knob overrides while keeping the profile, run:

```
sudo proteus config reset --yes
```

That removes every per-knob field from the file, leaving only `profile = "..."`. The next apply uses the pristine profile baseline.

## Auto-detect: missing hardware

Proteus does not write hardware-detection results to the config. Each module checks its own runtime prereqs at apply time and surfaces missing dependencies as skip notes:

```
$ sudo proteus apply --yes
apply summary:
  mac        applied   (ok)
  hostname   applied   (ok)
  bluetooth  skipped   (no BlueZ adapters detected)
  ipv6       applied   (ok)
  dhcp       applied   (ok)
  dns        applied   (ok)
  stack      applied   (ok)
  rf         skipped   (disabled in config (rf.tx_power_reduce = false))
  nft        skipped   (nftables not installed)
totals: applied=6 skipped=3 failed=0
```

`proteus doctor` lists every absent dependency as an informational warning so the operator knows in advance which features will skip. There is no separate scan command and no extra state on disk: the file is always the user's intent, and the runtime behavior reflects what the system can actually do.

## See also

- `proteus wiki config` — every key, its type, and where to set it
- `proteus wiki concepts` — the mental model behind apply, revert, and originals
- `proteus wiki rf-fingerprinting` — what the RF surface knobs do (and what they cannot do)
- `proteus wiki threat-model` — the boundary of in-scope and out-of-scope identifiers
