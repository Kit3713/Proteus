# Proteus

A Rust CLI that erases the network identifiers your Linux computer hands out every time it joins a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name. Rotates MACs on a schedule and on connectivity loss. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

Named after the shapeshifter.

## Status

`v0.1.0-alpha` — pre-release. Not a stable release; the CLI surface, config schema, and on-disk formats may still change before `v0.1.0`.

What has shipped on `main`:

- Phase A (skeleton) — done. Cargo project, full clap CLI surface, read-only commands, embedded wiki with terminal renderer, journald logging, stable exit codes
- Phase B (L2 identity) — done. Wi-Fi and Ethernet MAC rotation via NetworkManager DBus, OUI pool, ARP collision check, pin/unpin per interface or NM connection, Bluetooth alias + `discoverable=off` + BLE RPA via BlueZ
- Phase C (probes, timers, captive portals) — partial. NetworkManager dispatcher hook and systemd sleep hook (event-driven rotation, no daemon), `proteus probe` manual quorum check, `proteus timer` user-controllable timers. Probe-driven rotation callbacks and the captive-portal classifier are next
- Phase D (DHCP, IPv6, hostname, 802.1X, DNS) — partial. Hostname rotation via `hostname1` DBus (kernel/pretty/transient) plus a 534-entry router-flavored wordlist. DHCP, IPv6, 802.1X, and the ECS-strip DNS knob are still ahead
- Phase E (discovery silencing, stack fingerprint, RF surface) — not started
- Phase F (cross-cutting wiki, search, packaging) — packaging done. 32 wiki pages, `install.sh` and `uninstall.sh`, systemd units, man page, shell completions (bash/zsh/fish), PolicyKit policy, and distro packaging for Arch / Fedora / Debian / NixOS. Full-text wiki search and the error-path audit are still ahead
- Phase G (diff, dry-run, reset, uninstall, integration tests) — partial. `proteus reset` and `proteus uninstall` shipped; `proteus diff` and `proteus dry-run` are still ahead

