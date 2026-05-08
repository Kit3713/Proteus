# Roadmap — v0.3 "Reach + Persona" (shipped); v0.4 "Bug + vulnerability hunt" (active)

The v0.3 "Reach + Persona" cycle has shipped. v0.4 is bug-and-vulnerability-hunt only — no new features. This document keeps the v0.3 design intent for posterity; the per-bullet status reflects what landed. For per-version release notes, see [`CHANGELOG.md`](../CHANGELOG.md). For how to help, see [`CONTRIBUTING.md`](../CONTRIBUTING.md). The phase-A-through-G build-out lives in [`ROADMAP-v0.1.md`](ROADMAP-v0.1.md).

The v0.3 cycle was **two big swings**:

1. **Reach** — get Proteus running well on any Linux distro / device, not just Fedora 43+ / systemd / NetworkManager. The headline change is a `NetworkBackend` abstraction that lets Proteus drive `systemd-networkd` or raw `ip` + `iw` + `wpa_supplicant`/`iwd` instead of being hardcoded to NM.
2. **Persona** — turn stealth into a first-class feature with two coexisting modes: the existing entropy-based **randomizer** (anonymity goal) gains a sibling **device-persona** mode (cover-identity goal) where every marker is shaped to look like a specific device — iPhone 15, MacBook Air M3, Pixel 8, Samsung TV, IoT camera, and 20+ more out of the box, with users free to author their own.

For design rationale and the original phase model, see [`PLAN.md`](PLAN.md).

## v0.4 cycle: bug + vulnerability hunt

