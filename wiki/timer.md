`proteus timer` is a thin wrapper over `systemctl` for the systemd units Proteus owns. It exists so you don't hand-edit unit files or remember `OnUnitActiveSec=` syntax. Drop-ins under `/etc/systemd/system/proteus-*.timer.d/` are managed; everything else stays out of the way.

For the rotation triggers and trade-offs around cadence, see `proteus wiki rotation`. For the full CLI surface, see `proteus wiki cli`. For the mental model, see `proteus wiki concepts`.

## The named units

Proteus exposes four short names. Each maps to one systemd unit.

- **`rotate`** — `proteus-rotate.timer`. Scheduled MAC rotation cadence. Default 2h. The main knob most users want to tune.
- **`check`** — `proteus-check.timer`. Probe-driven rotation check. Default 5m. Mostly redundant once the NetworkManager dispatcher is installed; safe to disable if you trust the event triggers.
- **`resume`** — `proteus-resume.timer`. Fires on resume from suspend / hibernate (paired with `proteus-resume.service`). Default off until phase C ships.
- **`boot`** — `proteus-boot.service`. Oneshot at boot, ordered after `network-online.target`. Not a timer in the systemd sense; surfaced under `proteus timer` for symmetry. Cannot be set to a custom interval — `proteus timer set boot ...` exits `65`.

`proteus timer list` prints all four with their defaults.

## Subcommands

All read commands work for any user. Mutating commands require root and exit `66` otherwise. If systemd isn't running (no `/run/systemd/system`), every subcommand exits `70`.

### `proteus timer status [--json]`

Lists every Proteus-owned unit with: enabled state, active state, current interval, next/last fire (timers only), and whether a drop-in override is in effect (marked with `*`). The first thing to run when something looks off.

```sh
proteus timer status
proteus timer status --json | jq '.timers[] | select(.has_override)'
```

### `proteus timer list [--json]`

Lists the four named units and their unit-file defaults. Static — does not query systemd. Use this to discover what names are valid.

### `proteus timer enable <name>`

Runs `systemctl enable --now <unit>` for timers, `systemctl enable <unit>` for the boot/resume oneshots. Mutating; needs root.

### `proteus timer disable <name>`

Runs `systemctl disable --now <unit>`. Mutating; needs root. The unit stays installed — `enable` puts it back. To remove the unit files entirely, use `proteus uninstall` (phase G).

### `proteus timer set <name> --interval <duration>`

Writes a drop-in at `/etc/systemd/system/proteus-<name>.timer.d/override.conf`, runs `daemon-reload`, and restarts the unit. Mutating; needs root. Only valid for timer units (`rotate`, `check`, `resume`); `set` against `boot` exits `65` because the boot oneshot has no cadence to set.

```sh
sudo proteus timer set rotate --interval 30m
sudo proteus timer set rotate --interval hourly
sudo proteus timer set rotate --interval '*-*-* 06:00:00'
```

### `proteus timer reset <name>`

Removes the drop-in (and the now-empty `*.timer.d/` directory if nothing else is in it), runs `daemon-reload`, and restarts the unit. Restores the cadence baked into the unit file. Mutating; needs root.

### `proteus timer logs <name> [--lines N]`

Equivalent to `journalctl -u <unit> -n <lines> --no-pager`. Default 50 lines. Read-only.

## Duration syntax

`--interval` accepts three shapes. They map to systemd directives transparently.

**Compact durations.** `30s`, `5m`, `1h`, `2h`, `1d`, `1w`. Translates to `OnUnitActiveSec=<seconds>`. This is the right knob for "every N time after the last successful run", which is what you almost always want for rotation. Recognised suffixes: `s`/`sec`/`seconds`, `m`/`min`/`minutes`, `h`/`hr`/`hours`, `d`/`day`/`days`, `w`/`wk`/`weeks`. A bare number is interpreted as seconds.

