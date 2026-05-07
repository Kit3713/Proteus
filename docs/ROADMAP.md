# Roadmap

Status: Phase A skeleton ✅, Phase B (MAC + Bluetooth) ✅, Phase C event-driven triggers + probe ✅ (captive portal still open as #66), Phase D hostname ✅ (DHCP/DNS/IPv6/802.1X open as #71/#73), Phase E sysctl + nft open as #69/#70, Phase F packaging ✅ (search open as #72), Phase G reset/uninstall/apply orchestrator ✅ (revert pending).

This is the operational view: what's done, what's next, what's on the bench. For design rationale, see [PLAN.md](PLAN.md). For per-version release notes, see [CHANGELOG.md](../CHANGELOG.md). For how to help, see [CONTRIBUTING.md](../CONTRIBUTING.md). For the mental model behind the phases, run `proteus wiki concepts`.

## Recent landings

- ✅ #78 `proteus apply` orchestrator — runs every enabled component in dependency order
- ✅ #76 `proteus uninstall` — full removal with `--purge` for state directories
- ✅ #67 Hostname rotation via systemd hostname1 DBus (kernel/pretty/transient)
- ✅ #63 `proteus probe` — manual probe quorum check
- ✅ #65 aarch64 cross-compile + release workflow scaffold
- ✅ #64 RPM spec + Copr config (Fedora/RHEL primary target)
- ✅ #62 Debian/Ubuntu packaging (amd64 + arm64)
- ✅ #60 `proteus reset` — restore config to defaults (sacred originals preserved)
- ✅ #59 NixOS module + flake
- ✅ #58 `proteus doctor` — self-diagnostic health check
- ✅ #57 Arch Linux PKGBUILD
- ✅ #55 `proteus config` CLI for user-controllable settings
- ✅ #53 `proteus timer` CLI — user-controllable systemd timers
- ✅ Wiki polish: getting-started, hostile-environments, ip-rotation, security-checklist
- ✅ Wiki terminal renderer — markdown → ANSI on TTY, raw on pipe, `NO_COLOR` honored

## Try it today

```sh
git clone https://github.com/Kit3713/Proteus.git && cd Proteus
cargo build --release && sudo ./install.sh
proteus doctor
proteus wiki getting-started
sudo proteus apply --yes
```

`proteus apply` runs every enabled component in dependency order; `proteus reset` restores defaults; `proteus uninstall` removes the lot. See `proteus wiki getting-started` for the full first-run tour.

## What a great Linux tool looks like

Proteus aspires to be a real Linux tool, not a script collection — and ships like one:

- **Distro packaging** — Arch [`dist/arch/PKGBUILD`](../dist/arch/PKGBUILD), Fedora/RHEL [`dist/rpm/`](../dist/rpm/), Debian/Ubuntu [`dist/debian/`](../dist/debian/), NixOS [`dist/nix/`](../dist/nix/)
- **Man page** — [`dist/man/proteus.1`](../dist/man/proteus.1) with full subcommand reference
- **Shell completions** — bash, zsh, fish in [`dist/completions/`](../dist/completions/)
- **Systemd integration** — timer + service units, NM dispatcher hook, sleep hook in [`dist/systemd/`](../dist/systemd/) and [`dist/networkmanager/`](../dist/networkmanager/)
- **Polkit policy** — [`dist/polkit/com.kit3713.proteus.policy`](../dist/polkit/com.kit3713.proteus.policy) so a future GUI wrapper can prompt cleanly
- **Cross-compile** — aarch64 builds and a release workflow that ships stripped binaries
- **Embedded wiki** — `proteus wiki <page>` works offline, renders ANSI on a TTY, raw on a pipe

## Pending PRs needing manual rebase

These all landed cleanly in isolation but conflict on `cli.rs` after the recent CLI convergence (apply/uninstall/reset/doctor/config/timer all now share the same Cli enum and arg parsing). They need a rebase against current main; the underlying code is sound.

- 🚧 #66 `phase-c/captive-portal` — captive portal detection + `proteus portal` subcommand family. Blocked on cli.rs.
- 🚧 #69 `phase-e/stack-sysctl` — sysctl drop-in for TCP/ICMP/NDP stack hardening. Blocked on cli.rs.
- 🚧 #70 `phase-e/nft-rules` — nftables rule manager (ICMP info-drop + discovery blocks). Blocked on cli.rs.
- 🚧 #71 `phase-d/dns-ecs-strip` — DNS ECS-strip with detect-and-defer hard guard. Blocked on cli.rs.
- 🚧 #72 `phase-f/wiki-search` — full-text wiki search via build-time inverted index. Blocked on cli.rs (`proteus wiki search` plumbing).
- 🚧 #73 `phase-d/dhcp-suppression` — DHCP option 12/60/61/81 + DUID/IAID suppression via NM DBus. Blocked on cli.rs.

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
- 🚧 Captive portal detector + `proteus portal` family (PR #66 — needs rebase)
- 🚧 Portal classification (`clear` / `portal-required` / `portal-authed` / `unknown`) (PR #66)
- 🚧 Portal policy (`rotate-before-auth` default, `preserve-mac`, `ask`) (PR #66)
- 🚧 Suppress periodic rotation while authed; fresh MAC per visit to known-portal SSIDs (PR #66)

## Phase D — DHCP, IPv6, hostname, 802.1X, DNS

- ✅ Hostname wordlist (`data/hostname-wordlist.txt`, 534 router-flavored entries)
- ✅ Hostname rotation (kernel/pretty/transient) via hostname1 dbus; generic-default option (`fedora`); optional rotate-with-MAC
- 🚧 DHCP option 12/60/61/81 suppression via NM (PR #73 — needs rebase)
- 🚧 DUID rotation alongside MAC (PR #73)
- 🚧 IPv6 stable-privacy + temp addresses (PR #73)
- 🚧 `dns.strip-edns-client-subnet` with detect-and-defer hard guard (PR #71 — needs rebase)
- ⏳ 802.1X anonymous outer identity (opt-in, default off)

## Phase E — Discovery silencing, stack fingerprint, RF surface

- 🚧 Sysctl drop-in for `tcp_timestamps=0` etc. + ICMPv6/NDP fingerprint hardening (PR #69 — needs rebase)
- 🚧 nft rules for ICMP info-reply drops, NetBIOS/SSDP/WSD blocks, optional gratuitous-ARP suppression (PR #70 — needs rebase)
- ⏳ systemd-resolved drop-in: mDNS responder + resolver off, LLMNR off
- ⏳ WPAD off via NM
- ⏳ NTP normalization via timesyncd drop-in (skipped if chrony or ntpd present)
- ⏳ `wifi.tx-power-reduce` (opt-in)
- ⏳ Chipset reporting in `proteus status`

## Phase F — Cross-cutting wiki, search, packaging

- ✅ All wiki pages: `intro`, `quickstart`, `concepts`, `getting-started`, `mac-recipes`, `bluetooth`, `probes`, `rotation`, `captive-portals`, `dhcp`, `ipv6`, `hostname-recipes`, `enterprise-wifi`, `dns`, `discovery`, `stack-fingerprint`, `rf-fingerprinting`, `threat-model`, `cli`, `config`, `doctor`, `troubleshooting`, `verifying`, `uninstall`, `internals`, `faq`, `glossary`, `timer`, `ip-rotation`, `hostile-environments`, `security-checklist`, `throttling-detect`
- ✅ `install.sh` and `uninstall.sh` (POSIX shell)
- ✅ systemd units in `dist/systemd/`
- ✅ Man page (`dist/man/proteus.1`)
- ✅ Shell completions (bash, zsh, fish — hand-written, in `dist/completions/`)
- ✅ PolicyKit policy (`dist/polkit/com.kit3713.proteus.policy`)
- ✅ `examples/` config presets (minimal, standard, aggressive, paranoid, captive-portal-heavy, development, disabled)
- ✅ Arch PKGBUILD, RPM spec + Copr, Debian/Ubuntu, NixOS module + flake
- ✅ aarch64 cross-compile + release workflow scaffold
- 🚧 Full-text wiki search (PR #72 — needs rebase)
- ⏳ Audit pass: every error path points at a wiki page or `proteus help <feature>`

## Phase G — Diff, dry-run, reset, uninstall, integration tests

- ✅ `proteus reset` — restore config to defaults (sacred originals preserved)
- ✅ `proteus uninstall [--purge]` implementation
- ✅ `proteus apply` orchestrator — runs every enabled component in dependency order
- ⏳ `proteus revert` — undo the last apply (cached prior state)
- ⏳ `proteus diff` (config vs defaults vs live; flag drift via SHA in managed-file headers)
- ⏳ `proteus dry-run <command>` (every mutator routed through a `Plan` enum)
- ⏳ Integration tests in privileged Podman + systemd container with stubbed NM and BlueZ
- ⏳ Image-diff verification: clean install + uninstall returns to baseline
- ⏳ CI on Fedora-latest container with size check ≤3 MB
- ⏳ v1.0.0 release tag with stripped binary + SHA256

## Post-v1 / future

- 💭 Per-SSID profiles (config schema reserves the namespace)
- 💭 macOS / Windows ports (`Platform` trait reserved; backend swap, not a fork)
- 💭 BR/EDR Bluetooth BD_ADDR rotation (needs known-good chipset matrix)
- 💭 A GUI wrapper — someone else's project; the CLI surface is GUI-friendly (`--json`, `--yes`, stable exit codes, polkit policy in place)

## Things explicitly NOT on the roadmap

- TLS / browser fingerprint — use Tor Browser, librewolf, Brave's randomization
- DNS resolution policy beyond ECS strip — use dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole
- Tracker blocking — Pi-hole, NextDNS, uBlock Origin
- Traffic correlation defenses — Tor, Mullvad
- SSH client fingerprint (HASSH) — your `ssh_config` is yours
- Anything that weakens Fedora's `crypto-policies`, touches `/etc/ssh/ssh_config`, or rotates `/etc/machine-id`
- Telemetry, update checks, analytics — no telemetry, ever

## How to help

- **Code** — see [CONTRIBUTING.md](../CONTRIBUTING.md); the open frontier is the rebase queue (#66, #69–73) and Phase G integration tests
- **Wiki** — pages are landed but always improvable; voice should match `wiki/intro.md`
- **Plan feedback** — open a discussion or issue with `[plan-feedback]` in the title
- **Testing** — `proteus doctor` on your Fedora (or other systemd) host, then report bugs via the issue template
