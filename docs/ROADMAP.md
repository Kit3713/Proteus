# Roadmap — v0.3 "Reach + Persona"

This is the active roadmap. The phase-A-through-G build-out is complete and shipped in v0.2.7-alpha; that history lives in [`ROADMAP-v0.1.md`](ROADMAP-v0.1.md). The next cycle is **two big swings**:

1. **Reach** — get Proteus running well on any Linux distro / device, not just Fedora 43+ / systemd / NetworkManager. The headline change is a `NetworkBackend` abstraction that lets Proteus drive `systemd-networkd` or raw `ip` + `iw` + `wpa_supplicant`/`iwd` instead of being hardcoded to NM.
2. **Persona** — turn stealth into a first-class feature with two coexisting modes: the existing entropy-based **randomizer** (anonymity goal) gains a sibling **device-persona** mode (cover-identity goal) where every marker is shaped to look like a specific device — iPhone 15, MacBook Air M3, Pixel 8, Samsung TV, IoT camera, and 20+ more out of the box, with users free to author their own.

For design rationale and the original phase model, see [`PLAN.md`](PLAN.md). For per-version release notes, see [`CHANGELOG.md`](../CHANGELOG.md). For how to help, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).

## Status legend

- ✅ Landed (in `main`)
- 🚧 In progress (PR open)
- ⏳ Planned (next up)
- 💭 Deferred (in scope but not soon)

## Pre-cycle hotfix release: v0.2.8-alpha

The v0.2.7-alpha review surfaced six critical/high issues that ship before any v0.3 work starts. Hotfix scope is intentionally minimal — bug fixes only, no feature work, no roadmap restructure, no docs reshuffle.

- ⏳ 🔴 **#207 (critical, regression)** — `Connection.Update()` doesn't merge secrets in `src/nm/apply.rs` (rotate), `src/ipv6/nm.rs`, `src/nm/dhcp.rs`. Same class as #114, fix landed in only one of four sites. Every `proteus rotate` on a WPA-PSK Wi-Fi profile silently wipes the stored PSK. Fix lifts `merge_secrets` into a shared `nm::update_with_secrets(proxy, settings, secret_sections)` helper called from all four sites.
- ⏳ 🟠 **#209 (high, regression)** — `enterprise_wifi` originals keyed by display id; #124 migration silently deletes them on every state load. Fix routes through `nm::apply::read_connection_uuid` and keys by uuid (mirroring the working DHCP pattern).
- ⏳ 🟠 **#208 (high, regression)** — `capture_original_mac` (rotate.rs:264) falls back to NM's live `hw_address`, undoing the #123 factory-MAC guard on drivers without phy80211 / `ETHTOOL_GPERMADDR`. Fix drops the fallback and surfaces "no factory MAC captured" in `proteus status`.
- ⏳ 🟠 **#200 (high, test)** — `cargo test --release` fails on any host with `eth0` because `captured_factory_mac_persists_to_disk` reads real `/sys/class/net/eth0`. Fix wires the existing `permanent_address_under` injection point through `capture_original_mac_under` and uses `TempRoot` in the test.
- ⏳ 🟠 **#210 (medium, security)** — `points_to_resolved_stub` falls open on `canonicalize` failure (dangling-link case bypasses the DNS detect-and-defer). Fix drops the suffix-match fallback and returns `false` when canonicalize errors.
- ⏳ 🟠 **#201 (medium)** — ANSI warning leaks under `RUST_LOG=warn` / `-v`; `NO_COLOR=1` env still ignored. Fix consults `NO_COLOR` and stderr-isatty in `cli/mod.rs::run` before passing into `logging::init`.

Once v0.2.8-alpha tags and ships, Milestone 1 starts.

## Cycle overview

Six numbered milestones, executed roughly in order. Milestones 2–6 all build on Milestone 1.

| Milestone | Theme | Depends on |
|---|---|---|
| 1 | `NetworkBackend` abstraction (NM / networkd / raw) | — |
| 2 | Stealth: two coexisting modes (randomizer + device persona) | 1 |
| 3 | Per-SSID profile policies | 1, 2 |
| 4 | Finish fingerprint hardening + RF + rotation triggers | 1 |
| 5 | Distro reach (any-distro, any-arch) | 1 |
| 6 | CLI ergonomics, security review, docs, integration tests, ongoing bug-fix queue | runs alongside |

## Milestone 1 — `NetworkBackend` abstraction