See [CHANGELOG.md](CHANGELOG.md) for the full list and [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view.

## What it does

Commands shipping today:

- `proteus status`, `proteus current`, `proteus original` — read-only views of what is applied, what is live, and the cached originals
- `proteus rotate` — fresh MAC on one or every interface (NetworkManager DBus, no `nmcli` shelling)
- `proteus pin` / `proteus unpin` — pin a MAC per interface or per NM connection profile
- `proteus bluetooth apply` / `proteus bluetooth status` — generic alias, `discoverable=off`, BLE Resolvable Private Address mode where the controller supports it
- `proteus hostname rotate` / `proteus hostname pin` / `proteus hostname status` — rotate kernel/pretty/transient names from the wordlist or pin a generic
- `proteus timer status` / `enable` / `disable` / `set` / `reset` / `logs` — manage the systemd timers (`proteus-rotate.timer` and `proteus-check.timer`) without scripting
- `proteus probe` — manual probe quorum check against the configured targets
- `proteus config show` / `get` / `set` / `enable` / `disable` / `reset` / `edit` — edit `/etc/proteus/config.toml` without touching TOML by hand (round-trips through `toml_edit` so comments survive)
- `proteus apply` — orchestrator across enabled components (idempotent; modules not yet implemented surface as `not yet implemented`)
- `proteus doctor` — read-only health check (`ok / warn / fail / skip` per check, only `fail` is non-zero)
- `proteus reset` — restore config to defaults; cached originals are sacred and untouched
- `proteus uninstall [--purge]` — full removal hatch
- `proteus wiki <page>` — render any of the 32 embedded wiki pages to the terminal (markdown to ANSI on TTY, raw on pipe, `NO_COLOR` honored)

Planned, not yet shipped:

- `proteus revert` — back out Proteus changes to the cached originals (currently a stub)
- `proteus diff`, `proteus dry-run` — drift detection and mutation preview
- DHCP option 12/60/61/81 suppression, IPv6 stable-privacy + DUID rotation, 802.1X anonymous outer identity, the ECS-strip DNS knob
- Discovery silencing (mDNS / LLMNR / NetBIOS / SSDP / WSD), `tcp_timestamps=0`, ICMP rules, NDP hardening, NTP normalization, `wifi.tx-power-reduce`
- Captive portal detector and policy

Full per-feature plan in [docs/PLAN.md](docs/PLAN.md). Comparison to existing tools in [docs/PRIOR-ART.md](docs/PRIOR-ART.md).

## What it doesn't do

This is a network-layer fingerprint eraser. It is not:

- a TLS or browser fingerprint tool — use Tor Browser, librewolf, or Brave's randomization
- a DNS-privacy tool beyond the one ECS-strip knob (planned) — use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole
- a tracker blocker — use Pi-hole, NextDNS, or uBlock Origin
- a traffic correlation defense — use Tor or Mullvad VPN
- a hardening framework — Proteus refuses to weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`, or rotate `/etc/machine-id`

`proteus wiki threat-model` spells this out so you do not over-trust the tool.

## Quick start

```sh
git clone https://github.com/Kit3713/Proteus.git && cd Proteus
cargo build --release
sudo ./install.sh
proteus doctor
proteus status
sudo proteus apply --yes
```

`proteus doctor` is read-only and tells you what will work on this host before you change anything. `proteus status` shows per-feature `applied / skipped (reason) / failed (reason)`. `proteus apply` is idempotent — running it ten times converges to the same state as running it once.

For the first-time tutorial, run `proteus wiki getting-started`.

## Why use this

You join a coffee-shop, hotel, conference, or airport network and your laptop screams "I am Chris, I have been here before" at the L2 layer. The MAC, the hostname in the DHCP request, the `_workstation._tcp` mDNS announcement, the IPv6 address derived from the MAC. Network-side analytics platforms key on those. Proteus shuts them up.

Proteus is one layer in a defense-in-depth stack. It pairs naturally with:

- Tor Browser or LibreWolf for the L7 browser fingerprint
- dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole for DNS resolution policy
- Mullvad or Tor for IP-layer correlation and traffic analysis

Each layer is its own complex world and deserves its own tooling. Proteus owns the network-joining identity layer. It refuses to overstep — the detect-and-defer guards on DNS and NTP are deliberate, your tool wins. See `proteus wiki hostile-environments` for the field guide and `proteus wiki threat-model` for the boundary discussion.

## Requirements

- Linux with systemd
- NetworkManager (managed via DBus, no `nmcli` shelling)
- systemd-resolved
- BlueZ for the Bluetooth features (optional)
- firewalld or nftables for the future discovery blocks (optional)
- Glibc or musl
- Fedora 43+ is the primary target; other modern systemd distros are secondary

## Installing

### From source

```sh
git clone https://github.com/Kit3713/Proteus.git && cd Proteus
cargo build --release
sudo ./install.sh
```

`install.sh` is POSIX-shell (no bashisms). It copies the binary to `/usr/local/bin`, creates `/etc/proteus` and `/var/lib/proteus`, installs the systemd units from `dist/systemd/` if present, and applies SELinux file contexts on systems where `semanage` is available. It does not run `proteus apply` for you — applying is mutating, you should review your config first.

### Distro packages

Packaging recipes for the major distributions:

- `dist/arch/` — Arch Linux PKGBUILD
- `dist/rpm/` — Fedora / RHEL RPM spec + Copr config
- `dist/debian/` — Debian / Ubuntu deb packaging (amd64 + arm64)
- `dist/nix/` — NixOS module + flake

Each directory has a `README.md` with build instructions for that distro.

### Uninstalling

```sh
sudo proteus uninstall          # remove binary + systemd units; keep config and state
sudo proteus uninstall --purge  # also clear /etc/proteus and /var/lib/proteus
```

`./uninstall.sh` is a thin wrapper around the same code path so distro packages can reuse it.

## Documentation

Wiki entry points (every page is also accessible via `proteus wiki <page>` from the embedded copy):

- `proteus wiki getting-started` — first-time tutorial: doctor, current, first rotation, cadence, daily mental model
- `proteus wiki concepts` — mental model: identifiers, rotation, captive portals, managed files, revert
- `proteus wiki hostile-environments` — field guide for cafes, hotels, conferences, airports, hostile actors
- `proteus wiki threat-model` — what Proteus does not do and which tool to reach for instead
- `proteus wiki cli` — full command reference, exit codes, JSON schemas
- `proteus wiki troubleshooting` — symptom-based recovery recipes

Project-level docs:

- [docs/PLAN.md](docs/PLAN.md) — what is being built and in what order
- [docs/ROADMAP.md](docs/ROADMAP.md) — operational status by phase
- [docs/PRIOR-ART.md](docs/PRIOR-ART.md) — what already exists and where Proteus fits
- [CHANGELOG.md](CHANGELOG.md) — release notes per version
- [SECURITY.md](SECURITY.md) — how to report vulnerabilities
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to help

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The open frontiers right now are Phase C (probe-driven rotation callbacks, captive-portal classifier) and Phase D (DHCP suppression, IPv6 stable-privacy, the ECS-strip DNS knob). [docs/ROADMAP.md](docs/ROADMAP.md) marks every item; pick something flagged planned and open an issue first if it is non-trivial.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). If you distribute a modified version of Proteus, you must release the source under GPLv3 (or later) as well.

Contributions are accepted under the same terms.
