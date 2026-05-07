# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

See [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view of what has
landed, what is in flight, and what is on the bench. See
[README.md](README.md) for the project overview.

## [Unreleased]

### Roadmap follow-ups

- **M3 connection-up wiring** — `rotate-if-needed` grows `--ssid`; the
  NM dispatcher passes `CONNECTION_ID` through; per-SSID `pin_mac`
  short-circuits the rotate and `rotate_interval` lifts the cooldown
  floor. The `proteus events run` daemon's `RotateOnTriggerHandler`
  resolves the per-SSID policy on every `ConnectionUp` trigger and
  traces the contributing layers.
- **M4a persona-aware NTP** — `ntp::servers_for_persona` maps Apple /
  Pixel / Galaxy / Surface persona ids onto vendor-NTP pools so the
  wire-side NTP queries match the cover identity. Randomizers and
  unmapped covers leave the configured pool alone.
- **M4a persona-aware nft** — new `persona_drops` chain emits
  `udp dport 5353 drop` when the persona's `mdns_advertise` is false.
  Distinct chain priority (-97) preserves the issue-#148 eval-order
  invariant.
- **M4c `renew_on_apply`** — orchestrator now runs a backend-routed
  renew loop after a successful DHCP write when `[dhcp] renew_on_apply`
  is set. Folds the per-iface tally (reapplied / cycled / skipped /
  failed) into the `dhcp` row of the apply summary; flips status to
  Failed when the renew breaks.
- **M5 doctor** — new `next_steps` section synthesises actionable
  hints from the existing checks: backend-unavailable,
  pinned-but-missing driver, DNS / NTP defer, alternate iface manager,
  quirky setup, config parse error.
- **M6 `--format`** — global `--format json|yaml|table` flag at the
  CLI top level. `json` flips every reader's per-subcommand `--json`
  at dispatch time; `table` is the default; `yaml` redirects through
  a hand-rolled JSON-to-YAML emitter (no new deps — walks
  `serde_json::Value` and emits YAML block-style, with proper
  string-quoting for scalars that would parse ambiguously as
  numbers / booleans).
- **M6 bypass hardening pass** — `docs/security/bypass-hardening-pass.md`
  enumerates every `Command::new` site and every parser added since
  the May 2026 audit. New `crate::process` module pins privileged
  shellouts (`nft`, `ip`, `sysctl`, `systemctl`, `journalctl`, `ss`,
  `dmesg`, `semanage`) to canonical absolute paths with PATH
  fallback. Parser audit found and fixed two bugs in
  `per_ssid::parse_duration`: a panic on multi-byte trailing chars
  (`30é`) and silent overflow wrap on `n * 86_400` (now `checked_mul`).
- **M6 wiki-hint audit** — operator-facing `bail!` / `anyhow!`
  callsites in user-actionable error paths (config schema, OUI
  tokens, timer durations, pin/unpin, watch interval) now carry a
  `proteus wiki <page>` hint. Every reference verified to point at
  a real wiki page (`grep -roh 'proteus wiki [a-z-]*' src/` against
  `wiki/*.md`).

### Runtime performance

- **Config cache** — `Config::default_or_loaded` now hits a
  process-level mtime-keyed cache. The apply orchestrator's 12-call
  sequence (one per feature module) collapses from 12 file-reads +
  12 TOML parses to 1 read + 1 parse + 11 stat lookups. Invalidates
  on any mtime advance, so `proteus config edit` and hand-edits are
  picked up on the next call.
- **Built-in persona cache** — every embedded persona TOML is parsed
  once at first access into a `OnceLock<HashMap<id, Persona>>`. A
  single `proteus apply` cycle resolves the persona at six sites
  (rotate / hostname / dhcp / bluetooth / ntp / nft); the cache turns
  the per-site cost from O(parse) into O(hash).
- **Events daemon config cache** — the long-lived `proteus events run`
  daemon previously re-parsed `config.toml` on every trigger. Now
  caches by mtime and re-parses only when the operator actually edits
  the file.
- **Doctor PATH cache** — `binary_exists` walked `$PATH` independently
  for each of 8 lookups (chronyd / ntpd / nft / iwd /
  wpa_supplicant / dnscrypt-proxy / kresd / AdGuardHome). Now splits
  PATH once into a `OnceLock<Vec<PathBuf>>` and reuses for every
  lookup in the same CLI invocation.
- **Hostname wordlist cache** — `hostname::wordlist()` previously
  re-trimmed and re-validated all ~560 embedded entries on every
  call. The function is hit 2-3 times per `proteus rotate` / `apply`
  (MAC OUI shaping path, hostname rendering, Bluetooth alias);
  caching collapses the per-call cost to a pointer load + Vec clone
  of static references.

### Fixes

- **clippy cleanups** — removed an unused `PathBuf` import in
  `mac/factory.rs` tests, replaced manual `div_ceil` in
  `state_lock.rs` with the standard-library form, swapped
  `or_insert_with(Default::default)` for `or_default()` in
  `state.rs::migrate_known_portals_to_per_ssid`, dropped a redundant
  closure around `factory::permanent_address` in
  `commands/rotate.rs`. No functional changes.
- **`apply_json_to_command`** collapsed redundant per-action `match`
  arms into a single OR-pattern so the dispatch helper that
  implements `--format json` is one match expression rather than
  thirteen.
- **`rf chipset` table headers** — header rows in the `proteus rf
  chipset` table now inline the trailing column literal into the
  format string, which silenced the empty-format-string clippy
  warning without changing the rendered output.
- **field-assignment style** — three test helpers (`commands::ntp`,
  `commands::resolved`, `commands::rotate`) now build their structs
  in struct-literal-with-spread form instead of mutating after
  `Default::default`. Pure style; no functional difference.
- **`commands::rotate` test idiom** — replaced an
  `original_macs.get("eth0").is_none()` assertion with the more
  idiomatic `!original_macs.contains_key("eth0")`.

After this batch, `cargo clippy --all-targets` produces zero warnings.



## [0.3.1-alpha] - 2026-05-07

Final wrap-up batch on the v0.3 cycle. Roadmap moves to 5⏳ / 78✅ /
4🚧 — ~92% complete on bullet count. 794 tests passing.

### Fingerprint hardening completion (M4c)

- **Event sources** wired end-to-end. `proteus events run` daemon
  registers handlers for connection-up / link-flap / regulatory-domain
  change / portal-auth, with Mock variants for tests and real
  socket-probes that gracefully degrade to `Unsupported` when
  CAP_NET_ADMIN is absent. New `dist/systemd/proteus-events.service`
  with `RestrictAddressFamilies` / `AmbientCapabilities=CAP_NET_ADMIN
  CAP_NET_RAW`. Opt-in via `[events] enabled = true`.

### Doctor improvements (M5)

- `pkg-format` check reports the host's native package manager (deb /
  rpm / apk / pacman / xbps / portage) and points at the matching
  `dist/<recipe>/` entry.