**Why first:** per-SSID profiles, broader distro support, alternate network stacks, and several rotation/fingerprint features all need to call into "set this MAC / pin this connection / read this DHCP state" without caring whether NetworkManager, systemd-networkd, or raw `ip` is underneath.

**Explicit non-NM compatibility goal.** After this milestone ships, Proteus runs end-to-end on a stock distro that has *no NetworkManager installed at all* — either via `backend::networkd` (Fedora Server, NixOS, systemd-networkd-driven) or `backend::raw` (anything with `ip` + `iw` + `wpa_supplicant`/`iwd`, including OpenRC/runit-based distros from Milestone 5). A user on Alpine + iwd should be able to `proteus apply` without ever installing NM.

- ⏳ New `src/backend/` module with a `NetworkBackend` trait covering: enumerate interfaces, get/set cloned MAC per connection, get/set hostname-related DHCP options, trigger lease renew, read driver/chipset info, observe connection-up events.
- ⏳ Three implementations:
  1. `backend::nm` — moves the existing zbus code from `src/nm/` behind the trait. No behaviour change for the default path.
  2. `backend::networkd` — systemd-networkd via DBus (`org.freedesktop.network1`) + drop-in files in `/etc/systemd/network/`.
  3. `backend::raw` — `ip` + `iw` + `wpa_supplicant`/`iwd` direct, the "any distro" fallback.
- ⏳ Backend selection: `proteus doctor` autodetects; user can pin via `[backend] driver = "nm" | "networkd" | "raw" | "auto"`.
- ⏳ `proteus doctor` stops hard-failing when NM is absent — it reports which backends are available and which is selected.
- ⏳ All call sites in `src/commands/*.rs` route through the trait. The four `src/nm/` files become the NM backend's internals; `src/commands/dhcp.rs`, `src/commands/ipv6.rs`, `src/commands/apply.rs` lose their direct zbus imports.
- ⏳ Tests: a `MockBackend` impl drives the existing integration shell tests without containers; per-backend integration tests added to `tests/integration/scenarios/`.

**Issues absorbed by this milestone:**

- ⏳ 🟠 **#206-B** — `state_lock::HELD` is a process-wide `AtomicBool` that breaks down under multi-thread access. The new backend trait will be called from async event loops (this milestone's connection-up watcher, Milestone 4c's event-driven framework) which *will* introduce concurrent calls. Replace with `Mutex<Option<File>>` as part of this milestone's plumbing.
- ⏳ 🟡 **#206-C** — NM dispatcher sed-parses `proteus current --json` for `last_rotated`, fragile against nested keys. The backend trait gets a `rotate_if_needed(cooldown)` entry point that returns a typed result; the dispatcher calls it directly instead of grep'ing JSON.

**Acceptance:** full `proteus apply / revert / rotate` cycle works with NM (regression-free), with networkd (new), with raw (new). `proteus doctor` reports a backend matrix.

## Milestone 2 — Stealth: two coexisting modes (headline feature)

The next cycle's headline. Stealth becomes two modes that coexist in one binary; the user picks which one their threat model wants.

- **Randomizer mode (anonymity goal).** Every identifier rolls to entropy on a schedule. The user disappears into noise, no consistent target to lock onto. This is what Proteus does today; the existing `Profile { Off, Min, Low, Med, High, Agr }` slider is its aggressiveness control. **Default for fresh installs** so existing v0.2.7 users see no behaviour change on upgrade.
- **Persona mode (cover-identity goal).** Every marker is shaped to look like a specific device — iPhone 15, MacBook Air M3, Pixel 8, Samsung TV, IoT camera, etc. From `nmap -O`, p0f, fpdhcp / Fingerbank, passive WiFi capture, and OS-detection heuristics, the device should look like the chosen target. Wireshark + payload-content analysis is still defeatable — that boundary is documented in `wiki/personas.md` and the threat model.
- **Custom user profiles are a first-class feature.** Every built-in is just a `.toml` file under `data/personas/`; user-authored profiles in `/etc/proteus/personas/` use the same schema, the same loader, and appear next to the built-ins in `proteus persona list`. A user can author either kind:
  - A **custom randomizer recipe** — different OUI pools, different rotation cadences, different hostname patterns, different sysctl knob mixes than the built-in slider. Useful for users who want "Med profile, but with Intel + LG OUIs only and a 45m cadence" or similar non-stock blends.
  - A **custom stealth persona** — a specific device cover not in the built-in catalogue. The schema is the same one the built-ins use; users can clone any built-in (`proteus persona new my-iphone --from iphone-15`), edit the file, validate it, and use it. Sharing is a single file copy or `proteus persona export` / `import`.
  - Users switch between custom profiles, built-in profiles, and the entropy-only mode at will via `proteus persona use <id>` / `proteus persona clear`. No daemon restart, no reboot.

