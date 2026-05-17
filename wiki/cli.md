Full reference for the `proteus` command: every subcommand, every flag, every documented exit code, every JSON output schema. This page is the contract a wrapper author (GUI builder, automation script) reads.

## Synopsis

```sh
proteus [-v|--verbose ...] [-q|--quiet ...] [--config <path>] [--state <path>] [--no-color] <SUBCOMMAND>
```

## Global flags

These apply to every subcommand. They must precede the subcommand name.

- `-v`, `--verbose` — increase log verbosity. Repeatable: `-v` debug, `-vv` trace.
- `-q`, `--quiet` — decrease log verbosity. Repeatable: `-q` warn, `-qq` error.
- `--config <PATH>` — override `/etc/proteus/config.toml`. Useful for testing. Errors with exit `65` if `<PATH>` does not exist (except for commands that exist to create the file: `reset`, `config edit`, `config set`, `config set-profile`, `config enable`, `config disable`, `config reset`).
- `--state <PATH>` — override `/var/lib/proteus/state.json`. Useful for testing.
- `--no-color` — disable ANSI colors on stderr log output. Honors `NO_COLOR` too.
- `-h`, `--help` — print help and exit 0.
- `-V`, `--version` — print version and exit 0.

`RUST_LOG` overrides `-v` / `-q` when set; see the Logging section below.

## Prefix matching

`clap` accepts shortest-unambiguous subcommand prefixes by default — `proteus per` resolves to `persona`, `proteus pi` resolves to `pin`. This is a convenience, not a stability contract. A future subcommand that shares a prefix with an existing one will silently change what the abbreviation resolves to (or make it ambiguous). Scripts should spell out full subcommand names — `proteus pin`, not `proteus pi`.

## Subcommands

Alphabetical. Mutating commands require root and accept `--yes` for non-interactive runs. Read commands degrade quietly when files aren't readable. The full list also includes short aliases: `s` → `status`, `r` → `rotate`, `a` → `apply`. Most read commands accept `--watch [--interval <DUR>]`.

### `apply`

```sh
proteus apply [--yes]
```

Apply the current config to the system: rotate MACs that need rotating, write managed files under `/etc/`, install systemd timers. Idempotent — running ten times converges to the same state as once. Mutating; requires root.

Flags: `--yes` proceed without confirmation.
Exit: `0` success · `1` generic · `65` config error / missing `--yes` · `66` not root · `70` system unsupported · `75` state lock busy (another proteus instance running).
Example: `sudo proteus apply --yes`

### `config`

```sh
proteus config <SUBCOMMAND> [args...]
```

Manage `/etc/proteus/config.toml` from the CLI instead of hand-editing. Read sub-subcommands (`show`, `get`, `keys`, `validate`) work as any user. Mutating sub-subcommands (`set`, `enable`, `disable`, `edit`, `reset`) require root and explicit `--yes`. Round-trips through `toml_edit` so user comments survive.

Sub-subcommands:

