Practical patterns for MAC rotation. For the mental model, see `proteus wiki concepts`.

## One-shot rotation

Rotate a single interface:

```sh
sudo proteus rotate --iface wlan0
```

Rotate every managed interface in one shot:

```sh
sudo proteus rotate --yes
```

`--yes` is required when no `--iface` is given because the command touches every managed interface at once. Pinned interfaces are skipped silently; everything else gets a fresh MAC, a fresh DHCPv6 DUID, and (under stable-privacy) a fresh IPv6 IID. See `proteus wiki ipv6` for the IID coupling.

Confirm it took:

```sh
proteus current --json | jq .interfaces[].mac
```

## OUI pool selection

Rotation draws from one of five pools. The OUI (upper three bytes of the MAC) advertises a vendor; picking realistically matters because a passive observer sees the OUI and forms a prior about your hardware.

- `apple` — Apple-registered prefixes
- `intel` — Intel-registered prefixes
- `samsung` — Samsung-registered prefixes
- `dell` — Dell-registered prefixes
- `random` — locally-administered random (the U/L bit set, no real-world OUI)

Default is realism-first: Proteus reads your chipset from `proteus status` and biases the pool toward something plausible. An iwlwifi card gets `intel`-weighted, an rtw89 card gets `samsung`- or `random`-weighted. An Apple OUI on a non-Apple Wi-Fi card looks weird and is its own fingerprint.

Pin a pool in `/etc/proteus/config.toml`:

```toml
[mac]
oui-pool = "intel"     # or "apple", "samsung", "dell", "random", "auto"
```

`auto` is the default and applies the chipset-realism heuristic above.

## Avoiding collisions

Proteus never assigns a MAC matching the current gateway's MAC or anything else in the local ARP table. If the random draw collides, the generator re-rolls until clear. A MAC collision on the local segment would be hilarious and broken.

The check runs against the live ARP table at the moment of rotation, not a cached snapshot. There is no opt-out.

## Pinning

Freeze a MAC so neither the schedule nor the probe-driven trigger touches it.

Per interface:

```sh
sudo proteus pin --iface wlan0
```

Per NetworkManager connection profile:

```sh
sudo proteus pin --connection "Coffee Shop"
```

When both an interface pin and a connection pin would apply (the iface is currently joined to the connection), the connection profile wins. This is deliberate — connection-scoped pinning is more useful for the "MAC-bound DHCP reservation at one specific network" case.

Release a pin:

```sh
sudo proteus unpin --iface wlan0
sudo proteus unpin --connection "Coffee Shop"
```

Pinned targets show up in `proteus status` under `pinned`. They are skipped by both `proteus rotate` (without `--force`) and by the timer-driven rotation path.

## Per-NM-connection MAC

When Proteus rotates Wi-Fi, it sets `wifi.cloned-mac-address` on the NetworkManager connection profile rather than rewriting the device-level MAC directly. Other NM-aware tools (`nmcli`, GNOME Settings, KDE NetworkManager applet) then see consistent state — the cloned MAC is the source of truth for that connection.

The fallback is per-device rtnetlink when the connection isn't NM-managed (rare on Fedora, common on minimal systemd setups). `proteus status` names which path is in use per interface.

This means rotating mid-session on a Wi-Fi network triggers an NM reconnect — same effect as toggling `Use random hardware address` in GNOME, but driven by Proteus and synchronized with the DUID and IID rotations.

## Fresh MAC per visit (captive portals)

Known-portal SSIDs get a fresh MAC every time you join, regardless of the periodic schedule. Combined with the `rotate-before-auth` policy, this means whoever runs the portal cannot correlate today's visit with yesterday's.

Mark an SSID as a known portal in config:

```toml
[captive-portal]
known-ssids = ["Starbucks WiFi", "Boingo Hotspot", "_The Free WiFi"]
```

For SMS-bound portals where the auth ticket is tied to your MAC, switch policy to `preserve-mac` for that SSID — rotating mid-session locks you out of your own session. See `proteus wiki captive-portals` for the full policy matrix and the `_per-ssid-policy` table.

## Schedule

Default is every 2h via `proteus-rotate.timer`. Tunable in `/etc/proteus/config.toml`:

```toml
[rotation]
interval = "2h"        # systemd time spec; "0" disables the periodic timer
jitter   = "10m"       # randomized delay so multiple machines don't sync
```

Set `interval = "0"` to drop the periodic timer entirely and rely only on probe-driven and manual rotation. See `proteus wiki rotation` for the full timer story and how the boot oneshot fits in.

## Probe-driven rotation

A second timer (`proteus-check.timer`, default 5m) runs the probe quorum. Default is at least 3 of 4 endpoints failing → rotate. Single-endpoint flakiness will not trigger anything.

Tune in config:

```toml
[probes]
endpoints = ["1.1.1.1:443", "9.9.9.9:443", "8.8.8.8:443", "208.67.222.222:443"]
quorum    = 3
cooldown  = "60s"
interval  = "5m"
```

Probes target IPs, not hostnames — a broken resolver should not cause a rotation. Probe failures classified as portal-caused are excluded from the quorum so you don't loop behind a captive portal. See `proteus wiki probes` for quorum logic and the portal-classification path.

## Reverting a single MAC

Restore the original cached MAC for one interface:

```sh
sudo proteus revert --iface wlan0
```

This pulls the permanent MAC from `/var/lib/proteus/state.json` (captured the first time Proteus saw the system, never re-captured) and writes it back via the same NM-or-rtnetlink path. The DUID and IID for that interface are rolled back together.

Whole-system revert is `sudo proteus revert --yes` (no other flags).

## DUID coupling

Rotating the MAC also rotates the DHCPv6 DUID for that interface. Otherwise DHCP would hand the same client identity across MAC rotations, defeating the rotation.

DUID rotation is per-interface, not system-wide — better isolation, smaller blast radius if a DHCPv6 server caches aggressively. The DUID is regenerated using the link-layer-plus-time format (RFC 8415 §11.3) seeded with the freshly-assigned MAC.

## IPv6 IID coupling

Under stable-privacy (RFC 7217), the IPv6 IID derives deterministically from the MAC plus a network-scoped key. Rotate the MAC and the IID rotates with no extra action. Temp addresses (RFC 8981) are flushed on the same boundary.

If you've manually disabled stable-privacy in NM, the IID falls back to EUI-64 (which leaks the MAC directly) or to randomized — Proteus surfaces which mode is active in `proteus status`. See `proteus wiki ipv6` for the full address-mode story and the kernel knobs involved.

## Cross-references

- `proteus wiki concepts` — what counts as an identifier, what rotates together, why pinning exists
- `proteus wiki captive-portals` — policy matrix, fresh-MAC-per-visit, the loop-avoidance rule
- `proteus wiki rotation` — timers, jitter, the boot oneshot
- `proteus wiki probes` — quorum logic, cooldown, portal exclusion
- `proteus wiki ipv6` — stable-privacy, temp addresses, IID derivation
