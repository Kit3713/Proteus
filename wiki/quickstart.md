Install, apply, verify in five minutes. For the longer first-time tour, see `proteus wiki getting-started`. For the mental model, see `proteus wiki concepts`.

## Prerequisites

- Linux with systemd. Fedora 43+ is the primary target; other modern systemd distros are secondary.
- NetworkManager. Proteus talks to it over D-Bus and never shells out to `nmcli`.
- systemd-resolved as the active resolver for DNS-related features.
- Optional: BlueZ for the Bluetooth bits, firewalld or nftables for discovery blocks.
- Glibc or musl. The shipped binary is glibc-linked; a musl build is straightforward from source.

Other backends are available behind `[backend] driver = "networkd" | "raw"` for systems without NetworkManager; see `proteus wiki backend`.

## Install

From source:

```sh
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cargo build --release --locked
sudo ./install.sh
```

`install.sh` is POSIX-shell, copies the binary to `/usr/local/bin`, creates `/etc/proteus` and `/var/lib/proteus`, installs systemd units, and applies SELinux file contexts where `semanage` is available.

Distro packages live under `dist/` (Arch / RPM / Debian / Nix / Alpine / Void / Gentoo). Each directory has a `README.md` for that distro.

## First run: see what your system exposes

Read commands work without root when the relevant files are readable. Look before changing anything.

- `proteus doctor` — read-only health check, prints `ok / warn / fail / skip` per check.
- `proteus status` — interfaces, current MACs, chipset, what Proteus would touch, what it would skip and why.
- `proteus current` — current values of the identifiers Proteus tracks.
- `proteus original` — the values Proteus cached the first time it ran (sacred and never re-captured).
- `proteus show-defaults` — built-in defaults.
- `proteus show-config` — current config from `/etc/proteus/config.toml`.

Add `--json` to any of these for a machine-readable payload — the contract a wrapper reads.

## A first rotation

```sh
sudo proteus rotate --iface wlan0
proteus current
```

Watch the MAC change. The connection drops briefly while NetworkManager reconnects with the new MAC. DHCP renews. Expect a few seconds of dropped traffic.

Rotate every managed interface in one shot:

```sh
sudo proteus rotate --yes
```

Pinned interfaces are skipped silently. See `proteus wiki mac-recipes` for OUI pool selection, pinning, and per-connection MACs.

## Reverting

`sudo proteus revert --yes` restores everything to the original state Proteus cached on first run. This is the panic button. `revert` is an invariant — if a feature can't be backed out cleanly, it doesn't ship.

## Apply the full config

```sh
sudo proteus apply --yes
```

`apply` is idempotent — running it ten times converges to the same state as running it once. Per-feature `applied / skipped (reason) / failed (reason)` lines surface anything that didn't fully take effect.

## Configuration

- `proteus show-defaults` — print every knob's default.
- `proteus show-config` — print the current config from `/etc/proteus/config.toml`.
- `proteus config show / get / set / enable / disable / reset / edit / validate / keys` — first-class CLI for managing `/etc/proteus/config.toml`. Round-trips through `toml_edit` so user comments survive.
- The schema is documented in `proteus wiki config`.

## Presets

Annotated example configs live in `examples/` in the repo. Each is a starting point you copy into place and tweak.

- `examples/minimal.toml` — only MAC rotation; everything else stays at OS defaults.
- `examples/standard.toml` — balanced privacy + compatibility; recommended for most users.
- `examples/aggressive.toml` — stronger privacy at the cost of breaking KDE Connect, WSD printers, and possibly corporate Wi-Fi.
- `examples/captive-portal-heavy.toml` — for daily public-Wi-Fi routines (cafes, conferences, hotels, airports).
- `examples/paranoid.toml` — maximum privacy with significant breakage; read the warning header before using.
- `examples/disabled.toml` — every section off; equivalent to not running `proteus apply`.
- `examples/development.toml` — fast cycles for Proteus contributors; not a real-world preset.

```sh
sudo cp examples/standard.toml /etc/proteus/config.toml
sudo proteus apply --yes
```

See `examples/README.md` for the full index plus a "choosing a preset" decision guide.

## Captive portals

- `proteus status` shows the current portal classification: `clear`, `portal-required`, `portal-authed`, or `unknown`.
- Default policy is `rotate-before-auth`: get a fresh MAC, then complete the portal flow. After auth, periodic rotation is suppressed so you don't loop and lose the session.
- Probe failures classified as portal-caused never trigger MAC rotation.
- See `proteus wiki captive-portals`.

## Logs

All output goes to journald via `tracing-journald` when running under systemd, and falls back to stderr otherwise.

```sh
journalctl -t proteus -n 100
```

Add `--verbose` (`-v`) to any command for more detail. Read commands stay quiet by default; mutating commands log every change with the path and the SHA of the new content.

## Common first-run errors

- `permission denied` — mutating commands need root. Use `sudo`. Read commands work without root when the relevant files are readable, and degrade quietly when they aren't.
- `no NetworkManager detected` — Proteus is NM-aware. On systems running plain `dhclient` plus `wpa_supplicant`, expect feature gaps. Status will list which features are skipped and why. Switch to `[backend] driver = "raw"` if you need to run on those hosts.
- `detected dnscrypt-proxy, deferring` (or Pi-hole, AdGuard Home, custom resolv.conf) — the DNS knob refuses to apply when another DNS-privacy tool is present. This is intentional. The user's DNS setup wins, every time. See `proteus wiki dns`.
- `lock busy` (exit `75`) — another `proteus` instance is holding the state lock. Safe to retry; raise `PROTEUS_LOCK_TIMEOUT_MS` if it persists.

## Where to go next

- `proteus wiki concepts` — the mental model: what Proteus considers an identifier, where state lives, how rotation interacts with NM.
- `proteus wiki mac-recipes` — common MAC rotation patterns.
- `proteus wiki captive-portals` — captive portal handling, policies, edge cases.
- `proteus wiki dns` — the one DNS knob and its hard guard.
- `proteus wiki personas` — persona / randomizer mode field manual.
- `proteus wiki threat-model` — what Proteus doesn't do and which tool to reach for instead. Read this before trusting Proteus with anything that matters.