**Scope discipline.** Persona shapes only what Proteus already controls — MAC OUI + bytes pattern, hostname pattern, DHCP option ordering / vendor-class / parameter-request-list, IPv6 SLAAC choices, TCP window scale / MSS / sysctl knobs we already toggle, mDNS/LLMNR posture, RF TX-power band, BT name pattern. It does not touch TLS / browser / app-layer signatures.

### Schema and storage

- ⏳ New `src/persona/` module with a `Persona` struct: `id`, `display_name`, `kind` (`stealth` | `randomizer`), `category` (phone/laptop/tablet/tv/iot/router/console/printer/generic — only meaningful for `stealth`), `oui_pool`, `mac_byte_pattern`, `hostname_template`, `dhcp_fingerprint`, `tcp_stack`, `ipv6_traits`, `mdns_advertise`, `bt_name_template`, `rf_traits`, `rotate_cadence` (only meaningful for `randomizer`), `notes`.
- ⏳ Built-in stealth catalogue in `data/personas/*.toml` — at least: `iphone-15`, `iphone-13`, `pixel-8`, `pixel-6`, `galaxy-s24`, `macbook-air-m3`, `macbook-pro-m3`, `thinkpad-x1-carbon`, `dell-xps-13`, `surface-pro-9`, `ipad-air`, `samsung-tv-2024`, `lg-tv-2023`, `roku-ultra`, `chromecast`, `nest-mini`, `ring-doorbell`, `printer-generic-hp`, `printer-generic-canon`, `nintendo-switch`, `playstation-5`, `xbox-series-x`, `router-tplink`, `router-asus`, `iot-generic`. **25+ personas at launch.**
- ⏳ Built-in randomizer catalogue: the existing six `Profile` baselines (`off`/`min`/`low`/`med`/`high`/`agr`) gain identical-content `.toml` mirrors so they show up in `proteus persona list --kind randomizer` next to user-authored randomizer recipes. Functionally unchanged — purely a unification step.
- ⏳ User personas (both kinds): `/etc/proteus/personas/*.toml` only (system-wide; matches the root-via-polkit model). On id collision a user file shadows the built-in. Schema validation on load with wiki-linked errors.

### CLI

- ⏳ `proteus persona list [--kind stealth|randomizer] [--category phone|laptop|...] [--json]`
- ⏳ `proteus persona show <id>` — full schema + diff vs current device
- ⏳ `proteus persona use <id> [--apply]` — set active persona (and optionally apply in one step)
- ⏳ `proteus persona random [--kind stealth] [--category phone]` — pick a random persona, useful for scripted rotation between several covers
- ⏳ `proteus persona new <id> --from <id>` — clone an existing persona to `/etc/proteus/personas/` for editing
- ⏳ `proteus persona edit <id>` — open in `$EDITOR` (with the existing root-editor warning)
- ⏳ `proteus persona validate <path>` — schema check, prints exact field-level errors
- ⏳ `proteus persona current` — report active persona + kind + which fields are persona-shaped vs user-overridden vs profile-baseline
- ⏳ `proteus persona clear` — drop back to plain randomizer mode (no persona; `Profile` slider drives entropy)
- ⏳ `proteus persona import <path>` / `export <id> <path>` — share custom personas between machines

### Integration

