# Roadmap

Status: every phase of the original plan has shipped. Phase A skeleton ✅, Phase B (MAC + Bluetooth) ✅, Phase C event-driven triggers + probes + captive portal ✅, Phase D hostname + DHCP + DNS + IPv6 + 802.1X ✅, Phase E sysctl + nft ✅, Phase F packaging + wiki + search ✅, Phase G revert + diff + dry-run + reset + uninstall + kill switch + integration tests ✅.

The next big item is the v0.1.0-alpha release tag. Everything that follows is real-world testing, security review, and distro adoption.

This is the operational view: what's done, what's next, what's on the bench. For design rationale, see [PLAN.md](PLAN.md). For per-version release notes, see [CHANGELOG.md](../CHANGELOG.md). For how to help, see [CONTRIBUTING.md](../CONTRIBUTING.md). For the mental model behind the phases, run `proteus wiki concepts`.

## Recent landings

- ✅ #99 `proteus session` — current network session snapshot (one-screen status)
- ✅ #97 `proteus kill` / `proteus resume` — emergency network shutdown + restoration
- ✅ #96 `proteus enterprise-wifi` — 802.1X anonymous outer identity (opt-in)
- ✅ #91 IPv6 stable-privacy + temp + DUID rotation (sysctl + NM DBus)
- ✅ #90 `proteus dry-run <cmd>` — preview any mutation
- ✅ #89 `proteus diff` — config drift + managed-file drift detection
- ✅ #88 `proteus revert` — restore cached originals
- ✅ #73 DHCP option suppression (12/60/61/81 + DHCPv6 DUID/IAID) via NM DBus
- ✅ #72 Full-text wiki search via build-time index
- ✅ #71 DNS ECS-strip with detect-and-defer hard guard
- ✅ #70 nftables rule manager (ICMP info-drops + optional discovery blocks)
- ✅ #69 Sysctl drop-in for TCP/ICMP/NDP stack hardening
- ✅ #66 Captive portal detection + `proteus portal` family
- ✅ #93 Integration test scaffolding (privileged podman + systemd)
- ✅ #98 Reproducible build infrastructure (pinned toolchain + verification script)
- ✅ #94 cargo-bloat audit + feature trim
- ✅ Wiki: curated TOC (`_index.md`); `proteus apply` risk warnings; `cli.rs` split into modules

## Try it today

```sh
git clone https://github.com/Kit3713/Proteus.git && cd Proteus
cargo build --release && sudo ./install.sh
proteus doctor
proteus wiki getting-started
sudo proteus apply --yes
```

`proteus apply` runs every enabled component in dependency order; `proteus revert` backs out network-layer side-effects; `proteus reset` restores config defaults; `proteus uninstall` removes the lot. See `proteus wiki getting-started` for the full first-run tour.

## What a great Linux tool looks like

Proteus aspires to be a real Linux tool, not a script collection — and ships like one:

- **Distro packaging** — Arch [`dist/arch/PKGBUILD`](../dist/arch/PKGBUILD), Fedora/RHEL [`dist/rpm/`](../dist/rpm/), Debian/Ubuntu [`dist/debian/`](../dist/debian/), NixOS [`dist/nix/`](../dist/nix/)
- **Man page** — [`dist/man/proteus.1`](../dist/man/proteus.1) with full subcommand reference
- **Shell completions** — bash, zsh, fish in [`dist/completions/`](../dist/completions/)
- **Systemd integration** — timer + service units, NM dispatcher hook, sleep hook in [`dist/systemd/`](../dist/systemd/) and [`dist/networkmanager/`](../dist/networkmanager/)
- **Polkit policy** — [`dist/polkit/com.kit3713.proteus.policy`](../dist/polkit/com.kit3713.proteus.policy) so a future GUI wrapper can prompt cleanly
- **Cross-compile** — aarch64 builds and a release workflow that ships stripped binaries
- **Embedded wiki** — `proteus wiki <page>` works offline, renders ANSI on a TTY, raw on a pipe

## Path to v0.1.0-alpha release

Code work is complete. Remaining work is operational:

- ⏳ Tag `v0.1.0-alpha` — `git tag v0.1.0-alpha && git push origin v0.1.0-alpha` triggers the multi-distro pipeline. Without an actual release, the packaging infrastructure is theoretical.
- ⏳ Real-world testing on diverse Wi-Fi (coffee shops, hotels, conferences) — the unit suite is solid (251 tests) but the project hasn't been run against weird DHCP servers, captive portals with quirks, or older BlueZ versions.
- ⏳ Independent security review — threat model and DBus surface need eyes from someone like the Tor Project / Mullvad infra / EFF.
- ⏳ Distro adoption — AUR / Copr / Debian unstable submissions need a packager sponsor; the recipes in `dist/` are ready.

## Rescuable in-progress branches