- `quirky-setup` warning surfaces Pi-hole / dnscrypt-proxy /
  openresolv-without-binary / NetworkManager-l2tp profiles so the
  operator knows ahead of time which Proteus features will defer.

### Tests + harnesses (M6)

- `tests/integration/scenarios/persona-effectiveness.sh` — `nmap -O`
  before/after a persona apply, asserts the OS-detection row changed
  materially (Milestone 2 acceptance).
- `tests/integration/scenarios/image-diff.sh` — SHA tree of `/etc`,
  `/usr/bin`, `/var/lib`, etc. before install and after uninstall
  asserts byte-equality (catches stray-file regressions in
  install.sh).
- `tests/realworld/probe.sh` + README — read-only network-state
  capture for coffee-shop / hotel / conference / airport debugging,
  with anonymisation pass for public IPs and SSIDs.

### Bug fixes

- **#206-D** `TempRoot` / `TestSysfs` unified. `TestSysfs` now wraps
  `crate::testing::TempRoot` and adds the sysfs-specific writers as
  methods. Drop semantics, naming scheme, and collision resistance
  match the canonical `TempRoot`.

### Docs

- `wiki/backend.md` — user-facing `NetworkBackend` reference.
- `docs/security/dbus-surface.md` — implementation-side artifact for
  external security review.
- `wiki/troubleshooting.md` — symptom × backend / init-system /
  persona / exit-code matrix.

## [0.3.0-alpha] - 2026-05-07

