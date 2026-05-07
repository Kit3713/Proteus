## Audience

This page is for developers writing wrappers, GUIs, or tooling around Proteus. End users probably don't need it.

If you are wrapping the CLI, also read `proteus wiki cli` for the command surface, exit codes, and JSON output schemas. Read `proteus wiki concepts` for the mental model that this page assumes.

## Data flow

```
+-----------------+      +----------------------+
| /etc/proteus/   | <--- | proteus apply (root) |
| config.toml     |      +----------------------+
+-----------------+              |
                                 v
                        +-----------------------+
                        | /var/lib/proteus/     |
                        | state.json            |
                        +-----------------------+
                                 ^
            +-----------------+  |
            | /sys/class/net/ |--+
            | /run/...        |
            +-----------------+

CLI subcommands:
  apply ---> read config.toml, mutate system, update state.json
  rotate --> read state.json, mutate MAC via DBus, update state.json
  status --> read /sys, read state.json, read config.toml, write JSON to stdout
  revert --> read state.json (originals), restore system, update state.json
  diff   --> read /etc managed files, compare sha256 against state.json
```

Three things to take from this picture. One: config is input only — Proteus never writes back to `/etc/proteus/config.toml`. Two: `state.json` is the single source of truth for what Proteus has done to the system. Three: read commands like `status` and `current` are pure reads — they touch sysfs and the state file but mutate nothing.

## state.json schema (`/var/lib/proteus/state.json`)

```json
{
  "schema_version": 1,
  "captured_at": "2026-05-06T12:34:56Z",
  "proteus_version": "0.1.0",
  "originals": {
    "macs": {
      "wlan0": "11:22:33:44:55:66",
      "eth0": "aa:bb:cc:11:22:33"
    },
    "hostname": {
      "kernel": "fedora-laptop",
      "pretty": "Fedora Laptop",
      "transient": null
    },
    "bluetooth_aliases": {
      "hci0": "fedora-laptop"
    }
  },
  "managed": {
    "interfaces": {
      "wlan0": {
        "current_mac": "ee:dd:cc:bb:aa:99",
        "pinned": false,
        "last_rotated": "2026-05-06T14:34:56Z",
        "rotation_count": 12
      }
    },
    "connections": {
      "MyWifiSSID": {
        "cloned_mac": "ee:dd:cc:bb:aa:99",
        "applied_settings": {
          "wifi.cloned-mac-address": "stable-ssid",
          "ipv4.dhcp-send-hostname": "no",
          "ipv4.dhcp-vendor-class-identifier": "",
          "ipv4.dhcp-client-id": "mac"
        }
      }
    },
    "drop_ins": {
      "/etc/sysctl.d/95-proteus.conf": "sha256:abc...",
      "/etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf": "sha256:def...",
      "/etc/systemd/resolved.conf.d/10-proteus-discovery.conf": "sha256:..."
    },
    "nft_rules": ["proteus-icmp-info-drop", "proteus-discovery-block"],
    "current_hostname": {
      "kernel": "linksys-x42",
      "pretty": "Linksys X42",
      "transient": null
    }
  },
  "known_portal_ssids": ["StarbucksWiFi", "AirportFreeWiFi"]
}
```

Notes:

- `originals.*` is sacred. Written once on the first `proteus apply`, never updated afterwards. `proteus reset` does not touch it. Only `proteus uninstall --purge` removes it. This is the source of truth for `proteus revert`.
- `managed.*` reflects the current state of Proteus-managed mutations. `proteus apply` recomputes and updates this; `proteus rotate` updates the relevant interface entry; `proteus revert` clears it.
- File permissions: `0600`, owned by root. Reads require root for the full file; non-root can read `schema_version` and `proteus_version` only (TBD; phase G).
- Atomic writes: Proteus writes to a tmp file in `/var/lib/proteus/` and renames atomically over the destination. A crash mid-write leaves the previous state intact.
- Schema versioning: `schema_version: 1` for the v1.0 release. Increment on breaking changes; old versions migrate forward, never backward. A wrapper should refuse to parse a schema version higher than it was built against.

The phase A struct in `src/state.rs` is a flatter provisional shape (`original_macs`, `original_hostname`, `captured_by_version`, `captured_at`). The shape above is the v1.0 target; phase B+ migrates the on-disk file under `schema_version: 1`.

## Managed-file headers

Every file Proteus writes under `/etc/` carries a 3-line header:

```
# managed by proteus v0.1.0
# do not edit; manage via /etc/proteus/config.toml or `proteus apply`
# sha256:abc123...  (sha256 of the body content; checked by `proteus diff`)
```

`proteus diff` (phase G) reads the file, recomputes the sha256 over the body (everything after the header), and flags drift. Drift means someone edited the file by hand, or another tool did. The diff output names the file and the expected vs actual hash; the operator decides whether to re-apply, accept the local change by removing the file from Proteus management, or back the whole thing out with `proteus revert`.

The same hashes are mirrored into `state.json` under `managed.drop_ins`, so a wrapper can spot drift without re-reading every managed file.

## NetworkManager connection metadata

Proteus tags NM connections it modifies with `connection.user-data`:

```
proteus.managed=true
proteus.applied-version=0.1.0
proteus.applied-at=2026-05-06T14:34:56Z
```

`nmcli -g connection.user-data connection show <name>` to inspect. A wrapper that wants to enumerate Proteus-managed connections without parsing `state.json` can grep for `proteus.managed=true` here. The authoritative list is still `state.json`; this tag exists so the connection itself is self-describing if `state.json` is lost.