- `show [--json] [--annotate|--origin]` — print active config; alias for `proteus show-config`. With `--annotate` (alias `--origin`), each section is suffixed with `# <source>` showing its provenance (`file` if the user wrote it, `profile:<name>` if it falls back to the active profile baseline, `per-ssid:<ssid>` for per-SSID overrides, or `default` for the built-in default when no profile is set). Granularity is section-level: every key inside a section shares its header's label. Field-level provenance is a follow-up (#404). With `--json`, the resolved config is paired with a `_origins` map keyed by section name. Exit `0` success · `65` parse failure · `66` permission denied.
- `get <key> [--json]` — print a single dotted key, e.g. `mac.enabled`. Falls back to the built-in default when the user config doesn't set the key. Exit `0` success · `65` unknown key.
- `set <key> <value> --yes` — coerce `<value>` to the existing key's type (bool/int/string/array) and write atomically. Exit `0` success · `65` unknown key or invalid value · `66` not root.
- `set-profile <name> --yes` — write `profile = "<name>"` at the top of `/etc/proteus/config.toml`. Per-knob overrides already in the file are preserved (the override-only-if-present model); switching to `"off"` keeps overrides on disk and resolution simply ignores them until the profile changes back. `<name>` must be one of `off`, `min`, `low`, `med`, `high`, `agr`. Exit `0` success · `65` unknown profile or missing `--yes` · `66` not root.
- `enable <component> --yes` — set `<component>.enabled = true`. Exit `0` · `65` component has no `enabled` toggle · `66` not root.
- `disable <component> [--reason <text>] --yes` — set `<component>.enabled = false`. With `--reason`, writes a `# Proteus: disabled at <iso8601> - reason: <text>` comment above the section so `proteus status` can surface it. Exit `0` · `65` · `66`.
- `edit` — spawn `$VISUAL` or `$EDITOR` (default `vi`) on `/etc/proteus/config.toml`; validate on save and report errors without rolling back. Exit `0` valid · `65` invalid (file saved as-is) · `66` not root.
- `validate [--json]` — parse the current file; report success or errors with line + column context. Empty/missing config is treated as "defaults in effect" and exits `0`. Exit `0` ok · `65` errors.
- `reset [<section>] --yes` — reset a single section (or the entire file) to built-in defaults. Other sections are preserved when a section name is given. Exit `0` · `65` unknown section · `66` not root.
- `keys [--json]` — list every supported key with type and default; the introspectable schema. Exit `0`.

Examples:

```sh
proteus config show --json
proteus config show --annotate           # mark each section with its source
proteus config show --origin --json      # parallel _origins map for jq
proteus config get mac.rotation_interval
proteus config keys | head -10
sudo proteus config disable dns --reason "using dnscrypt-proxy" --yes
sudo proteus config enable bluetooth --yes
sudo proteus config set mac.rotation_interval 1h --yes
sudo proteus config set-profile high --yes
sudo proteus config reset dns --yes
sudo proteus config edit
```

Cross-ref `proteus wiki config` for the full schema.

### `current`

```sh
proteus current [--json] [--iface <NAME>]
```

Show the current network identifiers your system is handing out right now. Read-only. Supports `--watch [--interval <DUR>]`.

Flags: `--json` machine-readable (see the `current --json` schema below) · `--iface <NAME>` limit to one interface · `--watch` re-render on a fixed interval · `--interval <DUR>` cadence for `--watch` (default 1s; minimum 100ms).
Exit: `0` success · `1` generic.
Example: `proteus current --json | jq '.[] | select(.type == "wifi")'`

### `diff`

```sh
proteus diff [--json]
```