First v0.3 cycle release. v0.2.8-alpha hotfix batch + Milestones 1-6
substantial completion. Roadmap moved from 31⏳/13✅ to 11⏳/72✅
(~80% complete on bullet count). Test suite grew 421 → 755 (+334).

### v0.2.8-alpha hotfix batch (rolled into 0.3.0-alpha)

- **#207 (critical)** `Connection.Update()` now merges secrets at every
  call site via the shared `nm::update_with_secrets` helper. Rotate /
  DHCP / IPv6 / 802.1X paths all preserve the stored PSK + EAP password.
- **#209 (high)** `enterprise_wifi` originals now keyed by NM
  `connection.uuid` (mirrors the working DHCP pattern), so the v0.2.6
  uuid-keying migration no longer wipes them on every state load.
- **#208 (high)** `capture_original_mac` no longer falls back to NM's
  live `hw_address`; on drivers without phy80211 / `ETHTOOL_GPERMADDR`
  the original is left empty and `proteus status` surfaces "no factory
  MAC captured".
- **#200 (high)** `cargo test --release` is hermetic on hosts with
  `eth0`. The factory-address lookup is injected through the existing
  `permanent_address_under` test hook.
- **#210 (medium)** `points_to_resolved_stub` no longer falls open on
  canonicalize failure. Dangling-link case defers cleanly.
- **#201 (medium)** `NO_COLOR=1` and stderr-isatty are now consulted
  before `logging::init`; ANSI warning no longer leaks under
  `RUST_LOG=warn` / `-v`.

### v0.3 cycle — milestones

