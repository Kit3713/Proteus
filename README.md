# Proteus

A Rust CLI that reduces every identifier your Linux computer can locally control when joining or transmitting on a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name, and the parts of the RF surface software can shape (TX power, probe-request behavior). Rotates MACs on a schedule and on connectivity loss. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

Named after the shapeshifter.

## Status

`v0.1.0-alpha` — pre-release. Not a stable release; the CLI surface, config schema, and on-disk formats may still change before `v0.1.0`.

What has shipped on `main`:

- Phase A (skeleton) — done. Cargo project, full clap CLI surface, read-only commands, embedded wiki with terminal renderer + full-text search, journald logging, stable exit codes
- Phase B (L2 identity) — done. Wi-Fi and Ethernet MAC rotation via NetworkManager DBus, OUI pool, ARP collision check, pin/unpin per interface or NM connection, Bluetooth alias + `discoverable=off` + BLE RPA via BlueZ
- Phase C (probes, timers, captive portals) — done. NetworkManager dispatcher hook and systemd sleep hook (event-driven rotation, no daemon), `proteus probe` manual quorum check, `proteus timer` user-controllable timers, captive-portal detection + `proteus portal` family with policy-aware rotation
- Phase D (DHCP, IPv6, hostname, 802.1X, DNS) — done. Hostname rotation via `hostname1` DBus with 534-entry wordlist; IPv6 stable-privacy + temporary addresses + DUID rotation; DHCP option 12/60/61/81 + DUID/IAID suppression via NM DBus; 802.1X anonymous outer identity (opt-in); ECS-strip DNS drop-in with detect-and-defer hard guard
- Phase E (discovery silencing, stack fingerprint) — done. Sysctl drop-in for TCP/ICMP/NDP hardening; nftables ruleset for ICMP info-drops + optional SSDP/WSD blocks
- Phase F (cross-cutting wiki, search, packaging) — done. 38 wiki pages with curated TOC + full-text search, `install.sh` and `uninstall.sh`, systemd units, man page, shell completions (bash/zsh/fish), PolicyKit policy, distro packaging for Arch / Fedora / Debian / NixOS, reproducible-build infrastructure
- Phase G (revert, diff, dry-run, reset, uninstall, kill switch, integration tests) — done. Every mutator has a `revert` path that restores cached originals; `proteus diff` flags drift; `proteus dry-run <cmd>` previews any mutation; `proteus kill` / `proteus resume` is the emergency hatch; podman+systemd integration test scaffold landed