Show the delta between current config, built-in defaults, and live system state. Flags drift from managed files (the SHA in the `# managed by proteus` header is compared to the file's actual SHA). The SHA is an edit-detection signal, not an integrity guarantee against an attacker with write access. Read-only.

Flags: `--json` machine-readable diff.
Exit: `0` no drift · `1` drift detected.

### `doctor`

```sh
proteus doctor [--json] [--quick]
```

Self-diagnostic. Runs a battery of read-only checks across system / daemons / files / detect-and-defer / runtime / Proteus state and prints `ok / warn / fail / skip` per check with remediation pointers. The first thing to run when something looks wrong. Works without root — checks needing root degrade to `skip` rather than `fail`.

Flags: `--json` machine-readable (see the `doctor --json` schema below) · `--quick` skip slower checks (filesystem walks, DBus probes). Use the global `-v` for extra detail per check (check id beneath each line).
Exit: `0` no failures (warns and skips are fine) · `1` at least one `fail`.
Example: `proteus doctor` then `proteus doctor --json | jq '.checks[] | select(.status=="fail")'`

### `dry-run`

```sh
proteus dry-run <SUBCOMMAND> [args...]
```

Preview the mutations a command would make without performing them. Every mutator goes through a `Plan` enum that can be either previewed or executed. Read-only.

Exit: `0` plan empty · `1` plan would have failed.
Example: `sudo proteus dry-run rotate --iface wlan0`

### `help`

```sh
proteus help [<feature>]
```

Friendly entry point. With no argument, lists embedded wiki pages. With a feature name, prints the matching wiki page. Effectively an alias for `wiki <feature>` with a friendlier zero-arg behavior. Read-only.

Note: `proteus help` is **not** the same as `proteus --help`. `proteus --help` is the clap usage; `proteus help` opens the wiki.

Exit: `0` success or no-arg listing · `1` no such page.

### `logs`

```sh
proteus logs [-f|--follow] [-n|--lines N] [--since <TIME>] [--json]
```

Tail journald across every Proteus systemd unit (`proteus-boot.service`, `proteus-check.{service,timer}`, `proteus-events.service`, `proteus-resume.service`, `proteus-rotate.{service,timer}`) plus the NetworkManager dispatcher syslog tag (`proteus-dispatcher`). Composes a single `journalctl -u <unit> ... -t proteus-dispatcher` invocation so operators don't have to remember the unit list. Read-only.

Flags: `-f`/`--follow` tail-follow rather than exit after the initial batch · `-n`/`--lines N` line count (default 50; bounded 1..=100000) · `--since <TIME>` passthrough to journalctl's `--since` (e.g. `1h ago`, `09:00`, `2025-05-17`) · `--json` emit structured journal entries (`journalctl --output=json`).
Exit: `0` success · `1` generic · `70` no systemd or `journalctl` missing.
Example: `proteus logs -f --since '1h ago'`

### `original`

```sh
proteus original [--json]
```

Show the cached original MACs and hostname Proteus snapshotted on first run. Sacred — never re-captured. State path defaults to `/var/lib/proteus/state.json`; override with `--state`. Read-only.

Flags: `--json` machine-readable (see the `original --json` schema below).
Exit: `0` success (whether or not a cache exists) · `1` generic read failure.

The cache populates the first time a mutating command runs (e.g. `proteus rotate` or `proteus apply`).

### `pin`

```sh
proteus pin <TARGET>
```

Pin a MAC to a specific interface or NetworkManager connection. Pinned targets are skipped by both scheduled and probe-driven rotation. For environments that lock you to one MAC: corporate networks, hotel Wi-Fi after auth, MAC-bound DHCP reservations. `<TARGET>` is interface name or NM connection profile (profile preferred when ambiguous). Mutating; requires root and `--yes`.

Flags: `--mac <MAC>` pin to an explicit MAC instead of the current one · `--yes` proceed without confirmation.
Exit: `0` success · `1` generic · `65` missing `--yes` · `66` not root · `75` state lock busy.
Example: `sudo proteus pin "Home Wi-Fi" --yes`

### `probe`

```sh
proteus probe [--json] [--quick]
```

Run one probe round against the configured endpoints and print the result. Reads `[probes]` from `/etc/proteus/config.toml`; defaults to four public IPs at port 443. Read-only; no root required. ICMP fallback is documented in `proteus wiki probes` but not implemented yet — a TCP-only failure stays `inconclusive` rather than escalating.

Flags: `--json` machine-readable (see the `probe --json` schema below) · `--quick` single-endpoint fast check (uses the first configured endpoint and a 1-of-1 quorum).
Exit: `0` clear · `1` down · `2` inconclusive · `3` portal-suspected.
Example: `proteus probe --json | jq .classification`

### `reset`

```sh
proteus reset [--yes] [--dry-run]
```

Rewrite `/etc/proteus/config.toml` to a minimal `profile = "<name>"` file, preserving the active profile from the existing config. The "I tinkered and broke it" hatch. Resolution at load time fills in every other knob from the profile baseline, so the on-disk file stays human-readable instead of bloating with every default explicitly set. Deliberately does **not** touch the cached original MACs in `state.json` — those remain sacred. Mutating; requires root. Pass `--dry-run` to preview the action without writing.

Exit: `0` success · `65` missing `--yes` · `66` not root.

### `revert`

```sh
proteus revert [--yes]
```

Restore everything to the cached originals. The panic button. **Invariant**: must work at every commit — if a feature can't be backed out cleanly, it does not ship. Mutating; requires root.

Exit: `0` success · `1` generic · `65` missing `--yes` · `66` not root · `75` state lock busy.
Example: `sudo proteus revert --yes`

### `rotate`

```sh
proteus rotate [--iface <NAME>] [--yes]
```

Generate a fresh MAC and apply it. With no `--iface`, rotates every managed interface. Skips pinned interfaces. Avoids picking a MAC that matches the gateway or anything else in the local ARP table. Mutating; requires root.

Flags: `--iface <NAME>` limit to one interface · `--yes` proceed without confirmation · `--explain` print every candidate the generator considered with rejection reasons.
Exit: `0` success · `1` generic · `65` missing `--yes` · `66` not root · `75` state lock busy.
Example: `sudo proteus rotate --iface wlan0 --yes`

### `show-config`

```sh
proteus show-config [--json]
```

Print the active config from `/etc/proteus/config.toml` (override with `--config`). When the file is missing, prints a note that defaults are in effect and exits 0. Read-only.

Flags: `--json` emit JSON instead of TOML (see the `show-config --json` schema below).
Exit: `0` success (including missing file) · `65` parse failure · `66` permission denied · `1` other read failure.

### `show-defaults`

```sh
proteus show-defaults [--json]
```

Print the built-in default config. Use this to see every knob and its default before writing your own `config.toml`. Read-only; never touches the filesystem.

Flags: `--json` emit JSON instead of TOML.
Exit: `0` success.

### `status`

```sh
proteus status [--json]
```

The overall system + per-feature status report: whether systemd, NetworkManager, BlueZ, and systemd-resolved are present; physical interfaces with their current MAC; per-feature `applied / skipped (reason) / failed (reason)`. Read-only. Aliased as `proteus s`. Supports `--watch [--interval <DUR>]`.

Flags: `--json` machine-readable (see the `status --json` schema below) · `--watch` re-render on a fixed interval · `--interval <DUR>` cadence for `--watch` (default 1s; minimum 100ms).
Exit: `0` success.

### `timer`

```sh
proteus timer <SUBCOMMAND> [args...]
```

First-class CLI surface for managing the systemd timers Proteus owns. Read sub-subcommands (`status`, `list`, `logs`) work for any user; mutating ones (`enable`, `disable`, `set`, `reset`) require root and exit `66` otherwise. Every sub-subcommand exits `70` if systemd isn't detected.

Timer names are short identifiers, not full unit names: `rotate` -> `proteus-rotate.timer`, `check` -> `proteus-check.timer`, `resume` -> `proteus-resume.timer`, `boot` -> `proteus-boot.service`.

#### `timer status`

```sh
proteus timer status [--json]
```

List every Proteus timer with its enabled/active state, current cadence, next fire, last fire, and whether a user override drop-in is active. Read-only.

Flags: `--json` machine-readable.
Exit: `0` success · `70` no systemd.

#### `timer list`

```sh
proteus timer list [--json]
```

List the timer "types" defined by Proteus along with their default cadence and one-line description. Read-only; does not consult systemd at all.

Flags: `--json` machine-readable.
Exit: `0` success.

#### `timer enable <NAME>`

```sh
proteus timer enable <NAME>
```

Run `systemctl enable --now <unit>` for the named timer (or just `enable` for the boot oneshot). Mutating; requires root.

Exit: `0` success · `1` generic · `66` not root · `70` no systemd.
Example: `sudo proteus timer enable resume`

#### `timer disable <NAME>`

```sh
proteus timer disable <NAME>
```

Run `systemctl disable --now <unit>` for the named timer. Mutating; requires root.

Exit: `0` success · `1` generic · `66` not root · `70` no systemd.
Example: `sudo proteus timer disable check`

#### `timer set <NAME> --interval <DURATION>`

```sh
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

```sh
proteus timer reset <NAME>
```

Remove the drop-in for a timer and reset to the unit-file default. Runs `daemon-reload` + `restart`. Mutating; requires root.

Exit: `0` success · `1` generic · `65` config (non-timer unit) · `66` not root · `70` no systemd.

#### `timer logs <NAME> [--lines N]`

```sh
proteus timer logs <NAME> [--lines N]
```

Tail recent journald logs for a timer's unit (`journalctl -u <unit> -n N --no-pager`). Read-only.

Flags: `--lines N` (default 50).
Exit: `0` success · `1` generic · `70` no systemd.

### `uninstall`

```sh
proteus uninstall [--purge] [--yes]
```

Full removal. Runs `revert` first, removes the binary, removes the systemd timers. Mutating; requires root.

Flags: `--purge` also delete `/etc/proteus/` and `/var/lib/proteus/`. Without `--purge`, the original-MAC cache is preserved so a reinstall can restore the same identity. `--yes` proceed without confirmation.
Exit: `0` success · `65` missing `--yes` · `66` not root.

### `unpin`

```sh
proteus unpin <TARGET>
```

Remove a pin previously set with `pin`. The target rejoins the rotation pool. `<TARGET>` is interface name or NM connection profile. Mutating; requires root.

Exit: `0` success · `1` no such pin · `66` not root · `75` state lock busy.

### `wiki`

```sh
proteus wiki [<page>]
proteus wiki search <query>... [--json] [--limit <N>]
```

Browse the embedded wiki. With no argument, lists available pages. With a page name, prints that page's Markdown to stdout. Read-only.

`proteus wiki search <query>...` runs a full-text search across every embedded wiki page. The query is whitespace-tokenized and matched case-insensitively. Results are ranked by `matched_terms × log2(occurrences + 1)` and capped at the top 10 by default (`--limit` overrides). Each row shows `<page>:<line_no>  <snippet>` with ~40 chars of context on either side of the first match. Pass `--json` for a machine-readable payload (schema: `{query, count, hits: [{page, line_no, line, snippet, matched_terms, term_frequency, score}]}`).

Exit: `0` success (including no matches) · `1` no such page or empty query · `2` missing query argument.
Examples:
- `proteus wiki cli`
- `proteus wiki search captive`
- `proteus wiki search "ECS strip" --json`

## Exit codes

Stable. New codes may be added in future versions; existing codes never change meaning. Defined in `src/lib.rs` (`exit::*`).

| Code | Constant | Meaning |
|------|----------|---------|
| 0 | `SUCCESS` | success |
| 1 | `GENERIC_ERROR` | generic / unclassified error |
| 2 | _(clap)_ | invalid arguments — usage error from the parser |
| 64 | `NOT_IMPLEMENTED` | command parses but the chosen backend / driver path is a stub — stderr names what is missing |
| 65 | `CONFIG_ERROR` | config parse failure or invalid value |
| 65 | `CONFIRMATION_REQUIRED` | mutating command invoked without `--yes` (alias of `CONFIG_ERROR`) |
| 66 | `PERMISSION_ERROR` | mutating command run without root, or read denied on a privileged file |
| 70 | `SYSTEM_NOT_SUPPORTED` | not Linux, no systemd, or another precondition the binary can't satisfy |
| 75 | `LOCK_BUSY` | another `proteus` instance is holding the state lock; safe to retry |

`CONFIRMATION_REQUIRED` shares the wire value of `CONFIG_ERROR` (`65`); it's an intent alias so source code reads naturally and so wrappers grepping for `65` keep working. Wrappers that care about telling the two apart should branch on stderr rather than the numeric code.

`75` (`LOCK_BUSY`) is split out from generic error so wrappers can implement a retry-with-backoff loop. The default lock timeout budget is 5 s; raise it via `PROTEUS_LOCK_TIMEOUT_MS` (e.g. `10000`) when wrapping in a dispatcher or timer that may overlap.

Wrappers should treat `0` as success and any non-zero as failure. Inspect stderr or journald for the human reason; do **not** scrape stdout for status — `--json` is the contract.

## JSON output schemas

`--json` on a read command emits one JSON document to stdout, pretty-printed with a trailing newline. Stderr is reserved for log lines and human errors. Schemas may grow in future versions; wrappers must ignore unknown fields. Existing fields keep their names and types.

### `proteus status --json`

```json
{
  "proteus_version": "0.4.0-beta1",
  "phase": "G",
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
    { "name": "mac-rotation", "state": "applied", "note": null },
    { "name": "hostname",     "state": "skipped", "note": "disabled by user" }
  ]
}
```

- `interfaces[].mac` is `null` when the address is unreadable. `interfaces[].kind` is `wifi`, `ethernet`, or `other` (virtual interfaces are filtered out).
- `features[].state` is one of `applied`, `skipped`, `failed`, or `not implemented` (only used today when a backend selection cannot drive a feature). `features[].note` is human text — the reason for skip / fail.

### `proteus current --json`

```json
[
  { "iface": "eth0",  "mac": "11:22:33:44:55:66", "type": "ethernet" },
  { "iface": "wlan0", "mac": "aa:bb:cc:dd:ee:ff", "type": "wifi"     }
]
```

JSON array, sorted by interface name. `mac` is `null` when unreadable. `type` is `wifi`, `ethernet`, or `other`. With `--iface <NAME>`, the array contains zero or one entries.

### `proteus original --json`

When the cache exists:

```json
{
  "original_macs": { "wlan0": "11:22:33:44:55:66", "eth0": "aa:bb:cc:11:22:33" },
  "original_hostname": "fedora-laptop",
  "captured_by_version": "0.4.0-beta1",
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

### `proteus doctor --json`

```json
{
  "schema_version": 1,
  "proteus_version": "0.4.0-beta1",
  "phase": "G",
  "checks": [
    {
      "category": "system",
      "name": "linux_kernel",
      "status": "ok",
      "message": "Linux 6.x.y"
    },
    {
      "category": "daemons",
      "name": "network_manager",
      "status": "fail",
      "message": "NetworkManager not running — required for MAC rotation",
      "remediation": "systemctl start NetworkManager"
    }
  ],
  "summary": { "ok": 8, "warn": 2, "fail": 0, "skip": 4 }
}
```

- `schema_version` is the integer doctor JSON schema; `1` today. Bumped only if a backwards-incompatible change is made; new fields without one don't bump it.
- `checks[].category` is one of `system`, `daemons`, `files`, `detect_and_defer`, `runtime`, `proteus_state`.
- `checks[].status` is one of `ok`, `warn`, `fail`, `skip`. Wrappers should treat new statuses defensively.
- `checks[].remediation` is omitted when there's nothing actionable to suggest.
- `summary` aggregates the four statuses across all checks. Exit code is `1` iff `summary.fail > 0`.

### `proteus show-config --json` and `show-defaults --json`

The full `Config` struct serialized as JSON. Schema and every default cross-referenced in `proteus wiki config`. Indicative shape (the actual `Config` carries every section documented in `proteus wiki config`):

```json
{
  "mac":       { "enabled": false, "rotation_interval": "2h",
                 "oui_pool": ["apple", "intel", "samsung", "dell", "random-locally-administered"] },
  "hostname":  { "enabled": false, "mode": "wordlist", "pinned_value": null },
  "dns":       { "strip_edns_client_subnet": true },
  "discovery": { "mdns_silence": false, "llmnr_silence": false,
                 "ssdp_block": false, "wsd_block": false },
  "probes":    { "quorum_n": 3, "quorum_total": 4, "interval": "5m", "cooldown": "60s",
                 "endpoints": ["1.1.1.1:443", "8.8.8.8:443", "9.9.9.9:443", "142.250.190.78:443"] }
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

### `proteus probe --json`

```json
{
  "schema_version": 1,
  "classification": "clear",
  "endpoints": [
    { "target": "1.1.1.1:443",        "method": "tcp", "ok": true,  "duration_ms": 47,   "error": null },
    { "target": "8.8.8.8:443",        "method": "tcp", "ok": true,  "duration_ms": 53,   "error": null },
    { "target": "9.9.9.9:443",        "method": "tcp", "ok": true,  "duration_ms": 61,   "error": null },
    { "target": "142.250.190.78:443", "method": "tcp", "ok": false, "duration_ms": 3001, "error": "tcp: connection timed out" }
  ],
  "quorum_n": 3,
  "quorum_total": 4,
  "successes": 3
}
```

- `classification` is `clear`, `down`, `inconclusive`, or `portal-suspected`. The exit code matches the documented mapping (0/1/2/3).
- `endpoints[].method` is `tcp` today; `icmp` may appear once the fallback lands.
- `endpoints[].error` is `null` on success and a short reason string on failure.
- `schema_version` is the integer version of this shape; bump if a future change breaks parsers.

## Idempotency

- `proteus apply` is idempotent. Ten runs converge to one run's state.
- `proteus revert` is an invariant — must work at every commit.
- `apply` / `revert` / `apply` / `revert` is a no-op cycle by design.

## Logging

- **By default, the tracing subscriber is not installed** — every `tracing::*!` call is a no-op. This keeps the cold-start path lean for the common interactive case (errors are surfaced via stderr regardless; tracing only supplies diagnostic hints).
- The subscriber is installed when **any** of these is true:
  - `-v` / `--verbose` (any count) — promotes default level to DEBUG/TRACE.
  - `-q` / `--quiet` (any count) — demotes default level to WARN/ERROR.
  - `RUST_LOG` is set — standard `tracing` filter syntax: `RUST_LOG=debug`, `RUST_LOG=proteus=trace`, `RUST_LOG=proteus=debug,zbus=warn`.
  - `JOURNAL_STREAM` is set — running under systemd; output is routed to **journald** via `tracing-journald`.
- When the subscriber is on and `JOURNAL_STREAM` is unset, output goes to **stderr**. ANSI colors on stderr honor `--no-color` and `NO_COLOR`.
- `RUST_LOG` overrides `-v` / `-q` when set.
- Inspect timer-driven runs:
  ```sh
  journalctl -t proteus -n 100
  journalctl -u proteus-rotate.timer
  ```

## Wrapping Proteus

Notes for GUI / automation wrappers. The CLI is designed to be wrappable; the JSON contract above is for you.

- **Use `--json` on every read command.** Never scrape human output — column widths and wording will change.
- **Exit codes are your status signal.** `0` is success, anything else is failure. Don't parse stdout to confirm. Codes are stable.
- **Stderr is for humans.** Log lines, error context, "see proteus wiki X" pointers go to stderr. Don't confuse it with structured output.
- **Mutating commands need `--yes` for non-interactive runs.** `apply`, `revert`, `rotate`, `reset`, `uninstall` all accept it. The same convention applies across the per-feature mutators (`bluetooth apply`, `dhcp apply`, `dns apply`, `hostname rotate`, `ipv6 apply`, `nft apply`, `ntp apply`, `resolved apply`, `rf apply`, `stack apply`, `enterprise-wifi enable`, `portal mark/unmark`).
- **Override paths for testing.** `--config` and `--state` let you run an isolated Proteus against fixtures without touching `/etc/` or `/var/lib/`.
- **Tolerate unknown fields.** Future versions add fields; existing fields keep their names and types. Parse defensively.
- **`proteus wiki <page>` emits raw Markdown** to stdout. Render it; don't shell-quote it.
- **Long-running commands log progress** to journald or stderr — capture one or the other.
- **Lock contention is recoverable.** Wrap mutating calls in a retry loop on exit `75`; raise `PROTEUS_LOCK_TIMEOUT_MS` for environments where a dispatcher and a timer can overlap.

## Cross-refs

- `proteus wiki internals` — `state.json` schema, JSON output schemas in detail.
- `proteus wiki config` — full config schema, every flag with default and risks.
- `proteus wiki troubleshooting` — common errors, what to check, where the logs live.
- `proteus wiki concepts` — mental model: identifiers, rotation, captive portals, managed files, revert.
- `proteus wiki backend` — backend selection (`nm` / `networkd` / `raw`) and what each impl covers.
- `proteus wiki quickstart` — install, first run, basic recipes.