- **Milestone 1 — `NetworkBackend` abstraction.** Trait + three impls
  (NM full, networkd / raw stubs documented as such), backend
  selection via `[backend] driver`, doctor matrix, all per-command
  call sites routed through the trait. `state_lock` migrated to
  `Mutex<Option<File>>` (#206-B). NM dispatcher script replaced with
  `proteus rotate-if-needed` (#206-C).
- **Milestone 2 — Persona / Randomizer dual-mode stealth.** 25
  stealth covers + 6 randomizer mirrors. Schema, loader, validator,
  full 11-subcommand CLI surface, persona integration with apply /
  rotate / hostname / DHCP / Bluetooth. RFC 5227 ARP probe + IPv6
  DAD + adaptive backoff with persona oui_pool rotation. Wiki page +
  threat-model addendum.
- **Milestone 3 — Per-SSID profile policies.** `[per_ssid."<ssid>"]`
  config, `proteus ssid list / show / set / clear`, four-layer
  resolver with source trace, v1→v2 schema migration that mirrors
  legacy `known_portal_ssids` into per-SSID seed entries.
- **Milestone 4a — Fingerprint hardening.** `proteus resolved`
  (mDNS+LLMNR off), `proteus ntp` (timesyncd normalization,
  detect-and-defer for chrony / ntpd), nftables `extra_drops`
  chain (ICMPv4 timestamp / broadcast ping / IGMP query, all opt-in).
- **Milestone 4b — RF surface.** `proteus rf scan` + `proteus rf
  chipset`, per-scan MAC randomization, probe-request hygiene,
  chipset+firmware in `proteus status`.
- **Milestone 4c — Rotation triggers.** `proteus dhcp renew`
  (Reapply with Disconnect+ActivateConnection fallback). Event-driven
  framework scaffolding (`RotationTrigger` / `EventHandler` /
  `EventRegistry`); source bodies stubbed for the wiring follow-up.
- **Milestone 5 — Distro reach.** Init-system abstraction
  (`Systemd`/`Openrc`/`Runit`/`Sysvinit`), ARM + i686 cross-compile
  matrix, Alpine APKBUILD + Void template + Gentoo ebuild + AUR
  `-bin`/`-git` variants + Copr spec polish + Debian submission-prep,
  `wiki/distro-support.md`.
- **Milestone 6 — Ergonomics + bug-fix queue.** Short aliases
  (`proteus s/r/a`), `--watch` mode for `status`/`current`/`session`,
  `proteus completions <bash|zsh|fish>`, `LOCK_BUSY` exit code
  (#211), `State::schema_version` migration ladder (#204), 13
  bug-fix-queue items (#202 / #203 / #205 / #206-A/E/F/G/H / #211).

### Deferred to v0.3.1+

- Persona-aware NTP / nftables variants (the per-knob defaults ship
  now; persona-driven values are a follow-up).
- Event-source bodies (raw socket / netlink / nl80211 subscriptions).
- networkd / raw write paths (NM is the only fully-wired backend
  today; the trait + scenario scripts are in place for the follow-up).
- Image-diff verification of clean install/uninstall.
- `nmap -O` persona-effectiveness scenario in CI.
- `wiki/backend.md`, `docs/security/dbus-surface.md`.
- `--format yaml` for readers (would pull a YAML dep).
- Debian ITP filing + sponsor handoff.
- Unifying `TempRoot`/`TestSysfs` (#206-D).

## [0.2.7-alpha] - 2026-05-07

Sixth alpha point release. Closes the remaining issue queue: the high-severity multi-profile NM bug, the medium-severity state-keying bug, and the last low-severity polish item.

### Bug fixes
- **#122 (high)** `proteus rotate` and `proteus ipv6 apply` now iterate every NM connection profile bound to a device instead of only the first. A laptop with multiple stored Wi-Fi profiles (home / work / café / hotel / conference) used to leak the original MAC and un-rotated DUID through whichever profile didn't get touched. Per-profile failures are logged but don't fail the whole rotate.
- **#124 (medium)** `state.originals.connections` and `state.managed.connections` are now keyed by NM `connection.uuid` (the only uniqueness guarantee NM offers) instead of `connection.id` (a display string). Two profiles sharing an id no longer overwrite each other's snapshot. Old state.json files have id-keyed entries silently dropped on load — alpha state contract; the next `proteus apply` re-captures originals correctly.
- **#164 (low)** `proteus config set` warns on stderr when overwriting a value whose existing TOML type doesn't match the parsed-default type. Surfaces the user's typo (e.g. `mac.enabled = "no"`) instead of silently rewriting it.

## [0.2.6-alpha] - 2026-05-07

Fifth alpha point release after v0.2.0. Closes the remaining low-severity issue queue and the 2026-05-07 security audit.

### Security
- **L-2** timer expressions: reject `\n`, `\r`, `\0`, `[`, `]` in `proteus timer set` interval input so a calendar expression cannot break out of its systemd drop-in unit.
- **L-2** disable-reason: `proteus config disable --reason` strips `\n` / `\r` from the reason before it lands in the on-disk comment.
- **L-4** editor-as-root: `proteus config edit` warns when `$HOME != /root` because `$EDITOR` plugins / autoloads will inherit root privileges; recommends `sudo -H proteus config edit` or `proteus config set` for narrow edits.
- v0.2.5-alpha covered the higher-severity audit findings: **H-1** removed root-side `xdg-open` for portal URLs (now print-only with http(s) scheme guard), **M-3** CRLF rejection on captive-portal HTTP requests, **L-3** interface-name validation for `iw`/`ip` shell-outs.

### Bug fixes
- **#161** `proteus reset` prunes config backups beyond `MAX_BACKUPS = 5` so cached identifiers don't accumulate indefinitely.
- **#163** `proteus portal status` (and `proteus session`) caches the last detector result for 60 s so `watch -n 1` polling stops hammering shared third-party detect endpoints.
- **#166** `proteus help <feature>` falls back to wiki search with line snippets when no exact page name matches, instead of dumping an alphabetical page list.
- **#160** RF parser hardening for `iw dev <iface> info`.
- **#165** `build.rs` reruns at file-level granularity so wiki edits don't invalidate the whole build cache.

### Packaging
- **deb**: `debian/rules` uses `--locked` (offline mode for the cargo fetch step), drops the conflicting `dist/debian/compat`, and the lane skips `dpkg-checkbuilddeps` since `rustc` is rustup-provided and not in the dpkg DB.
- All five distro pipelines (x86_64 raw, aarch64 cross, RPM, .deb, Arch) ship release artifacts.

## [0.2.0-alpha] - 2026-05-07

Second alpha. Patches 28 issues across security, state safety, NM DBus correctness, CLI ergonomics, observability, and packaging. Adds a runtime-efficiency baseline. The CLI surface, config schema, and on-disk formats remain provisional.

### Security
- #112: CLI errors now print the full `anyhow` source chain to stderr instead of being swallowed silently.
- #113: tracing-subscriber suppresses ANSI when stderr is not a TTY or `NO_COLOR` is set.
- #114: enterprise-wifi `Connection.Update` now calls `GetSecrets` first so EAP passwords/certs survive the round-trip.
- #115: NM `cloned-mac-address` now written as `ay` (byte array) and `ipv6.addr-gen-mode` as `i32` — the types older NM versions accept.
- #116: `state.json` and `config.toml` writes now land at `0o600` mode.
- #117: mutating commands return `CONFIRMATION_REQUIRED` (65) instead of the misleading `NOT_IMPLEMENTED` (64) when `--yes` is missing.
- #118: `proteus pin` honors its `--yes` flag.
- #119: every apply path persists captured originals to `state.json` *before* the destructive mutation (sacred-originals invariant).
- #120: polkit `exec.path` now `/usr/bin/proteus` to match distro-package install paths.
- #121: NM dispatcher hook uses absolute paths and resets `$PATH` to a root-owned set, removing the privilege-escalation surface.
- #123: `original_macs` now records the burned-in factory MAC (via `phy80211/macaddress`, ethtool `ETHTOOL_GPERMADDR`, or `addr_assign_type == NET_ADDR_PERM`), not whatever was live when first observed.
- #125 + #150: `commands::write_atomic` defends against TOCTOU/symlink attacks via random-suffix temp filenames + `O_CREAT | O_EXCL` + RAII cleanup, and now fsyncs the parent directory after rename.
- #126: every mutating command acquires an advisory `flock(2)` on `<state-dir>/.lock` before mutating; concurrent runs serialize.
- #127: `State::load` quarantines a malformed `state.json` (renaming to `<path>.corrupt-<ts>`) and returns an empty state so read-only commands keep working.
- #128 + #129: probe and captive-portal flows now bound DNS resolution and HTTP body reads by their declared timeouts via std `mpsc::sync_channel` watchdogs.
- #130: DNS detect-and-defer guard canonicalizes drop-in symlinks before string-matching, defeating attacker-controlled tail redirection.
- #133: polkit mutating actions now use one-shot `auth_admin` (no `auth_admin_keep` cache window).
- #136: release pipeline drops `--skipchecksums` from makepkg.

### Bug fixes
- #131: `proteus reset` writes a minimal `profile = "<name>"` TOML preserving the active profile rather than the resolved (frozen) defaults.
- #132: `logging::parse_rust_log` keeps valid directives when one is malformed instead of dropping them all.
- #134: systemd services with `After=network-online.target` now also have matching `Wants=` so the target actually pulls up.
- #135: CI workflow uses the pinned `actions-rust-lang/setup-rust-toolchain` instead of the floating `dtolnay/rust-toolchain@stable`.
- #137 + #141: `--config /nonexistent/path` is now a hard error for read commands instead of silently falling back to defaults.
- #137 + #142: `NO_COLOR` env var is honored by logging init.
- #139 + #144: hostname revert skips per-name when no original was captured instead of collapsing to "".
- #140: `session::next_rotation_pair` parses systemctl with `LC_ALL=C` to avoid locale-dependent test failures.
- #143: bluetooth alias picker uses rejection sampling to remove modulo bias.
- #145 + #138: captive-portal URL parser handles IPv6 literals and userinfo correctly.
- #146: `/proc/net/route` parser checks `RTF_GATEWAY`; ARP parse errors surface instead of silently dropping.
- #147: `sysctl_path` validates the interface name (defense-in-depth).
- #148: nft chains use distinct priorities; revert is idempotent against races.
- #149: `src/stack/sha256.rs` uses `chunks_exact(64)` so the partial-block path is exercised correctly.
- #151: `revert_dhcp_settings` no longer materializes an empty `[ipv6]` section when none existed.
- #152 + #154: `bluetooth::apply_one` skips powered-off adapters instead of aborting the run.
- #153 + #155: `revert_best_effort` tracks per-step results and skips unconditional restarts when nothing changed.
- #157: kill switch documents the VPN-tunnel skip so wiki and code agree.
- #158: removed the duplicate Debian install path (`dist/install` vs `dist/debian/rules`).

### Performance
- perf: lazy `tracing-subscriber::registry()` init when verbose=0 + RUST_LOG unset + JOURNAL_STREAM unset. Saves ~46K instructions per cold-path invocation.
- New `docs/perf-baseline.md` captures methodology and baseline measurements.

### CI / packaging
- ci: raise the binary size cap from 3,750,000 bytes to 4,000,000 bytes. The defensive scaffolding from the security and state-safety fixes (state lock, atomic-write guard, `acquire_state_lock_or_print`, error-chain printing) added ~250 KB. The 4 MB target leaves ~250 KB headroom for v0.3 work.

### Deferred to v0.3
- Issues #122 + #124 (NM uuid keying, multi-profile updates) — implementation lives in branch `fix/issues-122-124-nm-uuid-keying-multi-profile`; conflicts deeply with #185 / #126 changes and needs a clean re-build against current main.
- Tracking issues #168, #169 — buckets for follow-up low-severity findings.
- The `claude/security-audit-sjnXX` branch's findings — incoming review will be addressed in v0.3.

## [0.1.0-alpha] - 2026-05-07

First public alpha. The codebase reflects every phase landed in main as of the tag commit. The CLI surface, config schema, and on-disk formats remain provisional and may change before v0.1.0. See `docs/ROADMAP.md` for the operational view, `wiki/profiles.md` for the six functional profiles, and `wiki/real-world-testing.md` for the field guide.

### Added

- feat(phase-h): `proteus rf` — Wi-Fi chipset inventory + opt-in TX-power
  reduction. New subcommands: `rf status` (read-only; lists driver, PCI/USB
  vendor/device IDs, firmware, current TX power, regulatory max for every
  Wi-Fi interface, plus the BlueZ adapter inventory for cross-referencing
  RF-fingerprinting research), `rf apply` (writes the configured floor via
  `iw dev <iface> set txpower fixed <mbm>` against every Wi-Fi interface;
  captures the original TX power once on first apply), `rf revert` (restores
  the cached pre-Proteus TX power exactly). New `[rf]` config section with
  `tx_power_reduce` (bool, default off in `min`/`low`/`med`, **on** in
  `high`/`agr`) and `tx_power_reduction_db` (u8, default `6`). Wired into
  `proteus apply` after `stack` and before `nft`; skips clearly when the
  master switch is off, no Wi-Fi hardware is detected, or `iw` is not on
  PATH. Emits a risk-warning line when the knob is on. Real-hardware
  effects are driver-dependent — if reception degrades after `rf apply`,
  run `sudo proteus rf revert --yes` to restore.
- feat(profiles): per-profile timer cadence baselines with user overrides.
  New `[timers.rotate]` and `[timers.check]` config sections set the systemd
  cadence for `proteus-rotate.timer` and `proteus-check.timer` respectively.
  Each profile carries a baseline (`off`/`min` -> `never`, `low` -> `4h`/`5m`,
  `med` -> `2h`/`5m`, `high` -> `30m`/`2m`, `agr` -> `15m`/`1m`); user
  overrides survive profile changes via the existing override-only-if-present
  resolution. `proteus apply` reconciles the configured intervals against the
  on-disk drop-ins at `/etc/systemd/system/proteus-*.timer.d/` (writing,
  updating, or removing as needed) and restarts the affected units. The
  apply path requires a real systemd context; non-systemd environments are
  skipped cleanly. `proteus config set timers.rotate.interval 1h --yes` is
  the supported CLI path for setting overrides.
- feat: `proteus session` — current network session snapshot in one read-only
  view (active SSID, gateway, link-layer MAC, hostname, captive-portal
  classification). Designed for GUI wrappers via `--json`.
- feat(phase-c): captive portal detection + `proteus portal` subcommand
  family. Classifies as `clear` / `portal-required` / `portal-authed` /
  `unknown` against `nmcheck.gnome.org` (configurable). New subcommands:
  `portal status`, `portal mark <ssid>`, `portal unmark`, `portal list`,
  `portal open` (launches the auth page in the default browser). Policy
  hooks for `rotate-before-auth` (default), `preserve-mac`, and `ask`.
- feat(phase-d): DHCP option suppression via NM DBus (12/60/61/81 + DHCPv6
  DUID/IAID). Per-managed-connection state is captured to `state.json` and
  restored by `proteus dhcp revert`. Driven entirely through NM settings
  keys — no `nmcli` shelling.
- feat(phase-d): DNS ECS-strip drop-in for systemd-resolved with
  detect-and-defer hard guard. If dnscrypt-proxy / Pi-hole / a non-default
  resolver is present, Proteus refuses to install the drop-in and exits 0
  with a friendly note. The user's DNS setup always wins.
- feat(phase-d): IPv6 stable-privacy + temporary addresses + DUID rotation.
  Per-iface sysctls go in a Proteus drop-in; per-NM-connection IPv6 settings
  are applied via DBus. `proteus ipv6 revert` restores the cached originals.
- feat(phase-d): 802.1X anonymous outer identity (eduroam, corporate Wi-Fi).
  Opt-in, default off. `proteus enterprise-wifi enable --connection <id>`
  sets `802-1x.anonymous-identity = anonymous@<realm>` per connection.
- feat(phase-e): nftables ruleset for ICMP info-reply drops, optional SSDP
  block, optional WSD block. Installed as the `inet proteus` table; left
  alone by `nft list ruleset` if absent.
- feat(phase-e): sysctl drop-in for TCP/ICMP/NDP stack hardening
  (`/etc/sysctl.d/95-proteus.conf`). Header SHA tracked so drift is
  detectable via `proteus stack status`.
- feat(phase-f): full-text wiki search via build-time index. `proteus wiki
  search <terms>` returns ranked hits with snippets; ~50ms cold on the
  ~38-page corpus.
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
- feat(phase-g): `proteus revert` — restore cached originals (hostname,
  Bluetooth alias, IPv6 + DHCP per-connection settings, sysctl/timesyncd/
  resolved drop-ins, dispatcher hook, nft table). Idempotent and shared
  with `proteus uninstall`.
- feat(phase-g): `proteus diff` — config-vs-defaults-vs-live drift detection
  including managed-file SHA verification.
- feat(phase-g): `proteus dry-run <cmd>` — preview any mutator without
  applying. Routes through a `Plan` enum so the same code path describes
  and executes.
- feat(phase-g): integration test scaffolding (privileged podman + systemd
  container with stubbed NM and BlueZ).
- feat: curated wiki TOC at `wiki/_index.md` — `proteus wiki` (no args) now
  renders the curated table of contents instead of an alphabetical page
  list. Search and the page list still exclude `_index` itself.
- feat: `proteus apply` risk warnings — when applying a config with knobs
  known to break specific things on some networks (`discovery.ssdp_block`,
  `discovery.wsd_block`, `enterprise_wifi.anonymous_outer_identity`,
  `stack.suppress_gratuitous_arp`), prints a one-line warning per active
  knob with a wiki pointer before running the orchestrator.

### Changed

- refactor: split monolithic `src/cli.rs` (744 lines) into
  `src/cli/{mod,command,actions,dispatch}.rs`. No behavior change — `Cli`,
  `Command`, every action enum, and `cli::run` keep the same paths.
- fix(phase-d): `proteus apply` now wires the dhcp/dns/stack/nft/ipv6
  modules into the orchestrator (previously they returned
  `not yet implemented` even though the modules themselves had landed).
  `proteus revert` now also calls `ipv6::revert` and `dhcp::revert`
  explicitly so per-NM-connection DBus state is restored alongside the
  on-disk drop-in cleanup.
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
- ci: raise the binary size cap from 3,000,000 bytes to 3,750,000 bytes.
  Phase D/E feature growth (DHCP, DNS, IPv6, stack, nft, captive portal,
  kill switch, enterprise-wifi, profiles, RF inventory) plus the new
  CLI subcommands and 38 wiki pages have legitimately outpaced the
  original target. Round 1 of the bloat audit (PR #94) trimmed every
  safely-droppable feature flag; remaining bloat is intrinsic to
  async DBus (zbus), the embedded wiki blob, and the clap derive
  surface for the expanded subcommand set. The 3.75 MB number is a
  considered target with ~200 KB headroom for the next phase of work.

### Phase scaffolding (historical detail for v0.1.0-alpha)

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

[unreleased]: https://github.com/Kit3713/Proteus/compare/v0.2.0-alpha...HEAD
[0.2.0-alpha]: https://github.com/Kit3713/Proteus/releases/tag/v0.2.0-alpha
[0.1.0-alpha]: https://github.com/Kit3713/Proteus/releases/tag/v0.1.0-alpha
