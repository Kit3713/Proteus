# Proteus

A Rust CLI that reduces every identifier a Linux system locally controls when joining or transmitting on a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name, and the parts of the RF surface software can shape (TX power, probe-request behavior). Rotates MACs on a schedule and on connectivity loss. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

Named after the shapeshifter.

## Status

`v0.3.2-alpha` — **final alpha** of the v0.3 cycle. The next cycle is
**v0.4 beta**, which is a bug-and-vulnerability hunt only: no new
features land until beta closes. See `docs/BETA-INTAKE.md` for the
intake/triage process and explicit out-of-scope list.

The v0.3 roadmap closes at **83 ✅ / 1 💭** — every scoped item is
either landed or explicitly deferred with a re-open trigger (the only
deferral is the Debian unstable submission, which needs a sponsor).
Not a stable release; the CLI surface, config schema, and on-disk
formats may still change before `v1.0`.

What has shipped on `main`:

- **Phase A-G** (v0.1 cycle) — full skeleton + L2 identity + probes/timers/captive-portals + DHCP/IPv6/hostname/802.1X/DNS + discovery silencing + stack fingerprint + 38-page wiki + packaging + revert/diff/dry-run/reset/uninstall/kill-switch + podman+systemd integration tests. See `docs/ROADMAP-v0.1.md` for the archived detail.
- **v0.2.x** — multi-profile NM rotation (#122), uuid-keyed state (#124), the May 2026 security audit, and a long tail of low-severity polish.
- **v0.2.8-alpha hotfix batch** (rolled into v0.3.0-alpha) — six critical/high/medium issues from the v0.2.7-alpha review: secrets-merge across all four NM Update sites (#207), enterprise-wifi keyed by uuid (#209), factory-MAC fallback dropped (#208), release-test sysfs hermetic (#200), DNS canonicalize-failure defer (#210), `NO_COLOR` + isatty(stderr) (#201).
- **v0.3 cycle "Reach + Persona"** — closed in `v0.3.2-alpha`:
  - **Milestone 1: `NetworkBackend` abstraction.** Trait + three impls (NM full, networkd / raw probes-then-degrades), `[backend] driver` config, doctor matrix. Every `commands/*.rs` call site routed through the trait. `proteus rotate-if-needed` typed entry point replaces the dispatcher's JSON sed-grep (#206-C). `state_lock` migrated to `Mutex<Option<File>>` (#206-B).
  - **Milestone 2: Persona / Randomizer dual-mode stealth.** 25 stealth covers + 6 randomizer mirrors. Schema, loader, validator, full 11-subcommand CLI surface. Full integration with apply / rotate (MAC OUI shaping, hostname template, DHCP fingerprint write, Bluetooth alias). RFC 5227 ARP probe + IPv6 DAD with adaptive backoff. `wiki/personas.md` + threat-model addendum.
  - **Milestone 3: Per-SSID profile policies.** `[per_ssid."<ssid>"]` config, `proteus ssid {list,show,set,clear}`, four-layer resolver with source trace, v1→v2 schema migration that mirrors legacy `known_portal_ssids` into per-SSID seed entries. NM dispatcher + events daemon now consume `--ssid` end-to-end so per-SSID `pin_mac` short-circuits the rotate and `rotate_interval` lifts the cooldown floor.
  - **Milestone 4: Fingerprint hardening + RF + rotation triggers.** `proteus resolved` (mDNS+LLMNR off), `proteus ntp` (timesyncd normalization, detect-and-defer; persona-aware vendor-NTP overrides for Apple / Pixel / Galaxy / Surface ids), nftables `extra_drops` chain (3 opt-in knobs) + persona-aware `persona_drops` chain (drops UDP 5353 inbound when the persona's `mdns_advertise = false`). `proteus rf scan/chipset` + per-scan MAC randomization. `proteus dhcp renew` + `[dhcp] renew_on_apply` orchestrator wiring. Event-driven framework with four sources (NM connection-up / link-flap / regulatory-domain / portal-auth) and `proteus events run` daemon under a hardened systemd unit.
  - **Milestone 5: Distro reach.** Init-system abstraction (`Systemd`/`Openrc`/`Runit`/`Sysvinit`), ARM + i686 cross-compile matrix, packaging recipes for Alpine APKBUILD + Void template + Gentoo ebuild + AUR `-bin`/`-git` + Copr spec polish (Debian unstable submission deferred — needs sponsor). `wiki/distro-support.md` + `wiki/backend.md`. `proteus doctor` reports init system, libc, distro, backend, package format, quirky-setup warnings, and a `next_steps` section that synthesises actionable hints from the existing checks.
  - **Milestone 6: Ergonomics + security + bug-fix queue.** Short aliases (`proteus s/r/a`), `--watch` mode, global `--format json|yaml|table` (zero-dep YAML emitter on top of `serde_json::Value`), `proteus completions <bash|zsh|fish>`, `LOCK_BUSY` exit code (#211), `State::schema_version` migration ladder (#204). **Bypass-hardening pass**: every privileged shellout now resolves the binary to a canonical absolute path with PATH fallback (`crate::process` module); the parser audit caught and fixed two real bugs in `per_ssid::parse_duration` (multi-byte panic, silent overflow wrap). **Wiki-hint sweep**: every operator-facing `bail!` / `anyhow!` callsite in user-actionable paths now carries a `proteus wiki <page>` hint. `wiki/troubleshooting.md` symptom matrix. `docs/security/dbus-surface.md` + `docs/security/bypass-hardening-pass.md` audit artifacts.
  - **Runtime perf**: process-level mtime-keyed Config cache (apply orchestrator's 12 reads + 12 parses → 1 + 1 + 11 stat lookups), built-in persona cache (parse-once `OnceLock`), events daemon mtime-config cache, doctor PATH cache, hostname wordlist cache.

See [CHANGELOG.md](CHANGELOG.md) for the full list and [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view.

## What's next: v0.4 beta

The cycle entering now is bug + vulnerability hunting only. **No new
features land until beta closes.** Hunt scope, intake form, severity
rubric, triage cadence, and out-of-scope list are documented in
[docs/BETA-INTAKE.md](docs/BETA-INTAKE.md). Expected exits: a clean
`v0.4-beta` tag with no open critical/high findings and a published
findings doc.

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
- `proteus diff` — drift between config, defaults, and live state (with managed-file SHA verification)
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
