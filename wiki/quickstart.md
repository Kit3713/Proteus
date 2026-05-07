## Prerequisites

- Linux with systemd. Fedora 43+ is the primary target; other modern systemd distros are secondary.
- NetworkManager. Proteus talks to it over dbus and never shells out to `nmcli`.
- systemd-resolved as the active resolver for DNS-related features.
- Optional: BlueZ for the Bluetooth bits, firewalld or nftables for discovery blocks.
- Glibc or musl. The shipped binary is glibc-linked; a musl build is straightforward from source.

## Installing

Phases A and B have shipped; the binary is at `target/release/proteus` after `cargo build --release`. The `./install.sh` script ships today as well; the SELinux file-context drop and packaged distro releases are still planned (phase F).

For now, build from source:

```
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cargo build --release
./target/release/proteus status
```

## First run: see what your system exposes

Read commands work without root when the relevant files are readable. Start by looking, before changing anything.

- `proteus status` — interfaces, current MACs, chipset, what Proteus would touch, what it would skip and why.
- `proteus status --json` — same in machine-readable form, for the GUI a friend may build later.
- `proteus current` — current values of the identifiers Proteus tracks: MACs, hostname, DUID, once their phases land.
- `proteus original` — the values Proteus cached the first time it ran. These are sacred and never re-captured. If you ever want to fully revert, this is the source of truth.
- `proteus show-defaults` — built-in defaults.
- `proteus show-config` — current config from `/etc/proteus/config.toml`.

## What works today vs later

Phases A, B, and parts of C/D/G have shipped. The mutating commands listed below as "stub" parse and have help text, but they exit with `not implemented in this phase, see phase X` so you know exactly when to expect them.

Working today (shipped):

- Read surface: `proteus status`, `proteus current`, `proteus original`, `proteus show-config`, `proteus show-defaults`
- Wiki: `proteus wiki <page>`, `proteus help`
- MAC: `proteus apply`, `proteus rotate`, `proteus pin`, `proteus unpin`
- Bluetooth: `proteus bluetooth status / apply / revert`
- Hostname: `proteus hostname status / rotate / pin / revert`
- Probes: `proteus probe`
- Timer management: `proteus timer status / list / enable / disable / set / reset / logs`
- Config CLI: `proteus config show / get / set / enable / disable / edit / validate / reset / keys`
- Diagnostics: `proteus doctor`
- Hatches: `proteus reset`, `proteus uninstall`

Parses but exits `64` (still planned):

- `proteus revert` (planned, phase G — cross-cutting umbrella; per-component `bluetooth revert` and `hostname revert` ship today)
- `proteus diff` (planned, phase G)
- `proteus dry-run` (planned, phase G)

Nothing surprises you. If a command isn't built yet, it says so and points at the phase that brings it.

## A first rotation

Phase B has shipped. MAC rotation is one command:

```
sudo proteus rotate --iface wlan0
```

Generate a new MAC and apply it via NetworkManager. Confirm it took:

```
proteus current --json | jq .interfaces[].mac
```

Rotate every managed interface at once:

```
sudo proteus rotate --yes
```

## Reverting

`sudo proteus revert` (planned, phase G) will restore everything to the original state Proteus cached on first run. This is the panic button.

Until the cross-cutting `proteus revert` ships, per-component revert paths are available where their feature has landed: `proteus bluetooth revert`, `proteus hostname revert`. The umbrella `proteus revert` is the planned single-shot path; today the only complete undo is the manual recipe in `proteus wiki uninstall`.

`proteus revert` is an invariant. It must work at every commit from phase B onward. If a feature can't be backed out cleanly, it doesn't ship.

## Configuration

- `proteus show-defaults` — print the built-in defaults so you can see what every knob does without writing a config file.
- `proteus show-config` — print the current config from `/etc/proteus/config.toml`, plus where each value came from (default vs file).
- Configure by writing TOML to `/etc/proteus/config.toml`. The path may need `sudo` to create. Schema is documented in `proteus wiki config`, which lands in phase F.

## Presets

Annotated example configs live in [`examples/`](../examples/) in the repo. Each is a starting point you copy into place and tweak, not a one-true-config. After copying, run `proteus show-config --json` to confirm it parses, then `sudo proteus apply`.

- `examples/minimal.toml` — only MAC rotation; everything else stays at OS defaults.
- `examples/standard.toml` — balanced privacy + compatibility; recommended for most users.
- `examples/aggressive.toml` — stronger privacy at the cost of breaking KDE Connect, WSD printers, and possibly corporate Wi-Fi.
- `examples/captive-portal-heavy.toml` — for daily public-Wi-Fi routines (cafés, conferences, hotels, airports).
- `examples/paranoid.toml` — maximum privacy with significant breakage; read the warning header before using.
- `examples/disabled.toml` — every section off; equivalent to not running `proteus apply`.
- `examples/development.toml` — fast cycles for Proteus contributors; not a real-world preset.

```sh
sudo cp examples/standard.toml /etc/proteus/config.toml
sudo proteus apply
```

Substitute the preset filename you picked. See `examples/README.md` for the full index plus a "choosing a preset" decision guide.

## Captive portals

- `proteus status` shows the current portal classification: `clear`, `portal-required`, `portal-authed`, or `unknown` (planned, pending PR #66).
- Default policy is `rotate-before-auth`: get a fresh MAC, then complete the portal flow. After auth, periodic rotation is suppressed so you don't loop and lose the session.
- Probe failures classified as portal-caused never trigger MAC rotation. That's how the loop is avoided.
- Portal handling and the `proteus portal` subcommand family are pending in PR #66 (DIRTY, awaiting maintainer rebase). See `proteus wiki captive-portals`.

## Logs

All output goes to journald via `tracing-journald` when running under systemd, and falls back to stderr otherwise.

```
journalctl -t proteus -n 100
```

Add `--verbose` (`-v`) to any command for more detail. Read commands stay quiet by default; mutating commands log every change with the path and the SHA of the new content.

## Common first-run errors

- `permission denied` — mutating commands need root. Use `sudo`. Read commands work without root when the relevant files are readable, and degrade quietly when they aren't.
- `Cargo.toml not found` — you are running from the wrong directory. During pre-release / source-build, run from the repo root.
- `no NetworkManager detected` — Proteus is NM-aware. On systems running plain `dhclient` plus `wpa_supplicant`, expect feature gaps. Status will list which features are skipped and why.
- `detected dnscrypt-proxy, deferring` (or Pi-hole, AdGuard Home, custom resolv.conf) — the DNS knob refuses to apply when another DNS-privacy tool is present. This is intentional. The user's DNS setup wins, every time. See `proteus wiki dns`.

## Where to go next

- `proteus wiki concepts` — the mental model: what Proteus considers an identifier, where state lives, how rotation interacts with NM.
- `proteus wiki mac-recipes` — common MAC rotation patterns. Phase B.
- `proteus wiki captive-portals` — captive portal handling, policies, edge cases. Phase C.
- `proteus wiki dns` — the one DNS knob and its hard guard. Phase D.
- `proteus wiki threat-model` — what Proteus doesn't do and which tool to reach for instead. Phase F. Read this before trusting Proteus with anything that matters.
