# Proteus

A Rust CLI that reduces every identifier a Linux system locally controls when joining or transmitting on a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name, and the parts of the RF surface software can shape (TX power, probe-request behavior). Rotates MACs on a schedule and on connectivity loss. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

Named after the shapeshifter.

## Status

`v0.4.0-beta1` — beta. Not a stable release; the CLI surface, config schema, and on-disk formats may still change before `v1.0`. v0.4 is the bug-and-vulnerability-hunt phase; no new features land in this cycle.

What has shipped on `main`:

- **v0.1 cycle (Phases A–G)** — full skeleton + L2 identity + probes/timers/captive-portals + DHCP/IPv6/hostname/802.1X/DNS + discovery silencing + stack fingerprint + 45-page embedded wiki + packaging + revert/diff/dry-run/reset/uninstall/kill-switch + podman+systemd integration tests. See [docs/ROADMAP-v0.1.md](docs/ROADMAP-v0.1.md) for the archived detail.
- **v0.2 cycle "Polish"** — multi-profile NM rotation (#122), uuid-keyed state (#124), the May 2026 security audit, and a long tail of low-severity polish.
- **v0.3 cycle "Reach + Persona"** — six milestones, all shipped:
  - **M1 `NetworkBackend` abstraction.** Trait + three impls (NM full; networkd / raw probes-then-degrades), `[backend] driver` config, doctor matrix. Every `commands/*.rs` call site routes through the trait. `proteus rotate-if-needed` typed entry point.
  - **M2 Persona / Randomizer dual-mode stealth.** 25 stealth covers + 6 randomizer mirrors. Schema, loader, validator, 11-subcommand CLI. Full integration with apply / rotate (MAC OUI shaping, hostname template, DHCP fingerprint write, Bluetooth alias). RFC 5227 ARP probe + IPv6 DAD with adaptive backoff. [wiki/personas.md](wiki/personas.md) + threat-model addendum.
  - **M3 Per-SSID profile policies.** `[per_ssid."<ssid>"]` config, `proteus ssid {list,show,set,clear}`, four-layer resolver with source trace.
  - **M4 Fingerprint hardening + RF + rotation triggers.** `proteus resolved` (mDNS+LLMNR off), `proteus ntp` (timesyncd normalization, detect-and-defer), nftables `extra_drops` chain. `proteus rf scan/chipset` + per-scan MAC randomization. `proteus dhcp renew`. Event-driven framework (`proteus events run`) under a hardened systemd unit.
  - **M5 Distro reach.** Init-system abstraction (`Systemd`/`Openrc`/`Runit`/`Sysvinit`), aarch64 + armv7 cross-compile matrix, packaging recipes for Alpine APKBUILD + Void template + Gentoo ebuild + AUR `-bin`/`-git` + Copr spec polish + Debian submission-prep.
  - **M6 Ergonomics + bug-fix queue.** Short aliases (`proteus s/r/a`), `--watch` mode, `proteus completions <bash|zsh|fish>`, `LOCK_BUSY` exit code, `State::schema_version` migration ladder, 13 bug-fix items closed. [wiki/troubleshooting.md](wiki/troubleshooting.md) symptom matrix. [docs/security/dbus-surface.md](docs/security/dbus-surface.md) audit artifact.
- **v0.4 cycle "Bug + vulnerability hunt"** — no new features. The May 2026 vulnerability hunt cluster (30+ issues) plus three critical-for-beta fixes (#276 packaging version sync, #284 `Mac::from_str` panic, #297 `timer set` newline injection) ship in `v0.4.0-beta1`.

See [CHANGELOG.md](CHANGELOG.md) for the full list and [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view.

## What it does

Commands shipping today:

- `proteus status`, `proteus current`, `proteus original`, `proteus session` — read-only views of what is applied, what is live, what the cached originals are, and a one-screen current-network snapshot
- `proteus rotate` — fresh MAC on one or every interface (NetworkManager DBus, no `nmcli` shelling)
- `proteus rotate-if-needed --cooldown <secs>` — typed-result entry the dispatcher script consumes
- `proteus pin` / `proteus unpin` — pin a MAC per interface or per NM connection profile
- `proteus persona list / show / use / random / current / clear / new / edit / validate / import / export` — device-persona management; 25 stealth covers (`iphone-15`, `pixel-8`, `macbook-pro-m3`, `samsung-tv-2024`, `nest-mini`, ...) + 6 randomizer mirrors
- `proteus ssid list / show / set / clear` — per-SSID profile policies (persona / aggressiveness / pin / rotate-interval / portal-policy overrides)
- `proteus bluetooth status / apply / revert` — generic alias, `discoverable=off`, BLE Resolvable Private Address mode where the controller supports it
- `proteus hostname rotate / pin / status / revert` — rotate kernel/pretty/transient names from the 534-entry wordlist or render a persona's `hostname_template`
- `proteus ipv6 status / apply / revert` — stable-privacy + temporary addresses + DUID rotation per NM connection
- `proteus dhcp status / apply / revert / renew` — option 12/60/61/81 + DUID/IAID suppression or persona-shaped writes; lease release+renew without changing MAC
- `proteus dns status / apply / revert` — EDNS-Client-Subnet strip drop-in for systemd-resolved with detect-and-defer hard guard
- `proteus resolved status / apply / revert` — mDNS+LLMNR off via systemd-resolved drop-in
- `proteus ntp status / apply / revert` — timesyncd NTP normalization (skips if chrony/ntpd present)
- `proteus stack status / apply / revert` — TCP/ICMP/NDP sysctl hardening drop-in
- `proteus nft status / apply / revert` — nftables table for ICMP info-drops, optional SSDP/WSD blocks, and an opt-in `extra_drops` chain (ICMP timestamp / broadcast ping / IGMP query)
- `proteus rf status / apply / revert / scan / chipset` — TX-power reduction, scan-style report, driver/chipset/firmware inventory
- `proteus enterprise-wifi status / enable / disable` — 802.1X anonymous outer identity (opt-in, default off)
- `proteus portal status / mark / unmark / list / open` — captive-portal detection and known-portal SSID list
- `proteus events run` — long-running daemon that subscribes to NM connection-up / link-flap / regulatory-domain / portal-auth events and re-applies the right policy per SSID (opt-in via `[events] enabled = true`)
- `proteus kill` / `proteus resume` — emergency network shutdown (interfaces down, radios off, BlueZ adapters powered down) and full restoration
- `proteus apply [--yes]` — orchestrator across every enabled component, prints risk warnings before applying breaking knobs
- `proteus revert [--yes]` — back out Proteus's network-layer side-effects
- `proteus diff` — drift between config, defaults, and live state (with managed-file SHA edit-detection; tamper hint, not an integrity guarantee against an attacker with write access)
- `proteus dry-run <cmd>` — preview any mutator without applying
- `proteus timer status / list / enable / disable / set / reset / logs` — manage the systemd timers without scripting
- `proteus probe` — manual probe quorum check against the configured targets
- `proteus config show / get / set / enable / disable / reset / edit / validate / keys` — edit `/etc/proteus/config.toml` without touching TOML by hand
- `proteus doctor` — read-only health check (`ok / warn / fail / skip` per check); now reports the `Backend`, `Init` system, package-format, and quirky-setup matrix
- `proteus reset` — restore config to defaults; cached originals are sacred and untouched
- `proteus uninstall [--purge]` — full removal hatch
- `proteus completions <bash|zsh|fish>` — print the bundled shell completions on stdout
- `proteus wiki [page]` — curated TOC by default, or render any embedded wiki page to the terminal (markdown to ANSI on TTY, raw on pipe, `NO_COLOR` honored)
- `proteus wiki search <query>` — full-text search across every embedded page
- Aliases: `proteus s` → `status`, `proteus r` → `rotate`, `proteus a` → `apply`. `--watch [--interval]` on `status` / `current` / `session`.

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

When a Linux system joins a coffee-shop, hotel, conference, or airport network it announces itself loudly — MAC, hostname in the DHCP request, `_workstation._tcp` mDNS broadcast, IPv6 address derived from the MAC, and a probe-request burst naming every saved SSID. Network-side analytics platforms key on those. Proteus shuts them up.

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

A PolicyKit action policy from `dist/polkit/` is also installed when `/usr/share/polkit-1/actions/` exists. This file is a **UX hint to GUI wrappers** that elevate via `pkexec` — it provides the desktop password-prompt text and the `auth_admin` defaults — and is **not a binary-side authorization gate**. The `proteus` binary never consults polkit; the only real privilege gates are `sudo` and `pkexec`. Anyone with sudo can run `sudo proteus apply` directly and bypass the policy entirely. See `dist/polkit/README.md` for the full framing.

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
