Rotation is how Proteus keeps your network identifiers from settling into a stable fingerprint over time. Two systemd timers and a boot oneshot drive everything; the CLI is the same surface they call into. There is no daemon.

For the mental model behind rotation, see `proteus wiki concepts`. For the probe quorum that decides "the network is down", see `proteus wiki probes`. For why captive portals are an exception, see `proteus wiki captive-portals`.

## The two timers and the boot oneshot

Three units do the work. They all shell out to the same `proteus` binary.

- `proteus-rotate.timer` — fires every 2h by default. On each tick, runs `proteus rotate` on every managed interface. This is the scheduled cadence.
- `proteus-check.timer` — fires every 5m. Runs the probe quorum (see `proteus wiki probes`). If quorum says "down" and the portal classifier says "not a portal", invokes a rotation. This is the reactive trigger.
- `proteus-boot.service` — oneshot, ordered after `network-online.target`. Applies the current config and does the first rotation of the session. This is what you want — fresh MAC at boot, before anything you do leaks the previous one.

All three are installed and enabled by `proteus apply`. None of them is required for the CLI to work; you can disable both timers and still call `sudo proteus rotate` by hand.

## Configuration

Rotation cadence and triggers live under `[rotation]` in `/etc/proteus/config.toml`:

```toml
[rotation]
interval = "2h"          # scheduled rotation cadence
on_probe_fail = true     # rotate when probe quorum says "down"
on_link_change = true    # rotate when an interface comes up after being down (e.g., re-plugged Ethernet)
on_ssid_change = true    # rotate when joining a new Wi-Fi SSID
```

`interval` accepts any systemd duration (`30m`, `1h`, `4h`, `8h`). Setting it to `0` disables the scheduled timer; the probe-driven and link-change triggers still apply unless you turn them off too.

The three boolean triggers are independent. Disabling `on_probe_fail` keeps the check timer running for status reporting but stops it from rotating. Disabling `on_link_change` or `on_ssid_change` is uncommon; the defaults are conservative.

## Per-feature triggers

Different identifiers rotate on different signals. The matrix:

- **Wi-Fi MAC** — scheduled, probe-fail, SSID-change. Per-NM-connection profile when possible (more compatible with NM workflows than per-interface). See `proteus wiki mac-recipes`.
- **Ethernet MAC** — scheduled, probe-fail. A wired drop is usually the cable being pulled, not a network failure, but rotate-on-probe-fail still applies for parity with Wi-Fi. Use `proteus pin <iface>` if your switch port is MAC-locked.
- **Bluetooth alias** — scheduled. Alias rotation is cheap (no link reset) so it can run hourly without disrupting anything. See `proteus wiki bluetooth`.
- **BLE Resolvable Private Address** — handled by the controller on its own schedule. Proteus enables the mode; the controller decides cadence. Nothing for the rotation timer to do here.
- **Hostname** — opt-in, only when `[hostname] rotate_with_mac = true`. Off by default; many users want a stable hostname even when MACs rotate.
- **DUID (DHCPv6)** — coupled with MAC rotation per-interface. Otherwise DHCPv6 would leak the same client identity across MAC rotations. See `proteus wiki dhcp`.
- **IPv6 IID** — derives from the MAC under stable-privacy. Rotates implicitly when the MAC rotates; nothing extra to schedule.

The pattern: MAC is primary, the L3 identifiers that derive from or correlate with it are coupled, the L7-style identifiers (hostname, alias) are independent and opt-in.

## What does not rotate

Some things are intentionally never touched by the timers.