- ⏳ Config: new `[persona] active = "..."` section. The aggressiveness `Profile` still gates *whether* features run; persona shapes *how* they look. Per-knob overrides beat persona; persona beats profile baseline.
- ⏳ `src/mac/oui.rs` gains a per-vendor `oui_for(Vendor)` registry that personas reference by token. Add Google, Microsoft, LG, TPLink, Asus, Roku, Amazon, generic-IoT to the table.
- ⏳ Hostname patterns: per-persona templates rendered from `data/hostname-wordlist.txt` plus persona-specific token sets (e.g. `{n}` digit, `{owner}` first-name pool). The existing 534-word list keeps powering router/IoT personas.
- ⏳ DHCP fingerprint: extend the DHCP-option path (now under `backend::*::dhcp`) to *set* `dhcp-vendor-class-identifier`, `dhcp-fqdn`, parameter-request-list ordering instead of only suppressing them. Defaults from persona; user override wins.
- ⏳ Threat model section in `wiki/threat-model.md`: "personas defeat OS-fingerprinting at L2/L3/L4 and DHCP/mDNS/RF; they do not defeat traffic-content analysis (Wireshark + payload inspection), TLS fingerprinting (JA3/JA4), or behavioural timing analysis. Use Tor / VPN for those."
- ⏳ New `wiki/personas.md` with the catalogue, build-your-own walkthrough, and verification checklist (`nmap -O` from a second host before/after).

### ARP / ND collision handling

The existing `src/mac/arp.rs` does a one-shot ARP-table check before assignment. For persona mode that's not strong enough — a persona-shaped MAC must not collide with anything live on the segment, and the failure mode should be visible to the user, not silent.

- ⏳ Pre-commit ARP probe: send an ARP request for the candidate (RFC 5227 ARP Probe). If a reply arrives, the MAC is taken — re-roll within the same OUI pool.
- ⏳ IPv6 parity: equivalent ND probe for the candidate's link-local address (DAD via Neighbor Solicitation).
- ⏳ Adaptive backoff: on three consecutive collisions, surface a warning and fall back to the next vendor in the persona's `oui_pool` (or, in randomizer mode, pure entropy). Log every collision with the conflicting neighbour's IP for forensic clarity.
- ⏳ Gateway / live-neighbour exclusion: extend the existing exclusion list (default-route gateway only) with all `arp -a` neighbours seen in the last N minutes (configurable, default 5).
- ⏳ Surfaced via `proteus rotate --explain` showing every candidate considered + reason for rejection.

**Acceptance:** `nmap -O` against the host before/after `proteus persona use iphone-15` produces materially different / matching-target detections. Integration test container has a side-car `nmap` runner that asserts persona effectiveness for a representative subset.

## Milestone 3 — Per-SSID profile policies

Builds on Milestone 1 (needs the backend trait to expose connection-keyed state) and Milestone 2 (each SSID can pin a different persona).

- ⏳ New `[per_ssid."<ssid>"]` config sections — fields: `persona`, `aggressiveness_profile` override, `pin_mac`, `rotate_interval` override, `portal_policy` override.
- ⏳ Match precedence at runtime: `per_ssid["X"]` (highest) → `[persona]` → `[profile]` baseline → `Config` defaults.
- ⏳ CLI: `proteus ssid list` / `ssid show <ssid>` / `ssid set <ssid> <key> <value>` / `ssid clear <ssid>`.
- ⏳ Integrates with the NM connection-up dispatcher and the new backend abstraction so changing networks re-applies the right SSID rules.
- ⏳ State migration: existing `known_portal_ssids` array merges into per-SSID with `portal_policy = "fresh-mac-per-visit"`.

## Milestone 4 — Finish fingerprint hardening + RF + rotation triggers

Three tightly related tracks; can land in parallel once Milestone 1 is done.

### 4a — Fingerprint hardening completion

- ⏳ `systemd-resolved` drop-in: mDNS responder + resolver off, LLMNR off (the ⏳ Phase E item) — `src/dns/resolved.rs` new module producing `/etc/systemd/resolved.conf.d/10-proteus.conf` with detect-and-defer if user has custom drop-ins, mirroring existing DNS drop-in logic.
- ⏳ `timesyncd` NTP normalization — `src/ntp/` new module producing `/etc/systemd/timesyncd.conf.d/10-proteus.conf` with persona-aware NTP server lists; skip if chrony or ntpd present.
- ⏳ `nftables` expansion — extend `src/nft/` with persona-aware rules (e.g. iOS personas drop port 5353 inbound by default, Android personas allow it), plus the long-standing optional rules: ICMPv4 timestamp drops, broadcast-ping drops, IGMP query suppression.

### 4b — RF surface controls finish

- ⏳ Complete `proteus rf` family from the partial stub: `rf scan` (passive-scanning preference), `rf chipset` (firmware/driver inventory), keep existing `status / apply / revert`.
- ⏳ Per-scan MAC randomization at the NM + wpa_supplicant layer (rescue branch `phase-d/wifi-privacy`).
- ⏳ Probe-request hygiene: never broadcast saved-SSID list (clamp `wifi.scan-rand-mac-address`, `mac-address-randomization`).
- ⏳ Chipset + firmware in `proteus status` (Wi-Fi driver/chip ID/firmware, BT chip vendor/firmware) — surfaces it via the new backend trait's `query_radio_info`.

