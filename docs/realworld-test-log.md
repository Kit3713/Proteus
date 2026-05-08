# Real-world testing log

A living log of `proteus apply` runs against networks the maintainers and
contributors actually use day-to-day. Every entry captures one network on
one host on one date — what was tested, what worked, what broke, and any
GitHub issue filed to track a regression.

This is the qualitative companion to the unit / integration / container
matrix in CI. The container matrix proves Proteus boots on Fedora / Debian
/ Gentoo / Alpine / Arch under `nm` / `networkd` / `raw`. This log proves
Proteus survives contact with **real** Wi-Fi: hotel captive portals,
conference NAT64, café DHCP throttling, airport-lounge enterprise
802.1X, residential ISP gateway quirks.

## How to use this file

1. Pick a network you intend to use (café, hotel, conference, airport,
   home, work).
2. `proteus status` and `proteus current --json` before joining — capture
   the baseline.
3. Join the network. Run through the persona / rotation / portal
   workflow you want to exercise.
4. `proteus status` and `proteus diff` after each action.
5. **File a GitHub issue for any anomaly**, even if you can't reproduce
   it. Tag with `triage` and reference this log entry's row id (e.g.
   `realworld-2026-05-08-002`).
6. Add a row below. Keep it terse — the issue carries the detail.

The columns are deliberately fixed; if a new column is needed, propose
in a PR rather than adding ad hoc text columns.

## Schema

| Column | Meaning |
|---|---|
| `date` | ISO 8601 (YYYY-MM-DD) |
| `network type` | `home`, `work`, `cafe`, `hotel`, `airport`, `conference`, `mobile-tether`, `enterprise-eap`, `other` |
| `distro` | `fedora-43`, `debian-13`, `gentoo`, `alpine-3.20`, `arch`, etc. — kernel version optional in the same cell |
| `persona` | the active persona id (`iphone-15`, `pixel-9`, `none`, etc.) |
| `proteus version` | `git describe --tags --dirty` output |
| `observed behaviour` | one sentence; "applied cleanly", "fresh MAC per portal visit triggered", "DHCP renewal denied", etc. |
| `bugs filed` | comma-separated GitHub issue numbers, or `—` for "no anomaly" |

## Log

| date | network type | distro | persona | proteus version | observed behaviour | bugs filed |
|---|---|---|---|---|---|---|
| _(no entries yet)_ | | | | | | |

## Reporting bug categories worth tagging

When you file an issue from a row in this log, prefer one of the existing
labels so triage rolls up cleanly. The categories below have shown up
historically — add a label suggestion if you hit something new.

- `realworld-portal` — captive portal misclassified, redirect loop, MAC
  rotation behind portal, or `fresh_mac_per_visit` failed to trigger.
- `realworld-dhcp` — DHCPv4 / DHCPv6 renewal throttled, DUID reused
  across rotations, server rejected client identity.
- `realworld-rf` — RF tx-power knob over-reduced range, scan-random-mac
  broke association on a quirky AP, regulatory domain mismatch.
- `realworld-enterprise` — 802.1X auth rejected on anonymous outer
  identity, EAP-PEAP / EAP-TLS specific failure, RADIUS server didn't
  accept rotated client identifier.
- `realworld-nm` — NetworkManager connection state stuck after
  `proteus apply`, `Activate(Connection)` returned a Failed state
  unexpectedly.
- `realworld-bluetooth` — adapter alias didn't rotate, BLE RPA mode not
  honoured by controller, device pairing broke after alias rotation.
- `realworld-distro` — distro-specific path (e.g. networkd-only
  Alpine, OpenRC interaction, custom dispatcher hooks).

## Triage SLA

A High-severity finding from this log opens a `v0.4.x.Y-beta` patch
slot per `docs/ROADMAP.md` ("Verification — end-to-end", item 4).
Anything Medium or below joins the regular issue queue.

## Cross-refs

- `docs/ROADMAP.md` — Stream 10 frontier item; this file is the
  living-doc deliverable.
- `wiki/real-world-testing.md` — user-facing "what to expect when
  travelling with Proteus" page.
- `wiki/captive-portals.md` — portal classifier and per-visit
  fresh-MAC flow.
- `docs/security/dbus-surface.md` — privileged DBus surface a
  reviewer should look at alongside real-world traces.