- **Cached "original" MACs** in `/var/lib/proteus/state.json`. Captured once, the first time Proteus sees a system, never rewritten. This is what `proteus revert` and `proteus original` read from. Cross-ref `proteus wiki concepts`.
- **Pinned interfaces and connections.** `proteus pin <iface>` or `proteus pin <connection>` freezes a MAC. Pinned targets are skipped by both timers. Use this for corporate networks, hotel Wi-Fi after auth, MAC-bound DHCP reservations. Cross-ref `proteus wiki mac-recipes`.
- **Captive-portal-classified states.** When the portal classifier reports `portal-required` or `portal-authed`, periodic rotation is suppressed and probe failures classified as portal-caused never trigger rotation. This is how the "rotate behind a portal forever" loop is avoided. Cross-ref `proteus wiki captive-portals`.
- **`/etc/machine-id`.** Out of scope. TPM, journald, and dbus all reference it; rotating it is real breakage risk, not a fingerprint win.

If you see Proteus skip a rotation, `proteus status` will name the reason — pinned, portal-authed, cooldown, or disabled.

## Cooldown

After any rotation, a 60s cooldown blocks the next probe-driven check. The freshly-rotated stack needs time to come up — DHCP renewal, IPv6 router-solicitation and DAD, NM connection-up signals all take real seconds. Without cooldown, the first probe round after a rotation would race the link coming back and trigger another rotation. See `proteus wiki probes` for the full quorum logic.

The cooldown does not block the scheduled timer. If `proteus-rotate.timer` fires inside the cooldown window, it runs as normal — the cooldown is a probe-driven-only debounce.

## Inspecting timer state

`proteus timer status` is the high-level view of every Proteus timer: enabled vs disabled, currently active, current cadence, next fire, last fire, and whether a user override is in effect. `--json` for wrappers.

```sh
proteus timer status
proteus timer status --json
proteus timer logs rotate --lines 50
```

Standard systemd tools still work as a fallback:

```sh
systemctl list-timers proteus-*
journalctl -u proteus-rotate -n 50
journalctl -u proteus-check -n 50
journalctl -u proteus-boot -n 50
```

`systemctl status proteus-rotate.timer` shows the next scheduled fire. `journalctl -t proteus -n 100` aggregates everything the binary itself logs, regardless of which unit invoked it.

`proteus status` is the high-level view of identifiers: when the last rotation ran, what triggered it, which interfaces are managed vs pinned, and the current portal classification.

## Managing timers via CLI

`proteus timer` is the first-class CLI surface for changing cadences and enabling/disabling rotation jobs without hand-editing systemd units. Drop-ins land at `/etc/systemd/system/proteus-<name>.timer.d/override.conf` and carry a `# managed by proteus` header. `daemon-reload` and a unit restart happen automatically.

```sh
# See current timer state
proteus timer status
proteus timer status --json

# List the timer types Proteus defines
proteus timer list

# Change rotate cadence to every 30 min
sudo proteus timer set rotate --interval 30m

# Disable the polling check timer (use NM dispatcher events instead)
sudo proteus timer disable check

# Enable rotate-on-resume
sudo proteus timer enable resume

# Reset a timer to the unit-file default
sudo proteus timer reset rotate

# Inspect journald
proteus timer logs rotate --lines 50
```

`--interval` accepts compact durations (`30s`, `5m`, `2h`, `1d`, `1w`), the named systemd cadences (`hourly`, `daily`, ...), and full systemd calendar expressions (e.g. `*-*-* 04:00:00`). For "every N time" Proteus uses `OnUnitActiveSec=` rather than `OnCalendar=` because it tracks the last successful run rather than wall-clock alignment.

Read commands (`status`, `list`, `logs`) work for any user. Mutating commands (`enable`, `disable`, `set`, `reset`) require root and exit `66` otherwise. If systemd isn't running, every subcommand exits `70`.

## Manual triggers

The CLI is the same surface the timers call. Manual invocations work identically.

```sh
sudo proteus rotate                        # rotate all managed interfaces now
sudo proteus rotate --iface wlan0          # single interface by name
sudo proteus rotate --connection home-wifi # single NM connection profile
sudo systemctl start proteus-rotate.service # same effect via the service unit
```