Six worktrees from rate-limited agents have substantive work that didn't land. Each is an independent rescue (extract files → fresh branch from main → verify → PR):

- `phase-c/auto-triggers` — auto enable/disable/trigger catalog
- `phase-d/ip-rotation` — DHCP lease release/renew without MAC change
- `feat/cli-ergonomics` — short aliases, `--watch`, `--format`
- `phase-d/wifi-privacy` — per-scan MAC randomization at NM layer
- `docs/distro-compat` — distro compatibility doctor check
- `phase-c/event-driven-triggers` — event-driven trigger framework

## Status legend

- ✅ Landed (in `main`)
- 🚧 In progress (PR open, may need rebase)
- ⏳ Planned (next up)
- 💭 Deferred (in scope but not soon)

## Phase A — Skeleton

- ✅ Cargo project (`opt-level="z"`, lto, codegen-units=1, panic=abort, strip)
- ✅ Pinned Rust toolchain + rustfmt config
- ✅ Full clap CLI surface; unimplemented subcommands return a "not implemented in this phase" stub
- ✅ Read-only commands: `status`, `current`, `original`, `show-config`, `show-defaults`
- ✅ `proteus doctor` — self-diagnostic
- ✅ `proteus config` CLI for user-controllable settings
- ✅ Embedded wiki via `proteus wiki`, ANSI rendering on TTY
- ✅ Logging wired (tracing-journald with stderr fallback)
- ✅ GitHub Actions CI: fmt, clippy, test, build, size check

## Phase B — L2 identity (MAC + Bluetooth)

- ✅ Wi-Fi MAC rotation via NetworkManager (zbus)
- ✅ Ethernet MAC rotation
- ✅ OUI-pool randomization (Apple, Intel, Samsung, Dell, locally-administered)
- ✅ ARP-table check before assignment (avoid gateway / live neighbor collisions)
- ✅ `pin` / `unpin` per interface and per NM connection profile
- ✅ Bluetooth adapter alias + discoverable=off (BlueZ via zbus)
- ✅ BLE Resolvable Private Address mode where the controller supports it
- 💭 BR/EDR (classic) BD_ADDR rotation — chipset-specific HCI territory

## Phase C — Probes, timers, captive portals

- ✅ Two systemd timers (`proteus-rotate.timer` 2h, `proteus-check.timer` 5m) and boot oneshot units in `dist/systemd/`
- ✅ NetworkManager dispatcher hook (`dist/networkmanager/dispatcher.d/01-proteus`) — event-driven rotation on connection up
- ✅ systemd sleep hook (`proteus-resume.service`) — re-rotate on resume from suspend
- ✅ `proteus timer` CLI — user-controllable timers (status / enable / disable / set / reset / logs)
- ✅ `proteus probe` command surface — manual probe quorum check
- ✅ Captive portal detector + `proteus portal` family (status / mark / unmark / list / open)
- ✅ Portal classification (`clear` / `portal-required` / `portal-authed` / `unknown`)
- ✅ Portal policy (`rotate-before-auth` default, `preserve-mac`, `ask`)

## Phase D — DHCP, IPv6, hostname, 802.1X, DNS

- ✅ Hostname wordlist (`data/hostname-wordlist.txt`, 534 router-flavored entries)
- ✅ Hostname rotation (kernel/pretty/transient) via hostname1 dbus; generic-default option (`fedora`); optional rotate-with-MAC
- ✅ DHCP option 12/60/61/81 suppression via NM DBus, plus DHCPv6 DUID/IAID
- ✅ IPv6 stable-privacy + temporary addresses + DUID rotation (sysctl + NM settings)
- ✅ `dns.strip-edns-client-subnet` with detect-and-defer hard guard (defers to dnscrypt-proxy / Pi-hole / non-default resolvers)
- ✅ 802.1X anonymous outer identity (opt-in, default off)

## Phase E — Discovery silencing, stack fingerprint

- ✅ Sysctl drop-in for `tcp_timestamps=0` + ICMPv6/NDP fingerprint hardening
- ✅ nft rules for ICMP info-reply drops, optional SSDP/WSD blocks, optional gratuitous-ARP suppression
- ⏳ systemd-resolved drop-in: mDNS responder + resolver off, LLMNR off
- ⏳ NTP normalization via timesyncd drop-in (skipped if chrony or ntpd present)

## Phase H — RF surface (focus area)

The OS-controllable half of the RF fingerprint. The hardware-analog half (oscillator drift, DAC nonlinearity, IQ imbalance) is documented in `wiki/rf-fingerprinting.md` as out-of-scope-by-physics; this phase is everything *above* the radio that software can shape.