### 4c — Rotation triggers

- ⏳ DHCP lease release+renew without MAC change (rescue `phase-d/ip-rotation`) — new `proteus dhcp renew` subcommand and config knob.
- ⏳ Event-driven framework (rescue `phase-c/event-driven-triggers` and `phase-c/auto-triggers`): triggers on connection-up, link-flap, regulatory-domain change, captive-portal auth completion. Routed through the backend's event stream.

## Milestone 5 — Distro reach (any-distro, any-arch)

The backend abstraction (Milestone 1) unblocks NM-less distros; this milestone closes the rest of the gap.

- ⏳ Init-system abstraction `src/init/` with `Systemd`, `Openrc`, `Runit`, `Sysvinit` impls covering: schedule a periodic check, hook resume-from-suspend, hook boot. Used by `dist/install.sh` and `proteus timer`.
- ⏳ Packaging:
  - Alpine APKBUILD (`dist/alpine/`) with musl + OpenRC service.
  - Void (`dist/void/`) with runit.
  - Gentoo ebuild (`dist/gentoo/`).
  - AUR submission (binary + -git) using existing PKGBUILD.
  - Copr submission for RPM.
  - Debian unstable submission.
- ⏳ Architectures: drop the `ExclusiveArch: x86_64 aarch64` gate from `dist/rpm/proteus.spec`, add **armv7** to the CI cross-compile matrix in `.github/workflows/`. Run the test suite at least under qemu for non-native arches. (Targeted matrix: x86_64 + aarch64 + armv7 covers laptops, Apple Silicon VMs, Raspberry Pi 2/3/4/5, ARM Chromebooks.)
- ⏳ `proteus doctor`:
  - Reports init system, libc, distro, backend, package format.
  - Suggests next step on misconfigured systems (e.g. "no NM and no networkd; install one or use `--backend=raw`").
  - Distro-compat warnings for known-quirky setups (Pi-hole, dnscrypt-proxy, openresolv, NetworkManager-l2tp).
- ⏳ Documentation: `wiki/distro-support.md` matrix.

## Milestone 6 — CLI ergonomics, security review, docs, integration tests, ongoing bug-fix queue

Cross-cutting polish; runs alongside the other milestones.

### CLI (rescue `feat/cli-ergonomics`)

- ⏳ Short aliases (`proteus s` for status, `r` for rotate, `a` for apply).
- ⏳ `--watch` mode for `status / current / session`.
- ⏳ `--format json|yaml|table` for all readers.
- ⏳ Colour theming via `NO_COLOR`.
- ⏳ `proteus completions <shell>` regenerator command.

### Tests