**Named systemd cadences.** `minutely`, `hourly`, `daily`, `weekly`, `monthly`, `quarterly`, `semiannually`, `yearly`/`annually`. Passed straight through to `OnCalendar=`. Aligned to wall-clock boundaries — `hourly` fires at the top of the hour, not "an hour after last run". Useful when you want predictability over freshness.

**Calendar expressions.** Anything containing whitespace, `*`, or `:` is treated as a raw `OnCalendar=` value. `*-*-* 06:00:00` fires at 6am every day. `Mon..Fri 09:00` fires weekday mornings. Use `man systemd.time` for the full grammar.

Zero-duration intervals are rejected. Empty strings are rejected. Unknown unit suffixes are rejected with the list of valid ones.

## The drop-in mechanism

Each `set` writes one file at `/etc/systemd/system/proteus-<name>.timer.d/override.conf`.

The file carries a two-line header:

```text
# managed by proteus v<version>
# do not edit; manage via `proteus timer set ...`
```

…followed by a `[Timer]` section with the new directive. The drop-in clears the unit-file `OnCalendar=` first (systemd otherwise *appends* triggers) and then sets exactly one of `OnUnitActiveSec=` or `OnCalendar=`. Other unit-file directives — `Persistent=`, `RandomizedDelaySec=`, `Unit=` — are inherited untouched.

Drop-ins survive package upgrades. The unit file underneath can be replaced freely; the drop-in keeps your cadence. `proteus diff` (phase G) flags drift if anything outside the managed header has been edited by hand.

If you delete the drop-in manually instead of running `proteus timer reset`, the next `daemon-reload` will pick up the change and the cadence reverts to the default. The next `proteus timer status` will show `*` removed from that row.

## Recipes

**Rotate every 30 minutes instead of 2 hours.**

```sh
sudo proteus timer set rotate --interval 30m
proteus timer status
```

**Rely on NM dispatcher events; disable the polling check timer.**

```sh
sudo proteus timer disable check
```

The dispatcher script under `/etc/NetworkManager/dispatcher.d/01-proteus` rotates on every `up` event subject to the cooldown window — much faster than 5-minute polling. See `proteus wiki rotation` for the event-driven trigger story.

**Restore the rotate timer's default cadence.**

```sh
sudo proteus timer reset rotate
```

**Inspect every Proteus unit at once.**

```sh
proteus timer status
```

**Tail the last 50 lines of rotation logs.**

```sh
proteus timer logs rotate --lines 50
```

**Pause scheduled rotation without uninstalling.** Disable both timers. The boot oneshot and the NM dispatcher hook still run unless you remove them too.

```sh
sudo proteus timer disable rotate
sudo proteus timer disable check
```

**Switch rotate to a wall-clock cadence.** `OnCalendar=` makes rotation predictable but loses the "N time since last run" property — if the host was suspended at 6am, the 6am rotation is missed unless `Persistent=true` is set in the unit file (it is, for `proteus-rotate.timer`).

```sh
sudo proteus timer set rotate --interval 'hourly'
sudo proteus timer set rotate --interval '*-*-* 06:00:00'
```

## Exit codes

- `0` — success.
- `1` — generic error (unknown timer name, systemctl/journalctl failed, drop-in write failed).
- `65` — config error (malformed interval, `set` or `reset` against the `boot` oneshot).
- `66` — not root for a mutating command.
- `70` — systemd not detected (no `/run/systemd/system`). Returned by every subcommand that needs systemd, including read-only `status` and `logs`.

These match the codes documented in `proteus wiki cli` for the rest of the CLI.

## Where to go next

- `proteus wiki rotation` — what each timer actually does, the cooldown window, event-driven triggers.
- `proteus wiki cli` — the full CLI reference and the JSON schemas a wrapper reads.
- `proteus wiki concepts` — identifiers, what's sacred, the rotation mental model.
- `man systemd.time` — full grammar for calendar expressions.
- `man systemd.timer` — the unit-file directives Proteus generates drop-ins for.