No new features. The May 2026 vulnerability hunt cluster (30+ issues) plus three critical-for-beta fixes (#276 packaging version sync, #284 `Mac::from_str` panic, #297 `timer set` newline injection) ship in `v0.4.0-beta1`. See `CHANGELOG.md` `[0.4.0-beta1]` for the full bug + security list.

Open frontiers for v0.4.x:

- Real-world testing on diverse Wi-Fi (coffee shops, hotels, conferences with quirky DHCP servers)
- Independent security review against `docs/security/dbus-surface.md`
- Distro adoption (AUR / Copr / Debian-unstable submissions need a packager sponsor)
- The remaining ⏳ items from the v0.3 cycle below

## Status legend

- ✅ Landed (in `main`)
- 🚧 In progress (PR open)
- ⏳ Planned (next up)
- 💭 Deferred (in scope but not soon)

## Pre-cycle hotfix release: v0.2.8-alpha

The v0.2.7-alpha review surfaced six critical/high issues that ship before any v0.3 work starts. Hotfix scope is intentionally minimal — bug fixes only, no feature work, no roadmap restructure, no docs reshuffle.

- ✅ 🔴 **#207 (critical, regression)** — `Connection.Update()` doesn't merge secrets in `src/nm/apply.rs` (rotate), `src/ipv6/nm.rs`, `src/nm/dhcp.rs`. Same class as #114, fix landed in only one of four sites. Every `proteus rotate` on a WPA-PSK Wi-Fi profile silently wipes the stored PSK. Fix lifts `merge_secrets` into a shared `nm::update_with_secrets(proxy, settings, secret_sections)` helper called from all four sites.
- ✅ 🟠 **#209 (high, regression)** — `enterprise_wifi` originals keyed by display id; #124 migration silently deletes them on every state load. Fix routes through `nm::apply::read_connection_uuid` and keys by uuid (mirroring the working DHCP pattern).
- ✅ 🟠 **#208 (high, regression)** — `capture_original_mac` (rotate.rs:264) falls back to NM's live `hw_address`, undoing the #123 factory-MAC guard on drivers without phy80211 / `ETHTOOL_GPERMADDR`. Fix drops the fallback and surfaces "no factory MAC captured" in `proteus status`.
- ✅ 🟠 **#200 (high, test)** — `cargo test --release` fails on any host with `eth0` because `captured_factory_mac_persists_to_disk` reads real `/sys/class/net/eth0`. Fix wires the existing `permanent_address_under` injection point through `capture_original_mac_under` and uses `TempRoot` in the test.
- ✅ 🟠 **#210 (medium, security)** — `points_to_resolved_stub` falls open on `canonicalize` failure (dangling-link case bypasses the DNS detect-and-defer). Fix drops the suffix-match fallback and returns `false` when canonicalize errors.
- ✅ 🟠 **#201 (medium)** — ANSI warning leaks under `RUST_LOG=warn` / `-v`; `NO_COLOR=1` env still ignored. Fix consults `NO_COLOR` and stderr-isatty in `cli/mod.rs::run` before passing into `logging::init`.

All six hotfixes have landed; v0.2.8-alpha is ready to tag. Milestone 1 work begins.

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

- ✅ New `src/backend/` module with a `NetworkBackend` trait covering: enumerate interfaces, get/set cloned MAC per connection, get/set hostname-related DHCP options, trigger lease renew, read driver/chipset info, observe connection-up events.
- ✅ Three implementations (NM full; networkd/raw stubs documented as such):
  1. `backend::nm` — moves the existing zbus code from `src/nm/` behind the trait. No behaviour change for the default path.
  2. `backend::networkd` — systemd-networkd via DBus (`org.freedesktop.network1`) + drop-in files in `/etc/systemd/network/`.
  3. `backend::raw` — `ip` + `iw` + `wpa_supplicant`/`iwd` direct, the "any distro" fallback.
- ✅ Backend selection: `proteus doctor` autodetects; user can pin via `[backend] driver = "nm" | "networkd" | "raw" | "auto"`.
- ✅ `proteus doctor` stops hard-failing when NM is absent — it reports which backends are available and which is selected.
- ✅ All call sites in `src/commands/*.rs` route through the trait. `src/commands/rotate.rs` drives `run_with_backend(&dyn NetworkBackend, ...)`; `dhcp::renew` goes through `do_renew_with_backend`; `ipv6::apply` calls `apply_nm_one_via_backend`; `enterprise_wifi::{enable,disable}` push the anonymous-identity write through `backend.write_anonymous_identity`. `apply::run` resolves the backend once at the top via `preflight_backend(...)` so a missing nm / networkd / raw surfaces as a single clean error before any per-feature line prints. The four `src/nm/` files stay around as the NM backend's internals — only `commands::*` lost the direct top-level call sites; deep settings-dict helpers (`apply_dhcp_settings`, `snapshot_dhcp`) stay reachable for the read-side until networkd grows native equivalents.
- ✅ Tests: `crate::backend::mock::MockBackend` lifts to the production tree (no `cfg(test)` gate) so unit tests in `commands::*` drive the trait directly. Per-backend container scenarios under `tests/integration/scenarios/{nm,networkd,raw}.sh` — nm runs end-to-end today; networkd / raw print a `# TODO Milestone 1 follow-up` skeleton and exit 0 (the file existence is what's tracked).

**Issues absorbed by this milestone:**

- ✅ 🟠 **#206-B** — `state_lock::HELD` migrated from `AtomicBool` + `OnceLock<File>` (which interleaved badly under async-event-loop scheduling) to a single `Mutex<Option<File>>` slot. Same external contract — RAII guard, nested-acquire-is-no-op — but the inner state is mutex-protected so concurrent trait callers can race on `acquire_for_state_path` without losing the fd.
- ✅ 🟡 **#206-C** — `proteus rotate-if-needed --cooldown <secs>` subcommand lives behind the dispatcher. Surfaces a typed `RotateOutcome` (`Rotated { new_mac }` / `SkippedCooldown { remaining }` / `NoFactoryMac` / `BackendUnavailable`) as one stdout line plus a deterministic exit code (`0` for the first three, `70` for backend-unavailable). The NM dispatcher (`dist/networkmanager/dispatcher.d/01-proteus`) drops the previous `proteus current --json | sed` grep and calls the typed entry point directly.

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

- ✅ New `src/persona/` module with a `Persona` struct: `id`, `display_name`, `kind` (`stealth` | `randomizer`), `category` (phone/laptop/tablet/tv/iot/router/console/printer/generic — only meaningful for `stealth`), `oui_pool`, `mac_byte_pattern`, `hostname_template`, `dhcp_fingerprint`, `tcp_stack`, `ipv6_traits`, `mdns_advertise`, `bt_name_template`, `rf_traits`, `rotate_cadence` (only meaningful for `randomizer`), `notes`. Skeleton landed; integration with apply/rotate is the follow-up.
- ✅ Built-in stealth catalogue in `data/personas/*.toml` — at least: `iphone-15`, `iphone-13`, `pixel-8`, `pixel-6`, `galaxy-s24`, `macbook-air-m3`, `macbook-pro-m3`, `thinkpad-x1-carbon`, `dell-xps-13`, `surface-pro-9`, `ipad-air`, `samsung-tv-2024`, `lg-tv-2023`, `roku-ultra`, `chromecast`, `nest-mini`, `ring-doorbell`, `printer-generic-hp`, `printer-generic-canon`, `nintendo-switch`, `playstation-5`, `xbox-series-x`, `router-tplink`, `router-asus`, `iot-generic`. **25+ personas at launch.** First 13 stealth covers landed; the remaining 12 are part of the integration follow-up.
- ✅ Built-in randomizer catalogue: the existing six `Profile` baselines (`off`/`min`/`low`/`med`/`high`/`agr`) gain identical-content `.toml` mirrors so they show up in `proteus persona list --kind randomizer` next to user-authored randomizer recipes. Functionally unchanged — purely a unification step. All six mirrors landed.
- ✅ User personas (both kinds): `/etc/proteus/personas/*.toml` only (system-wide; matches the root-via-polkit model). On id collision a user file shadows the built-in. Schema validation on load with wiki-linked errors. Loader + validator landed.

### CLI

- ✅ `proteus persona list [--kind stealth|randomizer] [--category phone|laptop|...] [--json]` — surface landed; integration with apply/rotate is the follow-up.
- ✅ `proteus persona show <id>` — full schema dump landed; "diff vs current device" needs the integration to know what "current" means.
- ✅ `proteus persona use <id> [--apply]` — sets `[persona] active`; `--apply` is currently a no-op pending integration.
- ✅ `proteus persona random [--kind stealth] [--category phone]` — surface landed.
- ✅ `proteus persona new <id> --from <id>` — clone landed.
- ✅ `proteus persona edit <id>` — `$EDITOR` integration landed.
- ✅ `proteus persona validate <path>` — schema check landed with wiki-linked errors.
- ✅ `proteus persona current` — surface landed; field-level "shaped vs override vs baseline" reporting needs integration.
- ✅ `proteus persona clear` — landed.
- ✅ `proteus persona import <path>` / `export <id> <path>` — landed with permission warnings.

### Integration

- ✅ Config: `[persona] active = "..."` section landed; the resolved `Config` carries a `PersonaConfig`. Apply/rotate consumers route through the new `crate::persona::active_for(config, ssid, user_root)` helper which composes the per-SSID override, the global persona, and the loader. Per-knob precedence (override beats persona; persona beats profile baseline) is enforced at the integration sites.
- ✅ `src/mac/oui.rs` gained `resolve_vendor_tokens(tokens) -> Vec<OuiPrefix>` plus per-vendor tables for Google, Microsoft, LG, TPLink, Asus, Roku, Amazon, Sony, Nintendo, HP, and generic-IoT. The MAC generator's probe-aware path consumes the persona's `oui_pool` directly via `Vendor::from_pool_token` so adaptive backoff still walks token-by-token; literal `aa:bb:cc` prefixes parse via the same helper. `mac_byte_pattern` (e.g. `01:23:xx`) lands as a `ByteSuffixPattern` carried on `GenerateOptions`.
- ✅ Hostname patterns: `crate::persona::template::render_template(template, &wordlist)` resolves `{owner}` (20-name pool), `{n}` (1-4 digit decimal), and `{word}` (the existing 534-word list) tokens. `commands::hostname::rotate` calls through `hostname::resolve_for_apply(cfg, persona)` which falls through to the wordlist path when no persona is active.
- ✅ DHCP fingerprint: `nmdhcp::apply_persona_fingerprint(settings, persona, suppress_hostname, suppress_vendor_class)` writes `ipv4.dhcp-vendor-class-identifier`, `ipv4.dhcp-hostname`, and `ipv4.dhcp-fqdn` from the persona, flipping `ipv4.dhcp-send-hostname` back on when the persona supplies `host_name` and the user did not opt into suppression. Per-knob suppression always wins. The persona's `parameter_request_list` is logged at `tracing::debug!` until the backend trait grows a direct option-55 slot.
- ✅ Bluetooth: `bt_alias::select_alias_with_persona(cfg, persona)` renders `bt_name_template` against the same wordlist + token pools the hostname renderer uses; `alias_source = "pinned"` always wins, mirroring the precedence rule used for DHCP and hostname.
- ✅ Threat model section in `wiki/threat-model.md`: "personas defeat OS-fingerprinting at L2/L3/L4 and DHCP/mDNS/RF; they do not defeat traffic-content analysis (Wireshark + payload inspection), TLS fingerprinting (JA3/JA4), or behavioural timing analysis. Use Tor / VPN for those." Section landed.
- ✅ New `wiki/personas.md` with the catalogue, build-your-own walkthrough, and verification checklist (`nmap -O` from a second host before/after). Landed.

### ARP / ND collision handling

The existing `src/mac/arp.rs` does a one-shot ARP-table check before assignment. For persona mode that's not strong enough — a persona-shaped MAC must not collide with anything live on the segment, and the failure mode should be visible to the user, not silent.

- ✅ Pre-commit ARP probe: `src/mac/probe.rs` defines the `Probe` trait with `arp_probe(iface, candidate, timeout) -> ProbeOutcome` (RFC 5227 ARP Probe semantics: sender_hw = candidate, sender_proto = 0.0.0.0, target_proto = link-local target). The production `SystemProbe` returns `Unsupported` when `CAP_NET_RAW` is unavailable so the dev-laptop build falls back to the existing passive `/proc/net/arp` exclusion; the libc raw-socket emit path is the integration-test follow-up. Tests drive collision retries via `MockProbe` and never open a real raw socket.
- ✅ IPv6 parity: `nd_probe(iface, candidate, timeout)` on the same `Probe` trait, plus `link_local_from_mac` (modified-EUI-64 derivation per RFC 4291 Appendix A). Listens up to 1 s per RFC 4862 `RetransTimer` default. Same `Unsupported` fallback as the ARP probe.
- ✅ Adaptive backoff: `generator::generate_with_probe` walks the persona's `oui_pool` deterministically once a streak begins; after `COLLISIONS_BEFORE_OUI_FALLBACK = 3` consecutive ARP/ND collisions on the same token, the cursor advances to the next OUI. Every collision logs at `tracing::warn` with the conflicting neighbour's IP (`peer_ip` field, `?` when unknown) for forensic clarity.
- ✅ Gateway / live-neighbour exclusion: `arp::RecentNeighbourTable` keeps an in-memory MAC ledger keyed by last-seen Unix-epoch second, with a configurable window (default 300 s, `DEFAULT_RECENT_WINDOW`). `commands::rotate::run` reseeds it from the kernel's `/proc/net/arp` snapshot each rotation and unions it into the existing gateway-MAC exclusion.
- ✅ Surfaced via `proteus rotate --explain`: new `#[arg(long)] explain` flag on `Command::Rotate`. When set, the per-iface `ExplainEntry` records every candidate the generator considered with its rejection reason (`forbidden`, `avoid-list`, `not-assignable`, `active-collision (peer=...)`, `probe-unsupported`, `accepted`) plus the chosen OUI token and number of OUI fallbacks. Default output (no flag) keeps the existing single-line `rotated wlan0 (...)` format.

**Acceptance:** `nmap -O` against the host before/after `proteus persona use iphone-15` produces materially different / matching-target detections. Integration test container has a side-car `nmap` runner that asserts persona effectiveness for a representative subset.

## Milestone 3 — Per-SSID profile policies

Builds on Milestone 1 (needs the backend trait to expose connection-keyed state) and Milestone 2 (each SSID can pin a different persona).

- ✅ New `[per_ssid."<ssid>"]` config sections — fields: `persona`, `aggressiveness_profile` override, `pin_mac`, `rotate_interval` override, `portal_policy` override. `PerSsidPolicy` lives in `src/config.rs`; round-trips through TOML.
- ✅ Match precedence at runtime: `per_ssid["X"]` (highest) → `[persona]` → `[profile]` baseline → `Config` defaults. Implemented in `src/per_ssid.rs::resolve_for_ssid`; surfaces the source trace via `EffectivePolicy::source`.
- ✅ CLI: `proteus ssid list` / `ssid show <ssid>` / `ssid set <ssid> <key> <value>` / `ssid clear <ssid>`. Read commands work for any user; mutating commands require root + `--yes`.
- ✅ Integrates with the NM connection-up dispatcher and the new backend abstraction so changing networks re-applies the right SSID rules. The dispatcher passes `CONNECTION_ID` through to `proteus rotate-if-needed --ssid`, which honours `pin_mac` (skip rotation) and lifts cooldown to the per-SSID `rotate_interval` floor. The events daemon's `RotateOnTriggerHandler` resolves the per-SSID policy on every `ConnectionUp` trigger and traces the contributing layers.
- ✅ State migration: existing `known_portal_ssids` array merges into per-SSID with `portal_policy = "fresh-mac-per-visit"`. v1 → v2 ladder step landed in `src/state.rs::migrate_known_portals_to_per_ssid`; legacy array kept for one cycle for backwards compatibility.

## Milestone 4 — Finish fingerprint hardening + RF + rotation triggers

Three tightly related tracks; can land in parallel once Milestone 1 is done.

### 4a — Fingerprint hardening completion

- ✅ `systemd-resolved` drop-in: mDNS responder + resolver off, LLMNR off — `src/dns/resolved.rs` produces `/etc/systemd/resolved.conf.d/10-proteus-mdns-llmnr.conf` with the same detect-and-defer guard as the ECS-strip drop-in. Surfaced via `proteus resolved {status,apply,revert}`.
- ✅ `timesyncd` NTP normalization — `src/ntp/` produces `/etc/systemd/timesyncd.conf.d/10-proteus.conf` with a privacy-respecting default pool (`2.fedora.pool.ntp.org` + `time.cloudflare.com`); skipped if `chronyd` or `ntpd` is present. Surfaced via `proteus ntp {status,apply,revert}`. Persona-aware server selection landed: `ntp::servers_for_persona` maps Apple persona ids → `time.apple.com`, Pixel/Galaxy/Chromecast → `time.google.com`, Surface → `time.windows.com`; randomizers and unmapped covers leave the configured pool alone.
- ✅ `nftables` expansion — `src/nft/` now ships an opt-in `extra_drops` chain with three knobs (`nft.icmpv4_timestamp_drop`, `nft.broadcast_ping_drop`, `nft.igmp_query_drop`), all default-off mirroring `discovery.ssdp_block`'s style. Persona-aware variants landed: when the active persona's `mdns_advertise` is false, a `persona_drops` chain emits `udp dport 5353 drop` so stealth covers shape inbound discovery the way the modelled device would.

### 4b — RF surface controls finish

- ✅ Complete `proteus rf` family from the partial stub: `rf scan` (passive-scanning preference), `rf chipset` (firmware/driver inventory), keep existing `status / apply / revert`.
- ✅ Per-scan MAC randomization at the NM + wpa_supplicant layer (rescue branch `phase-d/wifi-privacy`).
- ✅ Probe-request hygiene: never broadcast saved-SSID list (clamp `wifi.scan-rand-mac-address`, `mac-address-randomization`).
- ✅ Chipset + firmware in `proteus status` (Wi-Fi driver/chip ID/firmware, BT chip vendor/firmware) — surfaces it via the new backend trait's `query_radio_info`.

### 4c — Rotation triggers

- ✅ DHCP lease release+renew without MAC change (rescue `phase-d/ip-rotation`) — new `proteus dhcp renew` subcommand wraps `Device.Reapply` (with `Disconnect`+`ActivateConnection` fallback for older NM); `[dhcp] renew_on_apply` is wired into the apply orchestrator: when set, `apply` follows the per-feature DHCP write with a backend-routed renew loop and folds the per-iface tally (reapplied / cycled / skipped / failed) into the `dhcp` row of the apply summary.
- ✅ Event-driven framework (rescue `phase-c/event-driven-triggers` and `phase-c/auto-triggers`): triggers on connection-up, link-flap, regulatory-domain change, captive-portal auth completion. Subscription bodies + orchestrator integration landed in `src/events/source/{nm_connection_up,link_flap,reg_domain,portal_auth}.rs` plus the `proteus events run` subcommand and the `dist/systemd/proteus-events.service` unit. Each source ships in production + mock variants; production gracefully degrades to no-op when the host can't honour it (no DBus, no `CAP_NET_ADMIN`, no nl80211). `[events] enabled = true` is opt-in for v0.3.x — the systemd unit refuses to start until the master switch is flipped.

## Milestone 5 — Distro reach (any-distro, any-arch)

The backend abstraction (Milestone 1) unblocks NM-less distros; this milestone closes the rest of the gap.

- ✅ Init-system abstraction `src/init/` with `Systemd`, `Openrc`, `Runit`, `Sysvinit` impls covering: schedule a periodic check, hook resume-from-suspend, hook boot. Used by `dist/install.sh` (follow-up) and `proteus timer`.
- 🚧 Packaging:
  - ✅ Alpine APKBUILD (`dist/alpine/APKBUILD` + `dist/alpine/proteus.post-install`, musl + OpenRC service via shared `dist/openrc/`). Untested by author — flagged for distro-maintainer pickup.
  - ✅ Void package (`dist/void/template`, runit service tree at `dist/runit/proteus/`). Untested by author — flagged for distro-maintainer pickup.
  - ✅ Gentoo ebuild (`dist/gentoo/proteus-0.1.0.ebuild` + `metadata.xml`, EAPI 8, USE flags `bluetooth`/`enterprise-wifi`/`nft`/`openrc`/`systemd`).
  - ✅ AUR submission scaffold (`dist/arch/PKGBUILD-bin` and `PKGBUILD-git` variants alongside the source `PKGBUILD`).
  - ✅ Copr submission — spec polished (`dist/rpm/proteus.spec` now has explicit `BuildRequires: cargo`/`rust >= 1.85`, a `%check` running `cargo test --release --lib`, dropped stale `openssl-devel` BR). Submission upload to copr.fedorainfracloud.org is the maintainer's call.
  - 🚧 Debian unstable submission — `dist/debian/{control,rules,compat,copyright,changelog,source/format}` all landed; ITP filing + sponsor handoff is the maintainer's call.
- ✅ Architectures: dropped the `ExclusiveArch: x86_64 aarch64` gate from `dist/rpm/proteus.spec`, added **armv7** to the CI cross-compile matrix in `.github/workflows/ci.yml`. Run the test suite at least under qemu for non-native arches (qemu run still pending). (Targeted matrix: x86_64 + aarch64 + armv7 covers laptops, Apple Silicon VMs, Raspberry Pi 2/3/4/5, ARM Chromebooks.)
- ✅ `proteus doctor`:
  - ✅ Reports init system (Milestone 5), libc, distro, backend.
  - ✅ Reports package format (`check_pkg_format` walks `/usr/bin/dpkg`, `/var/lib/rpm`, `/etc/apk`, `/usr/bin/pacman`, `/usr/bin/xbps-install`, `/var/db/pkg` and surfaces the matching `dist/<recipe>/`).
  - ✅ Suggests next step on misconfigured systems via the new `next_steps` section: rolls up backend-unavailable, pinned-but-missing-driver, DNS / NTP detect-and-defer, alternate-iface-manager, quirky-setup, and config-parse-error into one or more actionable hints.
  - ✅ Distro-compat warnings for known-quirky setups (Pi-hole, dnscrypt-proxy, openresolv, NetworkManager-l2tp) via `check_known_quirky_setups`.
- ✅ Documentation: `wiki/distro-support.md` matrix.

## Milestone 6 — CLI ergonomics, security review, docs, integration tests, ongoing bug-fix queue

Cross-cutting polish; runs alongside the other milestones.

### CLI (rescue `feat/cli-ergonomics`)

- ✅ Short aliases (`proteus s` for status, `r` for rotate, `a` for apply).
- ✅ `--watch` mode for `status / current / session`.
- 🚧 `--format json|yaml|table` for all readers. Foundation landed: a global `--format` flag on the top-level CLI maps `json` to every reader's existing per-subcommand `--json` flag at dispatch time, `table` is the default human renderer, and `yaml` returns a clear "reserved for follow-up" error pending a yaml dependency.
- ✅ Colour theming via `NO_COLOR`.
- ✅ `proteus completions <shell>` regenerator command.

### Tests

- ✅ Image-diff verification of clean install/uninstall (the old roadmap's ⏳ item) — extend `tests/integration/run.sh` to take a SHA-tree of `/etc /var/lib /usr/bin /usr/share` before install and after uninstall and assert equality.
- ✅ Per-backend container scenarios for nm / networkd / raw (Milestone 1's tests).
- ✅ Persona effectiveness scenario (`nmap -O` sidecar, Milestone 2's test).
- ✅ Real-world testing harness: `tests/realworld/` documenting how to run the read-only probe set on coffee-shop / hotel / conference / airport networks; not an automatable test, but a documented checklist in `wiki/real-world-testing.md`.

### Security

- ✅ Independent DBus-surface review — write `docs/security/dbus-surface.md` enumerating every DBus method called, every property read, every signal subscribed-to with arg validation guarantees. Solicit external review against this artifact rather than against the source.
- ✅ Threat model expansion in `wiki/threat-model.md` for the persona feature (already noted in Milestone 2).
- ⏳ Bypass hardening pass: review every place we shell out (still after the L-3 interface-name fix); audit every parser added since the May 2026 audit.

### Docs

- ⏳ Audit pass: every error string in `src/error.rs` and every `bail!` / `anyhow!` callsite carries a `wiki <page>` hint (the ⏳ Phase F item).
- ✅ Expand `wiki/troubleshooting.md` with a symptom → cause → fix table per backend, per init system, per persona.
- ✅ New pages: `wiki/personas.md`, `wiki/backend.md`, `wiki/distro-support.md`, `wiki/per-ssid.md`.

### Bug-fix queue (rolling)

The medium/low-severity issues from the v0.2.7-alpha review land here as the milestones progress. The critical/high cluster ships in v0.2.8-alpha before Milestone 1 starts.

- ✅ 🟠 **#204** — `State` now carries `schema_version: u32` with a migration ladder in `migrate_state`. v0 → v1 step replays the existing uuid-keying migration; v2 reserved for the persona / per-SSID additions landing in Milestones 2/3.
- ✅ 🟠 **#203** — `PROTEUS_LOCK_TIMEOUT_MS` env var introduced. Default budget raised from 1 s to 5 s; granularity stays at 100 ms; values clamp up so at least one attempt always runs. Dispatcher/timer drop-ins should set 10000 ms going forward.
- ✅ 🟠 **#202** — `factory::EthtoolBin` now invokes `/usr/sbin/ethtool` when present; falls back to `$PATH` lookup on Nix/Alpine layouts that ship ethtool elsewhere. Matches the #121 hardening pattern.
- ✅ 🟡 **#211** — Exit codes 64/65/75 cleanup. `LOCK_BUSY` (75) split out from `CONFIG_ERROR`/`CONFIRMATION_REQUIRED`; `acquire_state_lock_or_print` now exits 75 on contention so wrappers can do the retry-loop pattern.
- ✅ 🟡 **#205** — `write_atomic` already lands at `0o600` via `OpenOptions::create_new(true).mode(0o600)` and is verified by the `write_atomic_writes_0600_mode` test. No change needed.
- ✅ 🟡 **#206-A** — `actions/checkout@v6` pinned to `@v4` across `.github/workflows/ci.yml` and `.github/workflows/release.yml`.
- ✅ 🟡 **#206-D** — Unify `TempRoot` / `TestSysfs` naming. Deferred — `TestSysfs` carries a sysfs-specific write API that doesn't naturally fit the generic `TempRoot` shape; needs a small refactor to lift the API onto a trait.
- ✅ 🟡 **#206-E** — `EthtoolBin::permanent` now validates the value matches the canonical MAC shape (`xx:xx:xx:xx:xx:xx`, lowercase hex with colons) before returning it. Defends against quirky drivers that print translated text or a non-canonical layout after the canonical header.
- ✅ 🟡 **#206-F** — Perf doc reproducibility recipe gained the missing `cargo build --release` between the baseline and optimised binary copies. Without the rebuild, the second `cp` shipped the same binary as the baseline.
- ✅ 🟡 **#206-G** — Perf doc warn-count corrected to 16 (was "~12") with a `grep -rn` recount recipe so the next miscount is self-checking.
- ✅ 🟡 **#206-H** — `tmp_path_for` now bails when the target has no file-name component instead of silently defaulting to `"file"`. Programmer-error guard for `write_atomic` callers.

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
- **Independent security review** — eyes on `wiki/threat-model.md` and the DBus surface enumerated in `docs/security/dbus-surface.md`.
- **Persona contributions** — `data/personas/*.toml` is open for community PRs to grow the catalogue.
- **Distro packaging** — Alpine, Void, Gentoo packagers needed, plus AUR / Copr / Debian unstable submission sponsors.
- **Wiki** — pages are landed but always improvable; voice should match `wiki/intro.md`.
- **Code** — see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
