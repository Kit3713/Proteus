First time with Proteus. Walks from "binary installed" to "I know what this thing does and how I'll use it." For the commands reference, see `proteus wiki quickstart`. For the mental model, see `proteus wiki concepts`.

## Five-minute introduction

Your laptop screams "I am Chris, I have been here before" to every coffee shop Wi-Fi. The MAC address, the hostname in the DHCP request, the `_workstation._tcp` mDNS announcement, the IPv6 address derived from the MAC — all of it. Proteus shuts it up at the network layer. Rotates MACs on a schedule and on connectivity loss. Suppresses the chatty discovery protocols. Handles captive portals so you don't loop behind one.

By the end of this page you will have:

- a working install verified with `proteus doctor`
- one rotation under your belt, with the new MAC visible in `proteus current`
- a rotation cadence configured for your daily routine
- a mental model of when Proteus acts and when you do nothing

Mutating commands need root. Read commands work for any user.

## Verify the install

```sh
proteus --version
proteus doctor
```

`doctor` runs a battery of read-only checks and prints `ok / warn / fail / skip` per check. Read it top to bottom. Sample output:

```
proteus doctor — system health check (v0.1.0)

System
  ✓ Linux 6.x.y
  ✓ systemd running
  - running as uid 1000 — some checks need root for full detail

Daemons
  ✓ NetworkManager running
  - BlueZ not running — Bluetooth features will skip
  ✓ systemd-resolved running

Summary: 9 ok, 1 warn, 0 fail, 9 skip
```

What each marker means:

- `ok` (`✓`) — the check passed. Nothing to do.
- `warn` (`⚠`) — suspicious but not broken. Usually means a feature will skip cleanly. The detect-and-defer checks for DNS and NTP show up here when you have `dnscrypt-proxy`, `chrony`, or similar installed — that is by design. Your tool wins.
- `skip` (`-`) — the check could not run. Common reason: not root. Re-run with `sudo proteus doctor` for full detail. Also fires when an optional dependency is absent (BlueZ for Bluetooth, for instance).
- `fail` (`✗`) — hard breakage. Proteus cannot do its job until you fix it. Each fail line carries a remediation pointer like `see: systemctl start NetworkManager`.

Only `fail` causes a non-zero exit. Warns and skips do not. If any check fails, see `proteus wiki troubleshooting` for symptom-based recovery. For the per-check reference, see `proteus wiki doctor`.

## See what your laptop currently exposes

Look before you change anything.

```sh
proteus current
proteus current --json | jq .
proteus original
```

`current` lists the live identifiers your machine is handing out right now — MAC per interface, hostname, Bluetooth alias once those phases land. `original` lists what Proteus snapshotted the first time it saw your system.

The first time you run this, `current` and `original` should match. Nothing has been rotated yet. The only difference will appear after the first `proteus rotate`.

## First rotation

```sh
sudo proteus rotate --iface wlan0
proteus current
```

Watch the MAC change. The connection drops briefly while NetworkManager reconnects with the new MAC. DHCP renews. The gateway sees a fresh device join. Expect a few seconds of dropped traffic.

To rotate every managed interface in one shot:

```sh
sudo proteus rotate --yes
```

`--yes` is required when no `--iface` is given because the command touches every managed interface at once. Pinned interfaces are skipped silently. See `proteus wiki mac-recipes` for OUI pool selection, pinning, and per-connection MACs.

## Set rotation cadence

Most users let the timer do the work after the first manual rotation. Default cadence is 2h.

```sh
proteus timer status
sudo proteus timer set rotate --interval 1h
```

Tradeoffs:

- **Faster (`30m`, `1h`)** — more privacy, more DHCP renewals. Some networks throttle or rate-limit renewals at very short intervals; `30m` is a reasonable floor for most networks.
- **Slower (`4h`, `8h`)** — quieter on the network, bigger correlation window for whoever is watching.
- **`0`** — disable the periodic timer entirely; rely on probe-driven and manual rotation only.

Restore the default:

```sh
sudo proteus timer reset rotate
```

See `proteus wiki timer` for cadence syntax (`30m`, `hourly`, `*-*-* 06:00:00`) and `proteus wiki rotation` for the full trigger story.

## Tweak config without editing TOML

`/etc/proteus/config.toml` is hand-editable, but `proteus config` is the first-class path. It round-trips through `toml_edit` so your comments and formatting survive.

```sh
proteus config show
sudo proteus config disable bluetooth --reason "I use AirPods, leave it alone" --yes
sudo proteus config set mac.rotation_interval 30m --yes
proteus config get mac.rotation_interval
```

`disable --reason` writes a comment above the section so `proteus status` can surface why a feature is off. The comment shows up in the same shape as the auto-defer messages — explicit override and automatic detect-and-defer use the same surface.

Verify the result:

```sh
proteus status --json | jq .features
```

For the full schema, see `proteus wiki config`.

## Daily use mental model

- **Most days, do nothing.** Proteus rotates on schedule, on probe-driven connectivity loss, and on link change. Set it up once and forget it.
- **When joining a new network**, nothing special. Proteus does its thing. Captive portals get a fresh MAC at join under the default policy.
- **When something breaks**, run `proteus doctor` first. Then `proteus wiki troubleshooting` for the symptom-based recovery recipes. `proteus revert` (planned, phase G) is the panic button; until it lands, use `proteus reset` to clear config to defaults or the manual rollback recipe in `proteus wiki uninstall`.
- **When you want a clean slate**, `sudo proteus reset --yes` clears your config back to defaults. The cached original MACs and hostname are not touched — those are sacred.

There is no daemon. The CLI is the whole product. Two systemd timers and a boot oneshot do the work.

## Things to know

**Originals are sacred.** The MAC and hostname Proteus saw the first time it ran are cached in `/var/lib/proteus/state.json`. Never re-captured. `proteus reset` does not touch them. Only `proteus uninstall --purge` removes them. If you ever want to fully revert, the cache is the source of truth.

**Captive portals are first-class.** Proteus knows the difference between "Internet down" and "stuck behind a hotel splash page." It will not loop-rotate behind a portal. Probe failures classified as portal-caused never trigger MAC rotation. Periodic rotation is suppressed while you are authed. See `proteus wiki captive-portals` for the policy matrix and the loop-prevention invariants.

**Scope is L2-L4 plus network-joining protocols.** Proteus is not a VPN, not a DNS resolver, not a browser fingerprint tool, not a tracker blocker. Compose it with the right tool for each layer. Tor Browser or librewolf for the browser. dnscrypt-proxy or Pi-hole for DNS. Mullvad or Tor for traffic correlation. See `proteus wiki threat-model`.

**No telemetry, no update check.** No network egress beyond your configured probe targets. Ever.

**`proteus revert` works at every release.** Backing out is a real option from day one.

## Where to go next

- `proteus wiki concepts` — the mental model in detail: identifiers, rotation, captive portals, managed files, revert.
- `proteus wiki recipes` — common scenarios, ready to copy.
- `proteus wiki hostile-environments` — the playbook for cafes, hotels, conferences, airports.
- `proteus wiki cli` — full command reference, exit codes, JSON schemas.
- `proteus wiki troubleshooting` — when things break, what to check first.
- `proteus wiki threat-model` — what Proteus does not do and which tool to reach for instead. Read this before trusting Proteus with anything that matters.
