# Roadmap

Status: Phase A landed. Phase B (MAC + Bluetooth) landed. Phase C event-driven triggers landed; probe quorum + captive portal still pending. Phases D, E, G not started. Packaging (F) mostly landed (wiki search still pending).

This is the operational view: what's done, what's next, what's on the bench. For design rationale and per-feature reasoning, see [PLAN.md](PLAN.md). For how to help, see [CONTRIBUTING.md](../CONTRIBUTING.md). For the mental model behind the phases, run `proteus wiki concepts`.

## Recent landings

- #53 `proteus timer` CLI — user-controllable systemd timers (status / enable / disable / set / reset / logs)
- #52 NetworkManager dispatcher hook + systemd sleep service — event-driven rotation, no daemon
- #51 Bluetooth alias + discoverable=off + BLE RPA via BlueZ DBus
- #50 Wiki page `throttling-detect` (research direction, deferred-with-honesty)
- #49 PolicyKit policy for future GUI wrappers
- #48 Wiki terminal renderer — markdown → ANSI on TTY, raw on pipe, respects `NO_COLOR`
- #47 Hostname wordlist (`data/hostname-wordlist.txt`, 534 entries) — ready for Phase D
- #44 MAC rotation core (`rotate`, `pin`, `unpin`) with NM DBus, OUI pool, ARP probe

## Status legend

- ✅ Landed (in `main`)
- 🚧 In progress (PR open or branch active)
- ⏳ Planned (next up)
- 💭 Deferred (in scope but not soon)

## Phase A — Skeleton

- ✅ Cargo project (`opt-level="z"`, lto, codegen-units=1, panic=abort, strip)
- ✅ Pinned Rust toolchain + rustfmt config
- ✅ Full clap CLI surface; unimplemented subcommands return a "not implemented in this phase" stub
- ✅ Read-only commands: `status`, `current`, `original`, `show-config`, `show-defaults`
- ✅ Embedded wiki via `proteus wiki`
- ✅ Wiki terminal renderer — markdown → ANSI on TTY, raw on pipe, `NO_COLOR` honored
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
- ✅ `proteus timer` CLI — user-controllable timers (status / enable / disable / set / reset / logs), no scripting needed
- ⏳ Wire timer/boot units to actual rotation logic (units exist, callbacks land here)
- ⏳ Probe quorum (default ≥3 of 4 fail → rotate), 60s cooldown, TCP-connect with ICMP fallback
- ⏳ `proteus probe` command surface
- ⏳ Captive portal detector (`nmcheck.gnome.org` or equivalent)
- ⏳ Portal classification: `clear` / `portal-required` / `portal-authed` / `unknown`
- ⏳ Portal policy: `rotate-before-auth` (default), `preserve-mac`, `ask`
- ⏳ Suppress periodic rotation while authed; fresh MAC per visit to known-portal SSIDs
- ⏳ Browser-helper to launch portal page

## Phase D — DHCP, IPv6, hostname, 802.1X, DNS

- ✅ Hostname wordlist (`data/hostname-wordlist.txt`, 534 router-flavored entries) — data shipped, code lands here
- ⏳ DHCP option 12/60/61/81 suppression via NM
- ⏳ IPv6 stable-privacy + temp addresses
- ⏳ DUID rotation alongside MAC
- ⏳ Hostname rotation (kernel/pretty/transient) via hostname1 dbus; generic-default option (`fedora`); optional rotate-with-MAC
- ⏳ 802.1X anonymous outer identity (opt-in, default off)
- ⏳ `dns.strip-edns-client-subnet` with hard guard (defers to dnscrypt-proxy / Pi-hole / AdGuard / custom resolv.conf / non-Proteus drop-ins)

## Phase E — Discovery silencing, stack fingerprint, RF surface

- ⏳ systemd-resolved drop-in: mDNS responder + resolver off, LLMNR off
- ⏳ firewalld / nftables-direct fallback: NetBIOS blocked
- ⏳ SSDP and WSD blocks (opt-in, default off — break KDE Connect and WS-Discovery printers)
- ⏳ Sysctl drop-in for `tcp_timestamps=0` (with documented PAWS edge case)
- ⏳ nft rules for ICMP info-reply drops; optional gratuitous-ARP suppression
- ⏳ ICMPv6 / NDP fingerprint hardening
- ⏳ WPAD off via NM
- ⏳ NTP normalization via timesyncd drop-in (skipped if chrony or ntpd present)
- ⏳ `wifi.tx-power-reduce` (opt-in)
- ⏳ Chipset reporting in `proteus status`

## Phase F — Cross-cutting wiki, search, packaging

- ✅ `intro`, `quickstart`, `concepts` (phase A pages)
- ✅ `mac-recipes`, `bluetooth` (phase B pages)
- ✅ `probes`, `rotation`, `captive-portals` (phase C pages)
- ✅ `dhcp`, `ipv6`, `hostname-recipes`, `enterprise-wifi`, `dns` (phase D pages)
- ✅ `discovery`, `stack-fingerprint`, `rf-fingerprinting` (phase E pages)
- ✅ `threat-model`, `cli`, `config`, `troubleshooting`, `verifying`, `uninstall`, `internals`, `faq`, `glossary` (phase F pages)
- ✅ `throttling-detect` (deferred-with-honesty research page)
- ⏳ Full-text wiki search (build-time inverted index, target <200ms cold)
- ⏳ Audit pass: every error path points at a wiki page or `proteus help <feature>`
- ✅ `install.sh` (POSIX shell)
- ✅ `uninstall.sh` (wrapper; underlying `proteus uninstall` impl is Phase G)
- ✅ systemd units in `dist/systemd/`
- ✅ Man page (`dist/man/proteus.1`)
- ✅ Shell completions (bash, zsh, fish — hand-written, in `dist/completions/`)
- ✅ PolicyKit policy (`dist/polkit/com.kit3713.proteus.policy`) — for future GUI wrappers
- ⏳ `examples/` config presets (laptop, desktop, paranoid, minimal)

## Phase G — Diff, dry-run, reset, uninstall, integration tests

- ⏳ `proteus diff` (config vs defaults vs live; flag drift via SHA in managed-file headers)
- ⏳ `proteus dry-run <command>` (every mutator routed through a `Plan` enum)
- ⏳ `proteus reset` (defaults + re-apply; preserves cached originals + history)
- ⏳ `proteus uninstall [--purge]` implementation (the script is a wrapper)
- ⏳ Integration tests in privileged Podman + systemd container with stubbed NM and BlueZ
- ⏳ Image-diff verification: clean install + uninstall returns to baseline
- ⏳ CI on Fedora-latest container with size check ≤3 MB
- ⏳ v1.0.0 release tag with stripped binary + SHA256

## Post-v1 / future

- 💭 Per-SSID profiles (config schema reserves the namespace)
- 💭 macOS / Windows ports (`Platform` trait reserved; backend swap, not a fork)
- 💭 Full-text wiki search optimizations
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

- **Code** — see [CONTRIBUTING.md](../CONTRIBUTING.md), pick an item marked ⏳; Phase C captive-portal/probe and Phase D are the open frontiers
- **Wiki** — pages are landed but always improvable; voice should match `wiki/intro.md`
- **Plan feedback** — open a discussion or issue with `[plan-feedback]` in the title
- **Testing** — try the binary on your Fedora (or other systemd) host; report bugs via the issue template