- ⏳ Image-diff verification of clean install/uninstall (the old roadmap's ⏳ item) — extend `tests/integration/run.sh` to take a SHA-tree of `/etc /var/lib /usr/bin /usr/share` before install and after uninstall and assert equality.
- ⏳ Per-backend container scenarios for nm / networkd / raw (Milestone 1's tests).
- ⏳ Persona effectiveness scenario (`nmap -O` sidecar, Milestone 2's test).
- ⏳ Real-world testing harness: `tests/realworld/` documenting how to run the read-only probe set on coffee-shop / hotel / conference / airport networks; not an automatable test, but a documented checklist in `wiki/real-world-testing.md`.

### Security

- ⏳ Independent DBus-surface review (the old roadmap's ⏳ item) — write `docs/security/dbus-surface.md` enumerating every DBus method called, every property read, every signal subscribed-to with arg validation guarantees. Solicit external review against this artifact rather than against the source.
- ⏳ Threat model expansion in `wiki/threat-model.md` for the persona feature (already noted in Milestone 2).
- ⏳ Bypass hardening pass: review every place we shell out (still after the L-3 interface-name fix); audit every parser added since the May 2026 audit.

### Docs

- ⏳ Audit pass: every error string in `src/error.rs` and every `bail!` / `anyhow!` callsite carries a `wiki <page>` hint (the ⏳ Phase F item).
- ⏳ Expand `wiki/troubleshooting.md` with a symptom → cause → fix table per backend, per init system, per persona.
- ⏳ New pages: `wiki/personas.md`, `wiki/backend.md`, `wiki/distro-support.md`, `wiki/per-ssid.md`.

### Bug-fix queue (rolling)

The medium/low-severity issues from the v0.2.7-alpha review land here as the milestones progress. The critical/high cluster ships in v0.2.8-alpha before Milestone 1 starts.

- ⏳ 🟠 **#204** — `State` has no `schema_version` despite #127's CHANGELOG claim. Add `schema_version: u32` with a migration ladder; bump `CURRENT_SCHEMA_VERSION` for the persona / per-SSID state additions in Milestones 2/3 anyway.
- ⏳ 🟠 **#203** — `state_lock` retry budget (1 s) too short for systemd-timer overlap with interactive `apply`. `PROTEUS_LOCK_TIMEOUT_MS` env var, default raised to 5 s, dispatcher/timer units set 10 s.
- ⏳ 🟠 **#202** — `factory::EthtoolBin` shells out via `$PATH`; pin to `/usr/sbin/ethtool` (matching the #121 hardening). Goes with the security review pass.
- ⏳ 🟡 **#211** — Exit codes 64/65/75 cleanup (`CONFIRMATION_REQUIRED` vs `LOCK_BUSY` vs `CONFIG_ERROR`). Pure CLI ergonomics.
- ⏳ 🟡 **#205** — `write_atomic` mode bits depend on caller umask. Add `fchmod(0o600)` after open. Goes with the security review pass.
- ⏳ 🟡 **#206-A** — `actions/checkout@v6` may not exist; pin to `@v4` or a SHA. Trivial CI fix, do early.
- ⏳ 🟡 **#206-D** — Unify `TempRoot` / `TestSysfs` naming. Trivial cleanup.
- ⏳ 🟡 **#206-E** — `EthtoolBin::permanent` doesn't validate MAC shape. Pairs with #202.
- ⏳ 🟡 **#206-F** — Perf doc reproducibility recipe is missing the rebuild step.
- ⏳ 🟡 **#206-G** — Perf doc miscounts `tracing::warn!` sites (says ~12, actual 14).
- ⏳ 🟡 **#206-H** — `tmp_path_for` fallback name "file" — bail instead of silently defaulting.

New issues from real-world testing land here too; high/critical findings cut a fast-follow `v0.3.x` patch release out-of-cycle.

## Things explicitly NOT on the roadmap

The mission is local controllable fingerprint reduction. These items live on another tool's layer or are physical limits:

- **TLS / browser fingerprint** (JA3/JA4 ClientHello, font / canvas / WebGL fingerprints) — use Tor Browser, librewolf, Brave's randomization. Persona mode does **not** touch this — the boundary is documented in `wiki/personas.md`.
- **Wireshark-class payload-content analysis** — persona mode shapes packet *headers* and protocol *fingerprints*, not payloads. A motivated analyst with full PCAP access and a content-decryption channel can still tell what's running. Use Tor / VPN if that's the threat.
- **DNS resolution policy beyond ECS strip** — use dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole.
- **Tracker blocking** — Pi-hole, NextDNS, uBlock Origin.
- **Traffic correlation defenses** — Tor, Mullvad.
- **SSH client fingerprint (HASSH)** — your `ssh_config` is yours.
- **Hardware-baked RF fingerprints** (oscillator drift, DAC nonlinearity, IQ imbalance) — physically impossible without a hardware swap; see `wiki/rf-fingerprinting.md`.
- **Telemetry, update checks, analytics** — no telemetry, ever.

## How to help

- **Real-world testing** — `proteus doctor` + `proteus apply` on coffee-shop / hotel / conference / airport networks; report bugs via the issue template (highest-value contribution right now).
- **Independent security review** — eyes on `wiki/threat-model.md` and the DBus surface in `src/nm/`, `src/bluetooth/`, `src/commands/dhcp.rs`, `src/commands/ipv6.rs`. Once Milestone 6's `docs/security/dbus-surface.md` lands, that's the artifact to review against.
- **Persona contributions** — once Milestone 2 lands, the `data/personas/*.toml` schema is open for community PRs to grow the catalogue.
- **Distro packaging** — Milestone 5 needs Alpine, Void, Gentoo packagers, plus AUR / Copr / Debian unstable submission sponsors.
- **Wiki** — pages are landed but always improvable; voice should match `wiki/intro.md`.
- **Code** — see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
