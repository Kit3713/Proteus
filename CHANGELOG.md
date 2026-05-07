# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

See [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view of what has
landed, what is in flight, and what is on the bench. See
[README.md](README.md) for the project overview.

## [Unreleased]

(work in progress; see [docs/ROADMAP.md](docs/ROADMAP.md))

### Added

- feat(phase-g): `proteus kill` / `proteus resume` — emergency network
  shutdown. `sudo proteus kill --yes` brings every managed interface down
  via `ip link`, disables NetworkManager Wi-Fi + WWAN radios via DBus, and
  powers off every BlueZ adapter. State is recorded under
  `state.kill_switch` so `sudo proteus resume --yes` can reverse exactly
  the set we touched. `proteus kill status [--json]` is a read-only
  reporter for wrappers. Idempotent: re-running while already active /
  inactive exits 0 with a note. New wiki page `kill-switch` documents
  when to use it (hostile networks, suspected compromise, border
  crossings), what it does, what it deliberately does not do, and the
  manual recovery path.

### Changed

- chore: reproducible build infrastructure (pinned toolchain, deterministic
  `SOURCE_DATE_EPOCH`, sha256 verification script). `rust-toolchain.toml` now
  pins to `1.93.0` instead of floating `stable`; the release workflow exports
  `SOURCE_DATE_EPOCH` from the tag commit and builds with
  `cargo build --release --frozen --locked`; each release attaches a
  `*.build-info` manifest (rustc, glibc, kernel, container image, binary
  sha256) so verifiers can match the build environment. New
  `scripts/verify-build.sh` (POSIX shell, no extra crate deps) clones the
  repo at a tag, rebuilds, and diffs the local sha256 against the published
  one. New `wiki/reproducible-builds.md` documents the recipe and caveats.
- chore: trim binary by ~316 KB via feature-flag audit (was 3,083,400 bytes,
  now 2,760,032 bytes stripped). Dropped `tracing-subscriber/env-filter` (it
  pulled `matchers` + `regex-automata` + `regex-syntax`, ~175 KB), replaced
  with a hand-rolled `RUST_LOG` parser on top of `tracing_subscriber::filter::Targets`
  that supports the same `RUST_LOG=debug` and `RUST_LOG=proteus=trace,zbus=warn`
  syntax we document. Also dropped `tokio/macros` (no `#[tokio::main]`/`select!`
  used) and `tracing/attributes` (no `#[instrument]` used). The binary is now
  comfortably under the 3,000,000-byte CI cap.

## [0.1.0-alpha] - 2026-05-07

Pre-release. Phase A skeleton, Phase B L2 identity, partial Phase C, partial
Phase D, partial Phase F packaging, and the first slice of Phase G have
shipped. NOT a stable release; the CLI surface, config schema, and on-disk
formats may still change before v0.1.0.

### Added

#### Phase A — Skeleton

- Cargo project tuned for size: `opt-level="z"`, `lto`, `codegen-units=1`,
  `panic="abort"`, strip
- Pinned Rust toolchain + rustfmt config
- Full clap CLI surface — every subcommand parses; unimplemented ones return
  a "not implemented in this phase" stub with a stable exit code
- Read-only commands: `status`, `current`, `original`, `show-config`,
  `show-defaults`
- Embedded wiki via `include_dir!` with a terminal markdown renderer:
  ANSI styling on TTY, raw on pipe, `NO_COLOR` honored
- Logging via `tracing-journald` with stderr fallback when not under systemd
- Stable, documented exit codes (0, 1, 2, 64, 65, 66, 70)
- `proteus doctor` self-diagnostic health check
- `proteus config` CLI for runtime-controllable settings (no TOML editing
  required)

#### Phase B — L2 identity (MAC + Bluetooth)

- Wi-Fi and Ethernet MAC rotation via NetworkManager DBus (`zbus`, no
  `nmcli` shelling)
- OUI pool: Apple, Intel, Samsung, Dell prefixes plus locally-administered
  random
- ARP-table collision check (never assigns a gateway or live-neighbor MAC)
- `pin` / `unpin` per interface and per NM connection profile
- Bluetooth adapter alias + `discoverable=off` via BlueZ DBus
- BLE Resolvable Private Address mode where the controller supports it

#### Phase C — Probes, timers, captive portals

- NetworkManager dispatcher hook (`dist/networkmanager/dispatcher.d/01-proteus`)
  for event-driven rotation on connection up
- systemd sleep hook (`proteus-resume.service`) — re-rotate on resume
- `proteus probe` — manual probe quorum check
- `proteus timer` CLI — user-controllable systemd timers (status / enable /
  disable / set / reset / logs), no scripting required

#### Phase D — Hostname

- Hostname rotation (kernel / pretty / transient) via `hostname1` DBus
- 534-entry router-flavored hostname wordlist
  (`data/hostname-wordlist.txt`)

#### Phase F — Packaging + wiki

- 28 wiki pages: `intro`, `quickstart`, `concepts`, `mac-recipes`,
  `bluetooth`, `probes`, `rotation`, `captive-portals`, `dhcp`, `ipv6`,
  `hostname-recipes`, `enterprise-wifi`, `dns`, `discovery`,
  `stack-fingerprint`, `rf-fingerprinting`, `threat-model`, `cli`, `config`,
  `troubleshooting`, `verifying`, `uninstall`, `internals`, `faq`,
  `glossary`, `throttling-detect`, `doctor`, `timer`
- POSIX-shell `install.sh` and `uninstall.sh` (the latter is a wrapper; the
  underlying `proteus uninstall` impl is Phase G)
- systemd unit files: `proteus-rotate.timer`, `proteus-rotate.service`,
  `proteus-check.timer`, `proteus-check.service`, `proteus-boot.service`,
  `proteus-resume.service`
- Man page: `dist/man/proteus.1`
- Hand-written shell completions: bash, zsh, fish
- PolicyKit policy for future GUI wrappers
  (`dist/polkit/com.kit3713.proteus.policy`)
- `examples/` config presets: `minimal`, `standard`, `aggressive`,
  `captive-portal-heavy`, `paranoid`, `disabled`, `development`
- Distribution packaging:
  - Arch Linux PKGBUILD (`dist/arch/`)
  - Fedora / RHEL RPM spec + Copr config (`dist/rpm/`)
  - Debian / Ubuntu deb packaging, amd64 + arm64 (`dist/debian/`)
  - NixOS module + flake (`dist/nix/`)
- Release-artifact policy and architecture matrix (`dist/release.md`)
- aarch64 cross-compile lane in CI plus a release workflow that builds both
  arches, computes SHA256 sums, and drafts a GitHub Release
- `scripts/check.sh` local pre-push checker (fmt, clippy, test, build, size)
- `.editorconfig` and `.gitattributes`

#### Phase G — Reset

- `proteus reset` — restores config to defaults; the cached original MAC and
  hostname are sacred and never re-captured

### Project meta

- License: GPL-3.0-or-later
- `SECURITY.md`, `CONTRIBUTING.md`, `docs/PLAN.md`, `docs/PRIOR-ART.md`,
  `docs/ROADMAP.md`
- GitHub: PR template, issue templates (bug + feature), Dependabot, CI
  workflow (fmt, clippy, test, build, size check)

### Constraints honored

- Binary stays ≤ 3 MB stripped (size-checked in CI)
- Cold release build under 60s on the dev host
- No daemon — only NM dispatcher events, systemd timers, and a boot oneshot
- `proteus apply` is designed to be idempotent (callbacks land in a follow-up)
- Cached original MAC and hostname are sacred
- No telemetry, no update checks, no network egress beyond probe targets
- Will not weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`,
  or rotate `/etc/machine-id`

### Known gaps (deliberately deferred or in flight)

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full status.

- Phase C: probe-driven and timer-driven rotation callbacks (units exist,
  callbacks land next), captive portal detector + classifier + policy
- Phase D: DHCP option 12/60/61/81 suppression; IPv6 stable-privacy + DUID
  rotation; 802.1X anonymous outer identity; `dns.strip-edns-client-subnet`
- Phase E: discovery silencing, `tcp_timestamps=0` sysctl drop-in, ICMP
  rules, NDP hardening, NTP normalization, `wifi.tx-power-reduce`
- Phase F: full-text wiki search; error-path audit
- Phase G: `proteus diff`, `proteus dry-run`, `proteus uninstall` impl,
  privileged Podman + systemd integration tests
- Bluetooth BR/EDR (classic) BD_ADDR rotation: deferred (chipset-specific
  HCI territory)

[unreleased]: https://github.com/Kit3713/Proteus/compare/v0.1.0-alpha...HEAD
[0.1.0-alpha]: https://github.com/Kit3713/Proteus/releases/tag/v0.1.0-alpha
