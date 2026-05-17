# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

See [docs/ROADMAP.md](docs/ROADMAP.md) for the operational view of what has
landed, what is in flight, and what is on the bench. See
[README.md](README.md) for the project overview.

## [Unreleased]

## [1.0.1] - 2026-05-17

Distro publishing infrastructure. No user-visible code changes; pure
release-pipeline work. Cut to exercise the new auto-publish jobs
end-to-end.

### Added — distro publishing pipelines

- **Copr auto-publish** (PR #483) — `.github/workflows/release.yml`
  gained a `publish-copr` job that submits the build-rpm SRPM to a
  configured Copr project on every tag push. Fedora users install with
  `sudo dnf copr enable kit3713/proteus && sudo dnf install proteus`.
  Setup walkthrough at `dist/copr/README.md`. Graceful skip when the
  `COPR_TOKEN` secret is unset, so forks of the repo don't see a red
  workflow. (PR #484 split the config: `COPR_LOGIN`/`COPR_TOKEN` stay
  in repo Secrets; `COPR_USERNAME`/`COPR_PROJECT` move to repo Variables
  so they're visible in logs for debugging.)
- **Launchpad PPA auto-publish** (PR #485) — `publish-ppa` GHA job
  builds a Debian source package per Ubuntu series in
  `vars.LAUNCHPAD_SERIES` (default: `noble`), signs it with the
  imported GPG key, and `dput`s it to `vars.LAUNCHPAD_PPA`. Ubuntu
  users install with `sudo add-apt-repository ppa:kit3713/proteus &&
  sudo apt install proteus`. Setup walkthrough at
  `dist/launchpad/README.md`. Same graceful-skip pattern.
- **OBS publishing** (PR #485) — `dist/obs/_service` template + setup
  walkthrough at `dist/obs/README.md`. OBS pulls from GitHub itself on
  a daily cadence (optional webhook for instant rebuild), so no GHA
  involvement is needed. Builds for Ubuntu + Debian + Fedora +
  openSUSE in one place.

### Packaging

- `dist/rpm/proteus.spec` Version → 1.0.1
- `dist/debian/changelog` prepended with `proteus (1.0.1-1)` stanza
- `dist/arch/PKGBUILD` and `dist/arch/PKGBUILD-bin` pkgver → 1.0.1

## [1.0.0] - 2026-05-17

First stable, non-beta release. v0.4.x closed every reachable High
and Critical bug-hunt finding; v1.0.0 lands the CLI ergonomics wave
on top: 18 new read-mostly subcommands, two residual hardening fixes,
and a clean bug-tracker. No breaking changes against v0.4.3-beta —
every existing shape, exit code, and `--yes` gate is preserved.

This release commits to semver: the CLI surface, on-disk state schema,
config schema, exit codes, and `--yes` gate semantics are now stable.
Breaking changes will bump the major version (v2.0.0+).

### Added — CLI ergonomics wave

- **`proteus version --json` / `proteus about`** (#376) — structured
  build info: package version, git sha, rustc, target triple,
  `SOURCE_DATE_EPOCH`-aware build timestamp. Wrappers can pin the
  exact binary they're driving without screen-scraping `--help`.
- **`proteus logs`** (#390) — thin journalctl wrapper across every
  Proteus systemd unit + the NM dispatcher syslog tag. Honours
  `--follow`, `--lines`, `--since`, `--json`. Degrades cleanly when
  journalctl is absent (`SYSTEM_NOT_SUPPORTED`).
- **`proteus state info` / `proteus state ...`** (#300) — read-only
  state.json summary: schema version, file path, size, per-iface
  last-rotated, count breakdown across managed interfaces, connections,
  pinned entries, cached originals. `--json` parity. New `state`
  top-level namespace reserved for future `state migrate` / `state
  dump`.
- **`proteus backup <path>` / `proteus restore <path>`** (#353) —
  first-class backup/restore for `/etc/proteus/` + `/var/lib/proteus/`.
  `tar.gz` with lstat-symlink reject, mode 0o600, optional SHA-256
  pinning via `--expected-sha`. Restore requires `--yes` and acquires
  the state lock for safety. The `contrib/recovery-kit/` sidecar
  scripts stay in place for distros without the binary installed.
- **`proteus pin list`** (#364) — read-only inverse of `pin`.
  Enumerates pinned interfaces + connections from state. Added
  `pinned_at: Option<String>` ISO-8601 timestamp to per-iface +
  per-connection records (set on `pin`, cleared on `unpin`).
- **`proteus unpin --all` / `--scope <type>`** (#392) — symmetric
  bulk-clear. `--all` removes every pin; `--scope iface|nm-connection`
  filters by kind. Both modes require `--yes`.
- **`proteus rotate --json`** (#395) — single-line `{"results": [...]}`
  envelope per iface, `outcome` as a stable categorical token
  (`rotated`/`skipped`), explain payload folded in under `--explain`.
- **`proteus rotate --reason "<text>"`** (#294) — optional audit
  string stamped into the per-iface state record alongside
  `last_rotated`, echoed at `tracing::info!`. Bounded 256 bytes;
  control bytes (C0/C1/bidi/DEL) stripped; truncated on overflow.
- **`proteus apply / revert --json`** (#343) — per-component summary
  envelope `{"command", "components": [...], "exit_code"}` for CI /
  Ansible consumers.
- **`proteus config show --annotate`** (#404) — sections marked with
  provenance (`file`/`profile:<name>`/`per-ssid:<ssid>`/`default`).
  Field-level annotation is a v0.5.x follow-up.
- **`proteus config explain <key>`** (#394) — surfaces type, default,
  current value, source file, doc-comment, risk warning, wiki link
  for any of 62 catalogued keys. Unknown keys exit `CONFIG_ERROR` (65)
  with a closest-known-key suggestion.
- **`proteus persona search <query>`** (#383) — case-insensitive scan
  over persona ids / display_names / notes; ranked id-exact >
  id-prefix > substring.
- **`proteus persona delete <id>`** (#338) — removes a user-authored
  persona with `--yes` + lstat-symlink reject + active-persona guard.
  Built-in personas are refused.
- **`proteus persona random --use [--apply]`** (#356) — one-shot
  pick + activate; `--apply` chains through `proteus apply`. Both
  require `--yes`.
- **`proteus events list-sources`** (#283) — read-only enumeration
  of the four event sources with a host-side availability probe
  (NetworkManager marker for `nm-connection-up`, netlink bind probes
  for `link-flap` + `reg-domain`, always-available for `portal-auth`).
- **`proteus events status`** (#393) — live per-source / per-handler
  counters + uptime + last-fired timestamps from a 1s on-disk snapshot
  the daemon writes. Missing/stale snapshots surface as
  `SYSTEM_NOT_SUPPORTED` (70) — the wrapper-friendly "daemon not
  running" signal.
- **`proteus events trigger <name> --debug --yes`** (#346) — synthetic
  trigger dispatch through an in-process registry for CI containers
  without a daemon. Live-daemon trigger IPC returns `NOT_IMPLEMENTED`
  pending a follow-up.
- **`proteus wiki list [--json]`** (#406) — programmatic enumeration
  of every embedded wiki page with title + description.

### Added — residual hardening

- **V11 follow-up** (#461) — `iot-generic` persona OUI pool now
  resolves cleanly. `Vendor::Espressif` (9 IEEE prefixes) and
  `Vendor::Realtek` (7) added to `src/mac/oui.rs`. Built-in personas
  hard-fail on unknown vendor tokens.
- **S3 regression test** (#462) — `mode(0o600)` on the state-lock
  `OpenOptions` is now pinned by an explicit `umask(0o000)` →
  acquire → stat test so a future refactor can't drop it without CI
  catching it.

### Tooling

- Stripped release binary size cap raised to 5.5 MB to accommodate
  the 18-subcommand expansion. CI + release workflows updated in
  lockstep.

### Closed (auto-closed via merged PRs)

The 18 ergonomics issues above closed via `Closes #N` on their
respective PRs. The wave-2 hardening backlog (#339, #345, #351,
#354, #355, #357, #358, #359, #360, #361, #362, #363, #365, #366,
#367, #370, #373, #374, #375, #377, #379, #380, #381, #382, #388,
#389, #391) was closed by hand with citations to the roadmap stream
that landed each fix.

## [0.4.3-beta] - 2026-05-12

Wave-2 v0.4.x hardening pass — closes every reachable High and Medium
roadmap ⏳ item across CLI safety, events daemon, NM backend, state
lock, panic hardening, error handling, security surface, and the
Stream 10 wiki-hint sweep. Only intentional defers remain in
`docs/ROADMAP.md`: typed-error refactor (cycle-sized work for v0.5+),
real-world Wi-Fi testing (needs physical access), and independent
security review (needs external party). See the roadmap for the
per-stream landing details.

### Known sharp edges

- **CLI prefix matching (CL5)** — `clap` accepts shortest-unambiguous
  subcommand prefixes by default (`proteus per` resolves to `persona`,
  `proteus pi` resolves to `pin`). A future subcommand that shares a
  prefix with an existing one (e.g. a hypothetical `pinger`) will
  silently change what `proteus pi` resolves to. Scripts should spell
  out full subcommand names — `proteus pin`, not `proteus pi`. There
  is no commitment to keeping prefix resolutions stable across
  releases.

### Security

- **NM `GetSecrets` failure surfacing (E6)** — `nm::update_with_secrets`
  routes through a new `get_secrets_or_warn` chokepoint with typed
  benign-vs-hard error classification. `AccessDenied` (polkit),
  `NoReply` (DBus disconnect), and unrecognised NM `MethodError`
  variants land as hard failures (with `tracing::error!` and an
  abort BEFORE `proxy.update(settings)`, so the stored secret is never
  silently wiped); empty dicts, `SettingNotFound`, `InvalidSetting`,
  `AgentManager.NoSecrets`, `InvalidProperty`, `MissingProperty`, and
  FDO `UnknownProperty`/`UnknownInterface` stay benign and merge as
  empty. Tracing routes the connection label through `display_string`
  so attacker-controlled secret values can't redraw `journalctl -t
  proteus`.
- **State-lock skew detection (C2)** — `backend::nm::remaining_cooldown`
  now treats `last > now` (wall-clock moved backward) and
  `elapsed > 30 days` (when configured cooldown < 30 days) as skew
  signals: returns `None` + `tracing::warn!` so a rotate proceeds
  rather than ratcheting into an "in the future" trap. Long-cooldown
  operators (≥ 30 days) get the literal remaining-budget answer.
- **Per-iface rotate debounce (N14)** — `backend::nm` now keeps a
  process-wide per-iface `Arc<tokio::sync::Mutex<()>>` registry,
  acquired BEFORE the state lock. Two same-process concurrent
  `rotate_if_needed` tasks on the same iface now fully serialise
  (state lock's reentrancy used to let both pass the cooldown
  check). Different ifaces still proceed in parallel.
- **Polkit hardening recipe** (`wiki/polkit-hardening.md`) — documents
  the optional `/etc/polkit-1/rules.d/49-proteus.rules` JS rule that
  restricts Proteus actions to `unix-group:wheel`/`sudo`, plus the
  `pkcheck --action-id ... --process $$` runtime-check pattern.
  Cross-linked from `wiki/security-checklist.md` and `wiki/_index.md`.

### Bug fixes

- **NCMD2.4 wire-up** — `revert::validate_cached_connection_uuids`
  is now called from `revert_best_effort` BEFORE any per-feature
  revert runs and honours the global `--state` override. Recycled
  NM uuids no longer cause Proteus to restore its cached snapshot
  onto an unrelated profile.
- **`--yes` end-to-end coverage (CL2/M1/N12.1/N12.2/N12.3)** — DHCP
  `apply`/`revert`, portal `mark`/`unmark`/`open`, and `unpin` now
  honour the project-wide `--yes` gate; previously the flag was
  silently dropped on dispatch's rest-pattern. Closes #348, #375,
  #391.
- **`rotate-if-needed --state` end-to-end (GH#381)** — backend trait
  signature now takes `state_path: Option<&Path>`; every impl
  (nm, mock, raw, networkd) honours it; C6 (mock flock) and N14
  (per-iface mutex) reuse the same arg.
- **`watch --interval` lower bound (CL1/CL7)** — sub-1ms intervals
  rejected at parse time with CONFIG_ERROR (65) instead of pegging
  a CPU core via `thread::sleep(0)`.
- **CaptivePortalReload primitive restored (R4)** — `Arc<RwLock<
  CaptivePortalConfig>>` shape exposed in `src/captive_portal/mod.rs`
  with `new`/`snapshot`/`swap`/`handle` + `Clone`. Tests that
  previously couldn't compile now do.
- **NM `Reapply` race detection (NBE.7)** — `nm/dhcp.rs::renew_lease`
  reads `Settings.Connection.VersionId` (NM 1.20+) and passes it to
  `Device.Reapply`, so a concurrent `nmcli connection modify` surfaces
  as a DBus version-mismatch error rather than a silent stale-write.
- **`ethtool -P` parser accepts Linux 6.3+ headers (NBE.10)** — matches
  both `permanent address:` and `permanent mac address:` (Intel
  iwlwifi variant).
- **Events daemon SIGTERM shutdown drain (C4)** — `tokio::select!`
  races the 250ms tick against `SignalKind::terminate()` /
  `SignalKind::interrupt()`. On signal the loop breaks normally so
  the existing `shutdown_tasks` source drain + in-flight rotate
  `JoinHandle` drain (both bounded at 5s) become reachable from
  systemd's `ExecStop` path.
- **Handler-panic visibility (C7)** — `EventRegistry::fire` wraps
  every `h.handle(&trigger)` in `catch_unwind(AssertUnwindSafe(...))`,
  logs panics at `tracing::error!` with `handler_index` + `kind` +
  downcast payload, bumps a per-registry `handler_panics: AtomicU64`
  counter, and continues dispatch so a single panicking handler can't
  take down the daemon.
- **Persona-effectiveness scenario poll (NTEST.2)** — replaces fixed
  `sleep 5` with a 60s-default poll loop on `proteus current --json`
  (MAC + `last_rotated`). Slow CI runners no longer conflate
  baseline and persona-applied variants.
- **clap arg ranges (N12.12)** — bounded with `value_parser!(T).range(...)`:
  `timer logs --lines` 1..=100_000, `wiki search --limit` 1..=500,
  `events run --max-triggers` 0..=10_000_000, `events run
  --once-after-secs` 0..=86_400, `rotate-if-needed --cooldown`
  0..=86_400.

### Tests

- **N5 PSK round-trip (`tests/nm_get_settings_roundtrip.rs`)** — test-local
  `MockNmConnectionSettings` shim pins that
  `802-11-wireless-security.psk` and `802-1x.password` /
  `802-1x.private-key-password` survive an unrelated `ssid`
  mutation through the production `nm::merge_secrets` +
  `nm::SECRET_SECTIONS` path. Negative control demonstrates the
  mock correctly models NM's wipe-on-absent-key behaviour.
- **CL4 scenario sweep** — twelve new `tests/integration/scenarios/`
  files cover `session`, `diff`, `dry-run`, every component status
  reader, every component apply/revert `--yes` gate, `persona`
  cli, `ssid` cli, `wiki search`, top-level `help`, `completions`,
  `kill`/`resume`, `timer` set/reset/logs, `config` set-profile,
  `probe`, and `events run`. Runner auto-discovers via
  `scenarios/*.sh` glob.
- **C6 mock-flock honesty** — `MockBackend::with_state_path(p)`
  opt-in flock so unit tests can pin the
  foreign-fd-flock-busy → `SkippedCooldown { remaining: 1s }`
  contract production NM uses.

### Code quality

- **Central iface validator full migration (GH#359 follow-up)** —
  all six per-module duplicates now delegate to `crate::iface`:
  `mac::factory`, `ipv6`, `rf`, `kill_switch`,
  `events::source::nm_connection_up`, and the nested
  `mac::probe::raw`. Three wrappers were strictly laxer than the
  central kernel-faithful validator and are now newly-strict on
  input that bypassed them before — defence-in-depth on callsites
  that already feed kernel-validated sysfs walks.
- **NMOD.4 const-assert** — `OWNER_POOL` non-empty check before
  indexing in `persona::template::pick_owner`.
- **E5 partial** — surveyed `apply.rs`, `show_config.rs`,
  `doctor.rs`, `config_cmd.rs`. The exact `eprintln+drop+
  Ok(GENERIC_ERROR)` pattern doesn't appear; every `if let Err(e)`
  arm uses a typed exit code that bubbling via `?` would change.
  Converted one site (`config_cmd::edit`) plus three breadcrumb
  comments. Full typed-error refactor deferred to v0.5+.

### Dependencies

- **tokio 1.52.2 → 1.52.3** (cargo patch bump).
- **toml_edit 0.22.27 → 0.25.11+spec-1.1.0** (compat bump).
- **actions-rust-lang/setup-rust-toolchain 1.16.0 → 1.16.1**
  (GH Actions patch).
- **softprops/action-gh-release 2.6.2 → 3.0.0** (release workflow).
- **actions/upload-artifact 4.6.2 → 7.0.1** (release workflow).
- Closed without merge (require source migration): `toml 0.8 → 1.x`
  (top-level API gone), `getrandom 0.2 → 0.4` (top-level
  `getrandom()` removed).

### Docs

- **Stream 10 wiki-hint sweep** — ~77 `; see proteus wiki <page>`
  hints appended to operator-facing `bail!` / `anyhow!` sites
  across `src/persona/`, `src/init/`, `src/backend/`,
  `src/enterprise_wifi/`, `src/hostname/`, `src/commands/{rotate,
  pin,config_cmd,dns,ntp,resolved,stack,enterprise_wifi,timer,
  apply,watch,mod}.rs`. Internal/defensive errors skipped.
- **`wiki/polkit-hardening.md` new** — documents the optional
  unix-group polkit JS recipe and the `pkcheck` runtime check.
- **`docs/ROADMAP.md` substantive update** — sync ⏳ → ✅ for every
  landed item with status notes referencing the PR that landed it.
- **`contrib/recovery-kit/`** — codex-authored backup/restore
  sidecar scripts (`backup.sh`, `restore.sh`, `README.md`) with
  deterministic tarball flags, `flock`-based concurrency, audit
  JSON for rollback, optional encryption via gpg/age. Followups
  fixed tar exit-status handling and stderr-into-listing
  contamination (codex P1 reviews).

## [0.4.2-beta] - 2026-05-08

Second v0.4 beta. Closes the remainder of the May 2026 audit tree plus
the audit follow-up findings carried over from
`docs/security/SECURITY-AUDIT-2026-05-07-followup.md`. Adds a
professional clean-and-polish pass over the entire repo (wiki, README,
SECURITY, CONTRIBUTING, dist READMEs, code comments, dead code).

### Security

- **Persona export safety parity** (#286) — `proteus persona export`
  requires `--yes`, refuses to overwrite an existing regular file unless
  `--force` is set, lstat-rejects symlink destinations even with
  `--force`, writes via `write_atomic` (0o600 + parent fsync) — full
  parity with `import`.
- **Quarantine preserves originals** (#290) — entering quarantine no
  longer destroys cached `originals.hostname` / `originals.bluetooth_alias`,
  so `proteus revert` correctly restores the actual originals (not the
  rotated quarantine values).
- **`PROTEUS_*_DIR` env-var lockdown** (audit M-2 / N-0) — `Layout::from_env()`
  in `commands/uninstall.rs` no longer reads env vars in production
  builds; the `cfg(test)` gate keeps the test override path. Hostile
  sudo-preserved env can no longer steer `remove_dir_all`.
- **Iface validation on ethtool / iw / ip** (audit N-1, L-3) — iface
  names validated against `[A-Za-z0-9_.-]+` (≤ 15 chars, no leading
  dash) before any subprocess invocation. `--` separator inserted before
  every user-influenced positional arg in `iw` / `ip` calls (defense in
  depth against future `allow_hyphen_values` flips).
- **`Mac::from_str` fast-fail on `proteus pin`** (#292) — `--mac` value
  is parsed up-front and rejected with a clear `MacError`-derived
  message before any DBus / NM connection is attempted.
- **`proteus timer` --yes gate + interval bounds** (#293) — `set` /
  `reset` / `enable` / `disable` honor the project-wide `--yes` gate.
  Interval bounds: ≥ 60s (sub-minute risks rotation stacking) and ≤ 30
  days (almost certainly user error). Bounds live as `MIN/MAX_TIMER_INTERVAL_SECONDS`
  in `src/timer/mod.rs`.
- **Cross-layer persona consistency** (#305) — brand-named stealth
  personas (iphone-15, pixel-8, macbook-pro-m3, …) extended with DHCP
  option 55 (parameter request list), mDNS records, and TCP fingerprint
  hints distinct per OS family. Single-pass cross-layer classifier no
  longer identifies "Proteus user with persona X" via the L2/opt-60 vs
  rest-of-stack mismatch.
- **Randomized rotation cadence** (#303) — default `OnCalendar` widened
  with `AccuracySec` ≥ 30min (and per-host random offset where
  applicable) so the v0.3.x recognizable "every-2h on the wallclock
  hour ±5min" Proteus signature is broken at the WLAN-controller layer.

### Bug fixes

- `proteus enterprise-wifi disable` restores cached `anonymous-identity`
  instead of clearing to `""`; `proteus revert` now also reverts
  enterprise-wifi changes (#298).
- `--config <missing-path>` accepted by `persona use` and `ssid set`
  (their write paths handle missing files; the rejection was a stale
  guard) (#302).
- `config reset` writes a near-empty file (header comment only) rather
  than the full set of resolved defaults — defaults stay in code, the
  file carries only user overrides (#304).
- `status` `feature_table` reports `probes` / `discovery-silence` /
  `rf-tx-power` correctly as implemented (#306).

### Code quality

- **SHA-256 deduplicated** (#299) — single canonical implementation in
  `src/crypto/sha256.rs`; deleted four near-identical copies in dns,
  ipv6, stack, diff. No new external dep.
- **Completions regenerated** (#285, #291) — bash, zsh, fish completions
  now reflect the current ~50-subcommand surface and the embedded wiki
  page list. Test asserts the bundled files contain a representative
  cross-section so this can't drift again silently.
- **Packaging**: RPM `%systemd_post` now enables `proteus-boot.service`
  alongside the timers and `proteus-resume.service` (#279). Dropped
  `dist/debian/compat` — it clashed with `debhelper-compat (= 13)` in
  Build-Depends and broke the deb pipeline.

### Docs

- Professional clean-and-polish pass (#335) — wiki, README,
  CONTRIBUTING, SECURITY, dist READMEs, docs/, dist/man/proteus.1.
  Stale phase markers (B/C/D/E/F) removed throughout. Cross-references
  verified, broken links fixed.
- May 2026 security audit doc + followup brought onto main under
  `docs/security/` with a status table mapping each finding to the
  fixing PR or status (#320).

### Polish

- Surgical polish pass (#333) — dead code removed (`commands/ipv6::apply_nm_one`
  orphan, `commands/ssid::_expose_per_ssid_policy_type` unused-import
  shim), stale phase markers in code comments trimmed, packaging
  recipe descriptions normalized, `rotate-if-needed` short-help fixed
  to fit one line in `--help`.

## [0.4.0-beta1] - 2026-05-08

First v0.4 beta. The "Reach + Persona" cycle closed in v0.3.x; v0.4 is bug
+ vulnerability hunting only, no new features. This release lands the May
2026 vulnerability hunt cluster (30+ issues) plus three critical-for-beta
fixes (#276 packaging version sync, #284 Mac::from_str panic, #297 timer
set newline injection).

### Security

- **Output sanitization** (#241, #234, #238) — captive-portal `Location:`
  header sanitized through a shared `display_string` helper; `# sha256:`
  framing reworded from "verification" to "edit detection"; polkit policy
  framing clarified as a GUI hint, not a binary-side authorization gate.
- **PATH hardening** (#239) — `main()` resets `$PATH` to a known-good list
  before any subcommand dispatch.
- **systemd hardening parity** (#228) — `proteus-rotate`, `proteus-resume`,
  `proteus-boot`, `proteus-check` services now carry the strict-protect
  shape that `proteus-events.service` had (`ProtectSystem=strict`,
  `ProtectKernel*`, `RestrictAddressFamilies`, `MemoryDenyWriteExecute`,
  `LockPersonality`, `SystemCallErrorNumber=EPERM`, etc).
- **State-dir + lock-file perms** (#275) — explicit chmod 0o700 / 0o600 on
  create regardless of umask; install.sh chmods existing dirs.
- **NM dispatcher hardening** (#225) — `--` separator, value validation,
  stderr captured to journal instead of /dev/null.
- **DBus error visibility** (#248) — replaced `unwrap_or_default()` on NM /
  Bluetooth property reads with skip-and-log instead of ghost devices.
- **Event-trigger rate limit + bounded FlapTable** (#254) — per-kind
  sliding-window limiter + LRU eviction for FlapTable.
- **Persona import TOCTOU** (#231) — single-read of source bytes.
- **Persona schema validation** (#266, #232, #253, #255) —
  `deny_unknown_fields` on Persona structs, full schema_check coverage,
  schema_check run during load_user / load_builtin / list_all.
- **Modulo-bias-free random pickers** (#226) — lifted `unbiased_index` to
  shared `src/rand/mod.rs`, adopted at every MAC / hostname / persona
  picker.
- **Config + SSID validation** (#227, #257) — `deny_unknown_fields` and
  range validation on Raw* structs; `ssid set` validates pin_mac /
  rotate_interval / persona / portal_policy.
- **Mac::from_str UTF-8 panic** (#284) — non-ASCII input that lands at 12
  cleaned bytes no longer panics; rejects cleanly.
- **Timer set unit-grammar injection** (#297) — `parse_interval` now
  rejects control chars, `[`, `]`, `;`, `#`, and caps length at 200 bytes
  before classifying as Calendar.

### Bug fixes

- Backend `available()` honesty for networkd/raw (#247).
- `rotate_if_needed` reports proper read-back errors instead of all-zero
  MAC (#250).
- Cooldown / rotate TOCTOU closed by holding state-lock across the
  decision and rotate (#245).
- RF revert preserves originals on partial failure (#269).
- Factory MAC readers reject multicast addresses (#271).
- `parse_duration` UTF-8-safe — no panic on `5µ` (#272).
- `dmesg_firmware_line` and `parse_iw_phy_capabilities` allocations
  hoisted out of hot loops (#273).
- ARP / IPv6 ND probes implemented (#267).
- Events daemon `--max-triggers` honored (#259, #262).
- `is_auth_edge` handles `PortalRequired → Unknown → Clear` (#261).
- `EventRegistry::register` recovers from poisoned mutex (#252).
- Per-watcher graceful shutdown for nm-connection-up (#256).
- Link-flap and reg-domain netlink sources implemented with kernel-origin
  validation (#251).
- `--yes` flag honored on bluetooth/hostname/dns/resolved/ntp apply+revert
  (#242).
- `version::PHASE` bumped from 'B' to 'G' (#249).
- All third-party GitHub Actions SHA-pinned (#260).
- `tests/realworld/probe.sh` IPv4 / IPv6 anonymisation regex fixes (#263,
  #264).
- `commands::uninstall::revert_best_effort` deduplicated; uninstall now
  delegates to `commands::revert::revert_best_effort` (#265).
- Persona `edit` HOME warning + $VISUAL / DEFAULT_EDITOR fallback (#230,
  #244).
- **Packaging** version strings synced to Cargo.toml (#276) — Arch
  PKGBUILD, RPM spec, Debian changelog all bumped to 0.4.0-beta1 so
  release pipeline produces correctly-labeled artifacts.

### CI

- 4 root-context test failures fixed (#274) — kill / events permission
  tests now skip when EUID=0, matching the existing dhcp pattern. Restored
  zero-warnings clippy build.

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