Manual rotations respect pins and the portal classifier. To rotate a pinned interface, unpin first (`proteus unpin <target>`), rotate, then pin again. Behind a portal, rotation is suppressed by design — see `proteus wiki captive-portals`.

## Disabling rotation entirely

To stop both timers without uninstalling:

```sh
sudo systemctl disable --now proteus-rotate.timer proteus-check.timer
```

The boot oneshot still runs at boot unless you also disable `proteus-boot.service`. The CLI continues to work for one-off rotations regardless. Re-enable with `enable --now` when you want the timers back.

A lighter option: leave the timers running and set `interval = "0"` plus `on_probe_fail = false` in `[rotation]`. The units stay enabled and visible in `systemctl list-timers`, but they don't rotate. This is closer to "paused" than "disabled".

## Tuning rotation cadence

The default of 2h is a balance. The trade-offs:

- **Shorter (1h, 30m)** — more privacy, more DHCP renewals, more NM connection events. Some networks throttle frequent DHCP requests; some captive portals require re-auth on every link event. If you live on networks that are sensitive to either, shorter is worse.
- **Longer (4h, 8h)** — less network noise, fewer disruptions, but more long-lived MAC-network correlation. An observer who stays on the same network as you for an afternoon sees one MAC.
- **Default 2h** — short enough that no single observer sees a stable identity for a full work session, long enough that DHCP and NM aren't constantly churning.

The reactive triggers (`on_probe_fail`, `on_link_change`, `on_ssid_change`) matter more than the cadence in practice. A laptop that moves between networks rotates on every join regardless of `interval`. The scheduled cadence mostly matters for stationary use.

## Event-driven triggers

The two timers are the baseline. On top of them, Proteus ships two event-driven hooks that react to network and power-state changes immediately, without any polling.

- **NetworkManager dispatcher** (`/etc/NetworkManager/dispatcher.d/01-proteus`) — NM invokes this script on every connection state change. On an `up` event the hook calls `proteus rotate --iface <name> --yes`, subject to the cooldown window (default 60s, matches `[probes] cooldown`). On `connectivity-change` it logs only — rotating behind a captive portal would loop. `down`, `pre-up`, `pre-down`, `vpn-*`, `dhcp*`, and `hostname` events log only. This gives you near-immediate rotation on disconnect/reconnect, much faster than the 5-minute check timer.
- **Sleep hook** (`proteus-resume.service`, `WantedBy=suspend.target hibernate.target hybrid-sleep.target suspend-then-hibernate.target`) — fires on resume from suspend or hibernate. Runs `proteus rotate --yes` so you wake with a fresh MAC. Same hardening profile as the other proteus units (`ProtectSystem=full`, `CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE`, etc.).

With both hooks installed, `proteus-check.timer` (5-minute polling) becomes a backup safety net. Users who trust the event triggers can disable it:

```sh
sudo systemctl disable --now proteus-check.timer
```

The scheduled `proteus-rotate.timer` is independent and stays useful — it covers the stationary case where nothing else is changing.

**Architectural note: no daemon.** Each event is a short-lived CLI invocation. The dispatcher is a bash script invoked by NM; the sleep hook is a oneshot systemd service. There is no long-lived process. This honors the "no daemon" invariant from `docs/PLAN.md`.

See `dist/networkmanager/README.md` for installation details and the dispatcher event matrix.

## Where to go next

- `proteus wiki probes` — the quorum logic that decides when "down" really means down, plus the cooldown rationale.
- `proteus wiki captive-portals` — why portal-classified failures are excluded from rotation, and the per-visit fresh-MAC flow.
- `proteus wiki mac-recipes` — pinning, OUI pools, per-connection vs per-interface, common patterns.
- `proteus wiki concepts` — the mental model: identifiers, what's sacred, what's managed.
- `proteus help rotate` — full CLI reference for the rotate subcommand.
