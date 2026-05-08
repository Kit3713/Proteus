`proteus doctor` is a read-only self-diagnostic. When something is misbehaving — Proteus can't talk to NetworkManager, the DNS knob is silently skipping, your config doesn't parse — `doctor` is the fastest way to localize the problem.

It runs a battery of small checks and prints, per check, one of `ok`, `warn`, `fail`, or `skip`. No mutations. No DBus calls today (path-based detection only). Safe to run anytime.

## Usage

```sh
proteus doctor                # human-readable report
proteus doctor --json         # machine-readable for wrappers
proteus doctor --quick        # fast subset (skip filesystem walks)
proteus -v doctor             # extra detail per check (check id, full path)
NO_COLOR=1 proteus doctor     # ASCII-only [ok]/[warn]/[fail] markers
proteus doctor --no-color     # same, via flag
```

Exit code: `0` if no checks failed, `1` if any check is `fail`, `2` if the args themselves are invalid.

`warn` and `skip` do **not** cause a non-zero exit. A warning is a heads-up ("Proteus might skip a feature here") and a skip is informational ("nothing to check, or need root for full detail"). Only `fail` is hard breakage.

## Status meanings

| Status | Meaning | Exit impact |
|--------|---------|-------------|
| `ok` | Check passed; nothing to do | none |
| `warn` | Suspicious but not broken; a feature may skip or behave differently | none |
| `fail` | Hard breakage; Proteus can't do its job until you fix it | exits `1` |
| `skip` | Couldn't run this check (need root, missing optional file, not applicable) | none |

## What each check does

### `system::linux_kernel`
Reads `/proc/sys/kernel/ostype`. `ok` if it's `Linux`; `fail` otherwise. Proteus is Linux-only.

### `system::systemd`
Looks for `/run/systemd/system`. `ok` when present (refines to "running" if PID 1's `comm` is `systemd`), `fail` otherwise.

### `system::root`
Reads UID from `/proc/self/status`. `ok` when root, `skip` otherwise — many checks degrade to skip when not root, this one just surfaces the fact informationally.

### `system::libc`
Probes `/lib*/ld-linux*` and `/lib/ld-musl-*` to identify glibc vs musl. `ok` either way; `skip` if it can't tell. Informational.

### `system::distro`
Reads `ID` and `VERSION_ID` from `/etc/os-release`. `ok` for Fedora 43+. `warn` for older Fedora or other distros — Proteus targets Fedora 43+; other systemd distros are secondary.

### `daemons::network_manager`
Checks `/run/NetworkManager` (with `/var/run/NetworkManager` fallback). `ok` if running, `fail` otherwise — NetworkManager is required for MAC rotation. Remediation: `systemctl start NetworkManager`.

### `daemons::bluez`
Checks `/run/bluetooth`. `ok` if running, `skip` otherwise — Bluetooth features just don't apply when BlueZ is absent.

### `daemons::systemd_resolved`
Checks `/run/systemd/resolve`. `ok` if running, `skip` otherwise — the DNS ECS-strip knob is a no-op without resolved.

### `daemons::nftables`
Looks for the `nft` binary on `PATH`. `warn` if missing — some discovery blocks need it. With `--quick` or non-root, reports `skip` because the ruleset isn't readable. With root and not quick, reports `ok`.

### `files::config_dir`
`/etc/proteus` exists? `ok` if so, `skip` if not (first run will create it).

### `files::config_file`
`/etc/proteus/config.toml` exists and parses? `ok` on parse, `fail` on parse error (with the parse error in the message), `skip` if missing (defaults are in effect). Honors `--config <PATH>` override.

### `files::state_file`
`/var/lib/proteus/state.json` exists with mode `0600`? `ok` if so, `warn` if mode differs (with `chmod 0600 …` as remediation), `skip` if missing (first run on this system) or unreadable (re-run as root). Honors `--state <PATH>` override.

### `detect_and_defer::dns`
Detects competing DNS-privacy tools — `dnscrypt-proxy`, `AdGuardHome`, `kresd` (knot-resolver), `pihole-FTL` (Pi-hole), non-Proteus drop-ins under `/etc/systemd/resolved.conf.d/`, and a non-systemd `/etc/resolv.conf`. If any is detected, returns `warn` because Proteus's DNS knob will skip — that's by design, not a bug. Wiki: `proteus wiki dns`.

### `detect_and_defer::ntp`
Detects `chronyd` or `ntpd` on `PATH`. `warn` if either is present — Proteus's NTP normalization defers to whatever NTP client you've chosen. `ok` otherwise.

### `detect_and_defer::iface_manager`
If NetworkManager isn't running but `iwd` or `wpa_supplicant` is on `PATH`, returns `warn` — Proteus needs NM. Otherwise `ok`.

### `runtime::version`
Surfaces the binary version and current phase. Always `ok`.

### `proteus_state::original_macs`
Counts entries in the `original_macs` cache. `ok` with count if non-empty; `skip` otherwise. The cache populates when you first run a mutating command like `rotate`.

### `proteus_state::pinned_interfaces`
Lists pinned interfaces (`name=mac`). `ok` if any, `skip` if none.

### `proteus_state::last_rotation`
Reports last-rotated timestamp per managed interface. `ok` if any rotations are recorded, `skip` if none.

## How to read the output

The default human format groups checks by category and prefixes each with a glyph:

```text
proteus doctor — system health check (v0.4.0-beta1, phase G)

System
  ✓ Linux 6.x.y
  ✓ systemd running
  - running as uid 1000 — some checks need root for full detail
  ✓ glibc-based
  ✓ fedora 43

Daemons
  ✓ NetworkManager running
  - BlueZ not running — Bluetooth features will skip
  ✓ systemd-resolved running
  - nft binary present, ruleset hidden (need root to inspect)
...
Summary: 9 ok, 1 warn, 0 fail, 9 skip
```

Glyphs: `✓` = `ok`, `⚠` = `warn`, `✗` = `fail`, `-` = `skip`.

Under `NO_COLOR=1` or `--no-color`, the glyphs become `[ok]`, `[warn]`, `[fail]`, `[skip]` — safe to grep.

When a check has a remediation pointer, it follows on an indented line:

```text
  ✗ NetworkManager not running — required for MAC rotation
      see: systemctl start NetworkManager
```

With `-v` you also get the check id beneath each line, e.g. `(daemons::network_manager)` — useful when filing bugs.

## JSON for wrappers

```json
{
  "schema_version": 1,
  "proteus_version": "0.4.0-beta1",
  "phase": "G",
  "checks": [
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

`schema_version` bumps only on backwards-incompatible changes. New fields don't bump it. `remediation` is omitted when there's nothing to suggest. Wrappers should ignore unknown `category` and `status` values defensively.

## When to run it

- Before filing a bug — paste the output.
- When a feature in `proteus status` says `failed` or `skipped` and you don't know why.
- After installing — sanity-check the system meets requirements.
- In a CI / smoke test for environments that wrap Proteus, e.g. `proteus doctor --json | jq '.summary.fail == 0'`.

## Cross-refs

- `proteus wiki troubleshooting` — symptom-based recovery recipes.
- `proteus wiki dns` — what the DNS detect-and-defer guard does.
- `proteus wiki cli` — full CLI reference and JSON schemas.
- `proteus wiki concepts` — the platform abstraction and supported environments.