See [CHANGELOG.md](CHANGELOG.md) for the full list and [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view.

## What it does

Commands shipping today:

- `proteus status`, `proteus current`, `proteus original`, `proteus session` — read-only views of what is applied, what is live, what the cached originals are, and a one-screen current-network snapshot
- `proteus rotate` — fresh MAC on one or every interface (NetworkManager DBus, no `nmcli` shelling)
- `proteus pin` / `proteus unpin` — pin a MAC per interface or per NM connection profile
- `proteus bluetooth status / apply / revert` — generic alias, `discoverable=off`, BLE Resolvable Private Address mode where the controller supports it
- `proteus hostname rotate / pin / status / revert` — rotate kernel/pretty/transient names from the 534-entry wordlist or pin a generic
- `proteus ipv6 status / apply / revert` — stable-privacy + temporary addresses + DUID rotation per NM connection
- `proteus dhcp status / apply / revert` — option 12/60/61/81 + DUID/IAID suppression on managed NM connections
- `proteus dns status / apply / revert` — EDNS-Client-Subnet strip drop-in for systemd-resolved with detect-and-defer hard guard
- `proteus stack status / apply / revert` — TCP/ICMP/NDP sysctl hardening drop-in
- `proteus nft status / apply / revert` — nftables table for ICMP info-drops and optional SSDP/WSD blocks
- `proteus enterprise-wifi status / enable / disable` — 802.1X anonymous outer identity (opt-in, default off)
- `proteus portal status / mark / unmark / list / open` — captive-portal detection and known-portal SSID list
- `proteus kill` / `proteus resume` — emergency network shutdown (interfaces down, radios off, BlueZ adapters powered down) and full restoration
- `proteus apply [--yes]` — orchestrator across every enabled component, prints risk warnings before applying breaking knobs
- `proteus revert [--yes]` — back out Proteus's network-layer side-effects (hostname, Bluetooth alias, DHCP/IPv6 NM settings, sysctl/timesyncd/resolved drop-ins, dispatcher hook, nft table)
- `proteus diff` — drift between config, defaults, and live state (with managed-file SHA verification)
- `proteus dry-run <cmd>` — preview any mutator without applying
- `proteus timer status / list / enable / disable / set / reset / logs` — manage the systemd timers without scripting
- `proteus probe` — manual probe quorum check against the configured targets
- `proteus config show / get / set / enable / disable / reset / edit / validate / keys` — edit `/etc/proteus/config.toml` without touching TOML by hand (round-trips through `toml_edit` so comments survive)
- `proteus doctor` — read-only health check (`ok / warn / fail / skip` per check, only `fail` is non-zero)
- `proteus reset` — restore config to defaults; cached originals are sacred and untouched
- `proteus uninstall [--purge]` — full removal hatch
- `proteus wiki [page]` — curated TOC by default, or render any embedded wiki page to the terminal (markdown to ANSI on TTY, raw on pipe, `NO_COLOR` honored)
- `proteus wiki search <query>` — full-text search across every embedded page

Full per-feature plan in [docs/PLAN.md](docs/PLAN.md). Comparison to existing tools in [docs/PRIOR-ART.md](docs/PRIOR-ART.md).

## What it doesn't do

The mission is **local controllable fingerprint reduction** — every identifier the OS / NetworkManager / BlueZ / kernel / supplicant can rewrite, plus the parts of the RF surface software can shape (TX power, probe behavior, scan policy). Things controlled by another tool's layer stay with that tool. So Proteus is not:

- a TLS or browser fingerprint tool — use Tor Browser, librewolf, or Brave's randomization
- a DNS-privacy tool beyond the one ECS-strip knob — use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole
- a tracker blocker — use Pi-hole, NextDNS, or uBlock Origin
- a traffic correlation defense — use Tor or Mullvad VPN
- a hardening framework — Proteus refuses to weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`, or rotate `/etc/machine-id`
- an SSH client fingerprint tool — your `ssh_config` is yours
- a fix for hardware-baked RF fingerprints (oscillator drift, DAC nonlinearity, IQ imbalance) — those need a swappable USB Wi-Fi adapter, not software

`proteus wiki threat-model` and `proteus wiki rf-fingerprinting` spell out the boundary so you do not over-trust the tool.

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

You join a coffee-shop, hotel, conference, or airport network and your laptop screams "I am Chris, I have been here before" — the MAC, the hostname in the DHCP request, the `_workstation._tcp` mDNS announcement, the IPv6 address derived from the MAC, and even the probe-request burst that names every saved SSID. Network-side analytics platforms key on those. Proteus shuts them up.

Proteus is one layer in a defense-in-depth stack. It pairs naturally with:

- Tor Browser or LibreWolf for the L7 browser fingerprint
- dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole for DNS resolution policy
- Mullvad or Tor for IP-layer correlation and traffic analysis
- A swappable USB Wi-Fi adapter when the RF threat is targeted SDR-in-the-room (Proteus reduces the OS-controllable RF surface; it cannot change your chip's analog characteristics)

Each layer is its own complex world and deserves its own tooling. Proteus owns the surface that the local OS can rewrite. It refuses to overstep — the detect-and-defer guards on DNS and NTP are deliberate, your tool wins. See `proteus wiki hostile-environments` for the field guide, `proteus wiki threat-model` for the boundary discussion, and `proteus wiki rf-fingerprinting` for the RF half.

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

Run `proteus wiki` (no args) for the curated TOC, or `proteus wiki search <term>` for full-text search across every embedded page.

Suggested entry points:

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

See [CONTRIBUTING.md](CONTRIBUTING.md). The major phases are landed; the open frontiers right now are real-world testing on diverse Wi-Fi (coffee shops, hotels, conferences with quirky DHCP servers), independent security review of the threat model + DBus surface, and distro adoption (AUR/Copr/Debian-unstable submissions need a packager sponsor). [docs/ROADMAP.md](docs/ROADMAP.md) marks every item; pick something flagged planned and open an issue first if it is non-trivial.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). If you distribute a modified version of Proteus, you must release the source under GPLv3 (or later) as well.

Contributions are accepted under the same terms.