## DBus interfaces used (Phase B+)

Proteus uses zbus to talk to:

- `org.freedesktop.NetworkManager` — ipv4/ipv6 settings, `cloned-mac-address`, `dhcp-*`, `802-1x.anonymous-identity`, per-connection user-data tagging.
- `org.bluez` — adapter alias, `Discoverable`, BLE Resolvable Private Address mode.
- `org.freedesktop.hostname1` — hostname (kernel), pretty hostname, transient hostname.
- NOT `org.freedesktop.timedate1`. NTP normalization is a `systemd-timesyncd` config drop-in, not a DBus call. The dbus interface only toggles NTP on/off; it does not let you point at a specific server set, which is what Proteus needs.

No `nmcli` shelling, no `bluetoothctl` shelling, no `hostnamectl` shelling. Everything goes through dbus. A wrapper that wants to mirror Proteus state can subscribe to the same dbus signals.

## sysfs and procfs paths read

Read-only, no writes:

- `/sys/class/net/*/address` — current MAC.
- `/sys/class/net/*/permaddr` — permanent (burned-in) MAC. The kernel reports this as a separate file because the address Proteus writes overrides the runtime `address` value.
- `/sys/class/net/*/device/uevent` — driver and chipset detection (`DRIVER=iwlwifi`, etc.).
- `/sys/class/net/*/wireless/` — presence of this directory marks a Wi-Fi interface.
- `/sys/class/net/*/type` — interface type (1 for ether, etc.).
- `/sys/class/bluetooth/*/` — Bluetooth adapters.
- `/proc/sys/net/ipv4/tcp_timestamps` and other sysctls — current values, for diff.
- `/proc/sys/net/ipv6/conf/*/` — per-interface IPv6 sysctls (use_tempaddr, addr_gen_mode, etc.).

Reading these does not require root. `proteus current` and `proteus status` work for any user as long as the relevant paths are readable; they degrade quietly when not.

## Files Proteus writes or touches

Exhaustive list. Anything outside this list, Proteus has not modified.

- `/var/lib/proteus/state.json` — written by every mutating command.
- `/etc/sysctl.d/95-proteus.conf` — written; phase E.
- `/etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf` — written; phase D.
- `/etc/systemd/resolved.conf.d/10-proteus-discovery.conf` — written; phase E.
- `/etc/systemd/timesyncd.conf.d/10-proteus.conf` — written; phase E.
- nftables table `inet proteus` — created; phase E. Lives in the kernel, not on disk; `nft list table inet proteus` to inspect.
- NetworkManager per-connection settings — written via DBus; phases B and D.
- Hostname — written via the `hostname1` DBus interface; phase D.
- Bluetooth adapter alias — written via the BlueZ DBus interface; phase B.

Notably absent: `/etc/ssh/`, `/etc/ssl/`, `/etc/crypto-policies/`, `/etc/machine-id`, `/etc/resolv.conf`. Proteus does not touch any of these.

## JSON output schemas (read commands)

See `proteus wiki cli` for the live JSON schemas of `status`, `current`, `original`, `show-config`, `show-defaults`, `diff`. The shapes are stable per major version; new fields may appear in minor versions and a wrapper should ignore unknown keys.

Every read command supports `--json`. The non-JSON output is for humans and is not contractually stable; do not parse it.

## Versioning

- **Binary version**: `proteus --version` (semver). Available without root.
- **Config schema**: TOML with `#[serde(default)]` everywhere; new fields are additive; old config files keep working without edits. There is no `schema_version` in the config because it is not needed when every field has a default.
- **State schema**: `schema_version` field. Bump on any breaking shape change. Old versions migrate forward on first read by a newer binary; never backward. Downgrading the binary while keeping a newer state file is unsupported.
- **JSON output**: stable per-major-version. New fields may be added in minor versions. Removed or renamed fields require a major bump.

## Wrapping for a GUI

The CLI is built to be wrapped. Keep these in mind:

- Use `--json` on every read command. Parse the JSON, not the human output.
- Trap exit codes. They are stable and documented in `proteus wiki cli`.
- Don't shell-escape arguments. Subcommand args are positional or `--key value`; there are no eval-style flags.
- Run mutating commands as root. `pkexec proteus apply` is the typical desktop way; `sudo proteus apply` is the typical terminal way. Both work.
- Mutating commands accept `--yes` to skip confirmation. A GUI should always pass `--yes` and provide its own confirmation dialog.
- Watch `journalctl -t proteus -f -o json` for live status during long-running operations like `apply` and `rotate`. Each line is a structured event.
- Cross-ref `proteus wiki cli` for the wrapper-friendly section, including the full exit-code table.

A wrapper does not need to read `state.json` directly. Everything in it is exposed via `proteus original --json`, `proteus current --json`, `proteus status --json`, and `proteus diff --json`. Direct reads of `state.json` are supported for tooling that wants to avoid forking the binary, but the JSON commands are the contract.

## Cross-refs

- `proteus wiki cli` — CLI surface, JSON schemas, exit codes.
- `proteus wiki config` — config schema (which corresponds to `[mac]` / `[hostname]` / `[dns]` / `[discovery]` / `[probes]` sections).
- `proteus wiki concepts` — sacred originals, managed files, idempotency, detect-and-defer, the Platform trait.
- `proteus wiki uninstall` — what `proteus uninstall` and `proteus uninstall --purge` actually remove.
