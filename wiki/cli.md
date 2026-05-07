Full reference for the `proteus` command: every subcommand, every flag, every documented exit code, every JSON output schema. This page is the contract a wrapper author (GUI builder, automation script) reads.

## Synopsis

```
proteus [-v|--verbose ...] [-q|--quiet ...] [--config <path>] [--state <path>] [--no-color] <SUBCOMMAND>
```

## Global flags

These apply to every subcommand. They must precede the subcommand name.

- `-v`, `--verbose` — increase log verbosity. Repeatable: `-v` debug, `-vv` trace.
- `-q`, `--quiet` — decrease log verbosity. Repeatable: `-q` warn, `-qq` error.
- `--config <PATH>` — override `/etc/proteus/config.toml`. Useful for testing.
- `--state <PATH>` — override `/var/lib/proteus/state.json`. Useful for testing.
- `--no-color` — disable ANSI colors on stderr log output. Honors `NO_COLOR` too.
- `-h`, `--help` — print help and exit 0.
- `-V`, `--version` — print version and exit 0.

`RUST_LOG` overrides `-v` / `-q` when set; see [Logging](#logging).

## Subcommands

Alphabetical. Every subcommand parses today; the ones marked **stub** return exit `64` with a one-line pointer to the phase that brings the implementation. Mutating commands require root and accept `--yes` for non-interactive runs. Read commands degrade quietly when files aren't readable.

### `apply` — phase **stub** (lands D)

```
proteus apply [--yes]
```

Apply the current config to the system: rotate MACs that need rotating, write managed files under `/etc/`, install systemd timers. Idempotent — running ten times converges to the same state as once. Mutating; requires root.

Flags: `--yes` proceed without confirmation.
Exit: `0` success · `1` generic · `64` stub · `65` config error · `66` not root · `70` system unsupported.
Example: `sudo proteus apply --yes`

### `current` — phase **A**

```
proteus current [--json] [--iface <NAME>]
```

Show the current network identifiers your system is handing out right now. Read-only.

Flags: `--json` machine-readable (see [`current` JSON](#proteus-current---json)) · `--iface <NAME>` limit to one interface.
Exit: `0` success · `1` generic.
Example: `proteus current --json | jq '.[] | select(.type == "wifi")'`

### `diff` — phase **stub** (lands G)

```
proteus diff [--json]
```

Show the delta between current config, built-in defaults, and live system state. Flags drift from managed files (the SHA in the `# managed by proteus` header is compared to the file's actual SHA). Read-only.

Flags: `--json` machine-readable diff.
Exit: `0` no drift · `1` drift detected · `64` stub.

### `dry-run` — phase **stub** (lands G)

```
proteus dry-run <SUBCOMMAND> [args...]
```

Preview the mutations a command would make without performing them. Every mutator goes through a `Plan` enum that can be either previewed or executed. Read-only.

Exit: `0` plan empty · `1` plan would have failed · `64` stub.
Example: `sudo proteus dry-run rotate --iface wlan0`

### `help` — phase **A**

```
proteus help [<feature>]
```

Friendly entry point. With no argument, lists embedded wiki pages. With a feature name, prints the matching wiki page. Effectively an alias for `wiki <feature>` with a friendlier zero-arg behavior. Read-only.

Note: `proteus help` is **not** the same as `proteus --help`. `proteus --help` is the clap usage; `proteus help` opens the wiki.

Exit: `0` success or no-arg listing · `1` no such page.

### `original` — phase **A**

```
proteus original [--json]
```

Show the cached original MACs and hostname Proteus snapshotted on first run. Sacred — never re-captured. State path defaults to `/var/lib/proteus/state.json`; override with `--state`. Read-only.

Flags: `--json` machine-readable (see [`original` JSON](#proteus-original---json)).
Exit: `0` success (whether or not a cache exists) · `1` generic read failure.

The cache itself only populates from phase B onward (when the first mutating commands run).

### `pin` — phase **stub** (lands B)

```
proteus pin <TARGET>
```

Pin a MAC to a specific interface or NetworkManager connection. Pinned targets are skipped by both scheduled and probe-driven rotation. For environments that lock you to one MAC: corporate networks, hotel Wi-Fi after auth, MAC-bound DHCP reservations. `<TARGET>` is interface name or NM connection profile (profile preferred when ambiguous). Mutating; requires root.

Exit: `0` success · `1` generic · `64` stub · `66` not root.
Example: `sudo proteus pin "Home Wi-Fi"`

### `reset` — phase **stub** (lands G)

```
proteus reset [--yes]
```

Clear `/etc/proteus/config.toml` to defaults and re-apply. The "I tinkered and broke it" hatch. Deliberately does **not** touch the cached original MACs in `state.json` — those remain sacred. Mutating; requires root.

Exit: `0` success · `64` stub · `66` not root.

### `revert` — phase **stub** (lands G)

```
proteus revert [--yes]
```

Restore everything to the cached originals. The panic button. **Invariant**: must work at every commit from phase B onward — if a feature can't be backed out cleanly, it does not ship. Mutating; requires root.

Exit: `0` success · `1` generic · `64` stub · `66` not root.
Example: `sudo proteus revert --yes`

### `rotate` — phase **stub** (lands B)

```
proteus rotate [--iface <NAME>] [--yes]
```

Generate a fresh MAC and apply it. With no `--iface`, rotates every managed interface. Skips pinned interfaces. Avoids picking a MAC that matches the gateway or anything else in the local ARP table. Mutating; requires root.

Flags: `--iface <NAME>` limit to one interface · `--yes` proceed without confirmation.
Exit: `0` success · `1` generic · `64` stub · `66` not root.
Example: `sudo proteus rotate --iface wlan0 --yes`

### `show-config` — phase **A**

```
proteus show-config [--json]
```

Print the active config from `/etc/proteus/config.toml` (override with `--config`). When the file is missing, prints a note that defaults are in effect and exits 0. Read-only.

Flags: `--json` emit JSON instead of TOML (see [`show-config` JSON](#proteus-show-config---json-and-show-defaults---json)).
Exit: `0` success (including missing file) · `65` parse failure · `66` permission denied · `1` other read failure.

### `show-defaults` — phase **A**

```
proteus show-defaults [--json]
```

Print the built-in default config. Use this to see every knob and its default before writing your own `config.toml`. Read-only; never touches the filesystem.

Flags: `--json` emit JSON instead of TOML.
Exit: `0` success.

### `status` — phase **A**

```
proteus status [--json]
```

The overall system + per-feature status report: whether systemd, NetworkManager, BlueZ, and systemd-resolved are present; physical interfaces with their current MAC; per-feature `applied / skipped (reason) / failed (reason)`. Read-only.

Flags: `--json` machine-readable (see [`status` JSON](#proteus-status---json)).
Exit: `0` success.

### `timer` — phase **C**

```
proteus timer <SUBCOMMAND> [args...]
```

First-class CLI surface for managing the systemd timers Proteus owns. Read sub-subcommands (`status`, `list`, `logs`) work for any user; mutating ones (`enable`, `disable`, `set`, `reset`) require root and exit `66` otherwise. Every sub-subcommand exits `70` if systemd isn't detected.

Timer names are short identifiers, not full unit names: `rotate` -> `proteus-rotate.timer`, `check` -> `proteus-check.timer`, `resume` -> `proteus-resume.timer`, `boot` -> `proteus-boot.service`.

#### `timer status`

```
proteus timer status [--json]
```

List every Proteus timer with its enabled/active state, current cadence, next fire, last fire, and whether a user override drop-in is active. Read-only.

Flags: `--json` machine-readable.
Exit: `0` success · `70` no systemd.

#### `timer list`

```
proteus timer list [--json]
```

List the timer "types" defined by Proteus along with their default cadence and one-line description. Read-only; does not consult systemd at all.

Flags: `--json` machine-readable.
Exit: `0` success.

#### `timer enable <NAME>`

```
proteus timer enable <NAME>
```

Run `systemctl enable --now <unit>` for the named timer (or just `enable` for the boot oneshot). Mutating; requires root.

Exit: `0` success · `1` generic · `66` not root · `70` no systemd.
Example: `sudo proteus timer enable resume`

#### `timer disable <NAME>`

```
proteus timer disable <NAME>
```

Run `systemctl disable --now <unit>` for the named timer. Mutating; requires root.

Exit: `0` success · `1` generic · `66` not root · `70` no systemd.
Example: `sudo proteus timer disable check`

#### `timer set <NAME> --interval <DURATION>`

```
proteus timer set <NAME> --interval <DURATION>
```

Change a timer's cadence. Writes a drop-in at `/etc/systemd/system/proteus-<name>.timer.d/override.conf` carrying a `# managed by proteus` header, then runs `systemctl daemon-reload` + `systemctl restart proteus-<name>.timer`. Mutating; requires root.

`<DURATION>` accepts:
- Compact durations: `30s`, `5m`, `2h`, `1d`, `1w`. Renders to `OnUnitActiveSec=<seconds>` so cadence tracks the unit's last successful run rather than wall-clock alignment.
- Named systemd cadences: `hourly`, `daily`, `weekly`, `monthly`, `yearly`. Renders to `OnCalendar=<name>`.
- Full systemd calendar expressions (e.g. `*-*-* 04:00:00`). Renders to `OnCalendar=<expr>` verbatim.

Exit: `0` success · `1` generic · `65` config (bad interval, or non-timer unit) · `66` not root · `70` no systemd.
Example: `sudo proteus timer set rotate --interval 30m`

#### `timer reset <NAME>`

```
proteus timer reset <NAME>
```

Remove the drop-in for a timer and reset to the unit-file default. Runs `daemon-reload` + `restart`. Mutating; requires root.

Exit: `0` success · `1` generic · `65` config (non-timer unit) · `66` not root · `70` no systemd.

#### `timer logs <NAME> [--lines N]`

```
proteus timer logs <NAME> [--lines N]
```

Tail recent journald logs for a timer's unit (`journalctl -u <unit> -n N --no-pager`). Read-only.

Flags: `--lines N` (default 50).
Exit: `0` success · `1` generic · `70` no systemd.

### `uninstall` — phase **stub** (lands G)

```
proteus uninstall [--purge] [--yes]
```

Full removal. Runs `revert` first, removes the binary, removes the systemd timers. Mutating; requires root.

Flags: `--purge` also delete `/etc/proteus/` and `/var/lib/proteus/`. Without `--purge`, the original-MAC cache is preserved so a reinstall can restore the same identity. `--yes` proceed without confirmation.
Exit: `0` success · `64` stub · `66` not root.

### `unpin` — phase **stub** (lands B)

```
proteus unpin <TARGET>
```

Remove a pin previously set with `pin`. The target rejoins the rotation pool. `<TARGET>` is interface name or NM connection profile. Mutating; requires root.

Exit: `0` success · `1` no such pin · `64` stub · `66` not root.

### `wiki` — phase **A**

```
proteus wiki [<page>]
```

Browse the embedded wiki. With no argument, lists available pages. With a page name, prints that page's Markdown to stdout. Read-only.

Exit: `0` success · `1` no such page.
Example: `proteus wiki cli`

## Exit codes

Stable. New codes may be added in future versions; existing codes never change meaning. Defined in `src/lib.rs` (`exit::*`).

| Code | Constant | Meaning |
|------|----------|---------|
| 0 | `SUCCESS` | success |
| 1 | `GENERIC_ERROR` | generic / unclassified error |
| 2 | _(clap)_ | invalid arguments — usage error from the parser |
| 64 | `NOT_IMPLEMENTED` | command parses but the feature has not landed yet — stderr names the phase |
| 65 | `CONFIG_ERROR` | config parse failure or invalid value |
| 66 | `PERMISSION_ERROR` | mutating command run without root, or read denied on a privileged file |
| 70 | `SYSTEM_NOT_SUPPORTED` | not Linux, no systemd, or another precondition the binary can't satisfy |

Wrappers should treat `0` as success and any non-zero as failure. Inspect stderr or journald for the human reason; do **not** scrape stdout for status — `--json` is the contract.

## JSON output schemas

`--json` on a read command emits one JSON document to stdout, pretty-printed with a trailing newline. Stderr is reserved for log lines and human errors. Schemas reflect the current binary (phase A); fields may be added in future phases. Wrappers must ignore unknown fields. Existing fields keep their names and types.

### `proteus status --json`

```json
{
  "proteus_version": "0.1.0",
  "phase": "A",
  "system": {
    "systemd": true,
    "network_manager": true,
    "bluez": false,
    "systemd_resolved": true
  },
  "interfaces": [
    { "name": "wlan0", "mac": "aa:bb:cc:dd:ee:ff", "kind": "wifi",     "wireless": true  },
    { "name": "eth0",  "mac": "11:22:33:44:55:66", "kind": "ethernet", "wireless": false }
  ],
  "features": [
    { "name": "mac-rotation", "state": "not implemented", "note": "phase B" },
    { "name": "hostname",     "state": "not implemented", "note": "phase D" }
  ]
}
```

- `interfaces[].mac` is `null` when the address is unreadable. `interfaces[].kind` is `wifi`, `ethernet`, or `other` (virtual interfaces are filtered out).
- `features[].state` is `not implemented`, `applied`, `skipped`, or `failed`. `features[].note` is human text — the reason or a phase pointer. Full feature list in [Phases](#phases-at-a-glance).

### `proteus current --json`

```json
[
  { "iface": "eth0",  "mac": "11:22:33:44:55:66", "type": "ethernet" },
  { "iface": "wlan0", "mac": "aa:bb:cc:dd:ee:ff", "type": "wifi"     }
]
```

JSON array, sorted by interface name. `mac` is `null` when unreadable. `type` is `wifi`, `ethernet`, or `other`. With `--iface <NAME>`, the array contains zero or one entries. Hostname/DUID fields land alongside the phase D hostname feature; the array shape is stable.

### `proteus original --json`

When the cache exists:

```json
{
  "original_macs": { "wlan0": "11:22:33:44:55:66", "eth0": "aa:bb:cc:11:22:33" },
  "original_hostname": "fedora-laptop",
  "captured_by_version": "0.1.0",
  "captured_at": "2026-05-06T12:34:56Z"
}
```

When no cache exists, the same shape with empty/null fields plus a `note`:

```json
{
  "original_macs": {},
  "original_hostname": null,
  "captured_by_version": null,
  "captured_at": null,
  "note": "no original cache yet"
}
```

Detect "no cache" via `captured_at == null` or via `note` presence.

### `proteus show-config --json` and `show-defaults --json`

The full `Config` struct serialized as JSON. Schema and every default cross-referenced in `proteus wiki config` (lands phase F alongside this page). Phase A shape:

```json
{
  "mac":       { "enabled": false, "rotation_interval": "2h",
                 "oui_pool": ["apple", "intel", "samsung", "dell", "random-locally-administered"] },
  "hostname":  { "enabled": false, "mode": "wordlist", "pinned_value": null },
  "dns":       { "strip_edns_client_subnet": true },
  "discovery": { "mdns_silence": false, "llmnr_silence": false,
                 "ssdp_block": false, "wsd_block": false },
  "probes":    { "quorum_n": 3, "quorum_total": 4, "interval": "5m", "cooldown": "60s" }
}
```

When `/etc/proteus/config.toml` is missing, `show-config --json` emits a different shape:

```json
{
  "config_present": false,
  "path": "/etc/proteus/config.toml",
  "note": "no config file; defaults are in effect — see `proteus show-defaults`"
}
```

Branch on `config_present` (absent → present config; `false` → defaults in effect).

## Idempotency

- `proteus apply` is idempotent. Ten runs converge to one run's state.
- `proteus revert` is an invariant — must work at every commit from phase B onward.
- `apply` / `revert` / `apply` / `revert` is a no-op cycle by design.

## Logging

- Output goes to **journald** via `tracing-journald` when running under systemd (detected by `JOURNAL_STREAM` being set).
- **Stderr fallback** otherwise (interactive shells, foreground runs).
- `RUST_LOG` overrides `-v` / `-q`. Standard `tracing` filter syntax: `RUST_LOG=debug`, `RUST_LOG=proteus=trace`.
- ANSI colors on stderr honor `--no-color` and `NO_COLOR`.
- Inspect timer-driven runs:
  ```
  journalctl -t proteus -n 100
  journalctl -u proteus-rotate.timer
  ```

## Wrapping Proteus

Notes for GUI / automation wrappers. The CLI is designed to be wrappable; the JSON contract above is for you.

- **Use `--json` on every read command.** Never scrape human output — column widths and wording will change.
- **Exit codes are your status signal.** `0` is success, anything else is failure. Don't parse stdout to confirm. Codes are stable.
- **Stderr is for humans.** Log lines, error context, "see proteus wiki X" pointers go to stderr. Don't confuse it with structured output.
- **Mutating commands need `--yes` for non-interactive runs.** `apply`, `revert`, `rotate`, `reset`, `uninstall` all accept it.
- **Override paths for testing.** `--config` and `--state` let you run an isolated Proteus against fixtures without touching `/etc/` or `/var/lib/`.
- **Tolerate unknown fields.** Future phases add fields; existing fields keep their names and types. Parse defensively.
- **`proteus wiki <page>` emits raw Markdown** to stdout. Render it; don't shell-quote it.
- **Long-running commands log progress** to journald or stderr — capture one or the other.

## Phases at a glance

| Phase | Brings | Subcommands wired |
|-------|--------|-------------------|
| A | skeleton, read surface, embedded wiki | `status`, `current`, `original`, `show-config`, `show-defaults`, `wiki`, `help` |
| B | L2 identity (MAC, Bluetooth alias) | `rotate`, `pin`, `unpin` |
| C | probes, timers, captive portals | (extends `apply` / `status`) |
| D | DHCP, IPv6, hostname, 802.1X, DNS knob | first wiring of `apply` |
| E | discovery silencing, stack fingerprint, RF | (extends `apply` / `status`) |
| F | cross-cutting wiki (this page), search, packaging | (no new subcommands) |
| G | diff, dry-run, reset, uninstall, full revert | `diff`, `dry-run`, `reset`, `revert`, `uninstall` |

Stub commands print one line to stderr and exit `64`:

```
$ sudo proteus rotate --iface wlan0
proteus: 'rotate' is not yet implemented; targets phase B. See: proteus wiki mac-recipes
$ echo $?
64
```

## Cross-refs

- `proteus wiki internals` — `state.json` schema, JSON output schemas in detail (phase F).
- `proteus wiki config` — full config schema, every flag with default and risks (phase F).
- `proteus wiki troubleshooting` — common errors, what to check, where the logs live (phase F).
- `proteus wiki concepts` — mental model: identifiers, rotation, captive portals, managed files, revert.
- `proteus wiki quickstart` — install, first run, basic recipes.