- ⏳ `proteus rf` subcommand family (status / apply / revert) following the established orchestrator pattern
- ⏳ `wifi.tx-power-reduce` (opt-in) — `iw dev <iface> set txpower fixed <regulatory_max - reduction_db>`
- ⏳ Per-scan MAC randomization at the NetworkManager / wpa_supplicant layer (the worktree `phase-d/wifi-privacy` is a starting point)
- ⏳ Probe-request behavior: prefer passive scanning where the regulatory domain allows; suppress unnecessary active probes; never broadcast saved-SSID list
- ⏳ Chipset + firmware inventory in `proteus status` (Wi-Fi driver + chip ID + firmware, Bluetooth chip vendor + firmware) so users can cross-reference RF-fingerprinting research for their hardware
- 💭 BR/EDR (classic) BD_ADDR rotation — chipset-specific HCI, deferred until a known-good chipset matrix exists

## Phase F — Cross-cutting wiki, search, packaging

- ✅ 38 wiki pages including the curated TOC at `_index.md`
- ✅ Full-text wiki search via build-time index (`proteus wiki search <terms>`)
- ✅ `install.sh` and `uninstall.sh` (POSIX shell)
- ✅ systemd units in `dist/systemd/`
- ✅ Man page (`dist/man/proteus.1`)
- ✅ Shell completions (bash, zsh, fish — hand-written, in `dist/completions/`)
- ✅ PolicyKit policy (`dist/polkit/com.kit3713.proteus.policy`)
- ✅ `examples/` config presets (minimal, standard, aggressive, paranoid, captive-portal-heavy, development, disabled)
- ✅ Arch PKGBUILD, RPM spec + Copr, Debian/Ubuntu, NixOS module + flake
- ✅ aarch64 cross-compile + release workflow scaffold
- ✅ Reproducible build infrastructure (pinned toolchain + verification script)
- ⏳ Audit pass: every error path points at a wiki page or `proteus help <feature>`

## Phase G — Revert, diff, dry-run, reset, uninstall, kill switch, integration tests

- ✅ `proteus reset` — restore config to defaults (sacred originals preserved)
- ✅ `proteus uninstall [--purge]` implementation
- ✅ `proteus apply` orchestrator — every enabled component in dependency order, with risk warnings
- ✅ `proteus revert` — restore cached originals (hostname, Bluetooth, IPv6 + DHCP per-connection, drop-ins, dispatcher, nft table)
- ✅ `proteus diff` — config vs defaults vs live, with managed-file SHA verification
- ✅ `proteus dry-run <cmd>` — preview any mutator
- ✅ `proteus kill` / `proteus resume` — emergency network shutdown + restoration
- ✅ Integration test scaffolding (privileged podman + systemd container)
- ⏳ Image-diff verification: clean install + uninstall returns to baseline
- ⏳ v0.1.0-alpha release tag with stripped binary + SHA256
- ⏳ Real-world testing on diverse Wi-Fi (the open frontier)

## Post-v1 / future

- 💭 Per-SSID profiles (config schema reserves the namespace)
- 💭 macOS / Windows ports (`Platform` trait reserved; backend swap, not a fork)
- 💭 BR/EDR Bluetooth BD_ADDR rotation (needs known-good chipset matrix)
- 💭 A GUI wrapper — someone else's project; the CLI surface is GUI-friendly (`--json`, `--yes`, stable exit codes, polkit policy in place)

## Things explicitly NOT on the roadmap

The mission is *local controllable* fingerprint reduction. These items live on another tool's layer (not OS-controllable from Proteus's vantage point) or are physical limits:

- TLS / browser fingerprint — use Tor Browser, librewolf, Brave's randomization
- DNS resolution policy beyond ECS strip — use dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole
- Tracker blocking — Pi-hole, NextDNS, uBlock Origin
- Traffic correlation defenses — Tor, Mullvad
- SSH client fingerprint (HASSH) — your `ssh_config` is yours
- Hardware-baked RF fingerprints (oscillator drift, DAC nonlinearity, IQ imbalance) — physically impossible without a hardware swap; see `wiki/rf-fingerprinting.md` for what *is* in scope
- Anything that weakens Fedora's `crypto-policies`, touches `/etc/ssh/ssh_config`, or rotates `/etc/machine-id`
- Telemetry, update checks, analytics — no telemetry, ever

## How to help

- **Real-world testing** — `proteus doctor` + `proteus apply` on coffee-shop / hotel / conference / airport networks; report bugs via the issue template (this is the highest-value contribution right now)
- **Independent security review** — eyes on `wiki/threat-model.md` and the DBus surface in `src/nm/`, `src/bluetooth/`, `src/commands/dhcp.rs`, `src/commands/ipv6.rs`
- **Distro packaging sponsorship** — AUR / Copr / Debian unstable submissions; the recipes in `dist/` are ready
- **Wiki** — pages are landed but always improvable; voice should match `wiki/intro.md`
- **Code** — see [CONTRIBUTING.md](../CONTRIBUTING.md); rescuable in-progress branches listed above
