# Roadmap — v0.4.x "Hardening Across Streams" (active)

Three bug-hunt sessions across two branches produced a combined 141-item
issue log captured in [`docs/ISSUES.md`](ISSUES.md): the original 85-item
sweep, a 20-item second pass, and a 36-item third-pass parallel sweep
(Section 13) brought forward from `claude/find-hidden-bugs-RbE2H`. The log
spans security, panic potential, concurrency, config validation, build /
packaging, error handling, docs drift, CLI dispatch, network backends,
performance, mac internals, commands orchestration, subsystems, events /
captive portal, packaging, backend / NM integration, and tests / examples /
data.

The first v0.4 beta (`v0.4.0-beta1`) shipped about 30 of those items. That
leaves ~111 unfixed — including 4 critical CLI-confirmation bypasses that
mutate state without `--yes`, 24 high-severity items (3 added by Section 13:
NM2.1, NMOD.1, NTEST.1), and a long tail of medium / low findings. This
roadmap organises the remaining work into ten **parallelisable streams**,
each scoped to a distinct module or file area so multiple contributors can
land changes simultaneously without merge conflict.

Versioning stays inside `0.4.x-beta`. The project does **not** advance to
`0.5.x` until every High and Critical row in `docs/ISSUES.md` has a non-empty
"fixed-in" cell within `0.4.x-beta`. New features are out of scope for the
entire cycle.

For per-version release notes see [`CHANGELOG.md`](../CHANGELOG.md). For prior
cycles see [`ROADMAP-v0.3.md`](ROADMAP-v0.3.md) (Reach + Persona, shipped) and
[`ROADMAP-v0.1.md`](ROADMAP-v0.1.md) (Phases A–G, archived). For design
rationale see [`PLAN.md`](PLAN.md). For how to help see
[`CONTRIBUTING.md`](../CONTRIBUTING.md).

## Source-of-truth migration (read this first)

This roadmap is the **single source of truth** for every unresolved problem
reported anywhere in the repo. Findings that historically lived in the two
security-audit documents have been absorbed into the streams below. The audit
documents are now archived as historical records; **stop tracking status against
them**. The mapping is explicit so nothing is lost:

| Audit ID | Origin doc | Severity (current) | Absorbed into |
|---|---|---|---|
| M‑2 / N‑0 | `SECURITY-AUDIT-2026-05-07.md`, `-followup.md` | High (re-classified) | Stream 9 — `PROTEUS_*_DIR` env hardening |
| L‑3 (residual) | `SECURITY-AUDIT-2026-05-07.md` | Low | Stream 9 — `--` separator on `iw`/`ip` positional args |
| N‑1 | `SECURITY-AUDIT-2026-05-07-followup.md` | Low | Stream 4 — `ethtool -P` iface validation in `mac/factory.rs` |
| N‑3 (residual) | `SECURITY-AUDIT-2026-05-07-followup.md` | Low | Stream 5 — `O_NOFOLLOW` on `state_lock` open |
| I‑1 | `SECURITY-AUDIT-2026-05-07.md` | Info | Stream 9 — SHA-256 consolidation into `crate::hash` |
| I‑2 | `SECURITY-AUDIT-2026-05-07.md` | Info | Stream 3 — `cargo audit` in release CI |

The closed audit findings (H-1, H-2, M-1, M-3, L-1, L-2, L-4, N-2, T-1) stay in
the archived audit files as the historical fix record; they need no roadmap
entry because they are already on `main`.

### Section 13 absorption (third-pass findings)

The 36 Section 13 entries land in the streams below; the table reproduces the
mapping for fast lookup. None of these were addressed by the current
`v0.4.x-beta` work-in-progress — every row is `⏳`.

| ID | Severity | Stream | Sub-area |
|---|---|---|---|
| NM2.1 | high | 4 | mac generator livelock on single-token pool |
| NM2.5 | low | 4 | `generate_for_vendor` postcondition doc |
| NM2.7 | low | 1 | `mac::plan::preview_mac` ignores OUI pool |
| NCMD2.1 | medium | 1 | `proteus doctor` exit code conflates Warn + Fail |
| NCMD2.3 | medium | 4 | `apply` skips `daemon-reload` on dns/stack/ipv6/resolved |
| NCMD2.4 | medium | 4 | revert mis-targets deleted / recycled NM uuids |
| NCMD2.5 | low | 1 | `status --json` outputs U+FFFD on non-UTF-8 ifaces |
| NSUB.1 | medium | 4 | `[stack]` apply silently no-ops kernel-unsupported keys |
| NSUB.2 | medium | 4 | `[stack]` revert leaves orphaned hardened sysctls |
| NEV2.1 | medium | 9 | `wiki::get_page` lacks input validation |
| NEV2.2 | medium | 4 | captive portal misclassifies empty-body 200 as Clear |
| NEV2.3 | medium | 5 | hostname DBus calls have no timeout |
| NEV2.4 | low | 7 | bluetooth adapter-disappeared logs `error!` not `warn!` |
| NEV2.5 | low | 4 | bluetooth name cap doesn't differentiate BR/EDR vs BLE |
| NEV2.7 | low | 1 | dispatcher rc=70 conflates "not supported" reasons |
| NPKG.3 | medium | 3 | `build.rs` panics fatally on EACCES |
| NPKG.4 | low | 3 | `cargo:rerun-if-changed` not per-file |
| NPKG.6 | low | 3 | Debian compat pinned at 13 |
| NPKG.7 | medium | 3 | Alpine APKBUILD `sha512sums="SKIP"` (supply chain) |
| NPKG.8 | medium | 3 | Void template `checksum=SKIP` (supply chain) |
| NPKG.9 | low | 3 | RPM `%check` bypassable via `--without check` |
| NPKG.13 | info | 10 | `proteus-rotate.timer` ±75 min jitter (intentional) |
| NPKG.14 | low | 3 | `install.sh` polkit `sed` rewrite uses unquoted var |
| NBE.1 | medium | 8 | every backend method opens fresh `zbus::Connection` |
| NBE.2 | info | 8 | `select_auto` + `availability_matrix` double-probe |
| NBE.3 | medium | 4 | DHCP DUID/IAID asymmetry on rotate |
| NBE.4 | low | 4 | `suppress_vendor_class` not sticky across persona |
| NBE.5 | low | 4 | `802-1x.private-key-password` round-trip test gap |
| NBE.6 | low | 5 | `MockBackend::set_cloned_mac` skips MAC validation |
| NBE.7 | low | 4 | NM `Reapply(empty,0,0)` racy under concurrent edits |
| NBE.8 | low | 4 | backend device path cached across reconfiguration |
| NBE.10 | low | 4 | `ethtool -P` parser breaks on Linux 6.3+ output variants |
| NMOD.1 | high | 5 | `apply::run` acquires state lock before validating config |
| NMOD.2 | medium | 5 | `apply` `--yes` gate fires before config validation |
| NMOD.3 | medium | 6 | `diff` skips files Proteus once-managed but operator deleted |
| NMOD.4 | info | 2 | `pick_owner` indexes `OWNER_POOL` without non-empty guard |
| NTEST.1 | high | 2 | persona `lg-tv-2023.toml` `hostname_template` unusable; `schema_check` doesn't render through `validate_hostname` |
| NTEST.2 | medium | 4 | `persona-effectiveness.sh` fixed `sleep 5` is flaky on slow CI |
| NTEST.3 | low | 6 | `tests/realworld/probe.sh` assumes `/sys/class/net` exists |

**Issue-log convention.** `docs/ISSUES.md` is also archived now — Section 13
(brought forward from `claude/find-hidden-bugs-RbE2H`) brings the cumulative
total to 141 findings, and every High and Critical row is referenced by some
stream below. The roadmap is the single live tracker. The roadmap references
issue-log IDs directly:

- Sections 1–12 IDs: `S*`, `P*`, `C*`, `V*`, `B*`, `E*`, `D*`, `CL*`,
  `N1`–`N14`, `R*`, `M1`–`M5`, `N12.*`.
- Section 13 IDs: `NM2.*`, `NCMD2.*`, `NSUB.*`, `NEV2.*`, `NPKG.*`, `NBE.*`,
  `NMOD.*`, `NTEST.*`.
- Audit IDs (hyphenated, from the archived security audits): `M‑2`, `N‑0`,
  `N‑1`, `L‑3`, `N‑3`, `I‑1`, `I‑2`.

The hyphen distinguishes audit IDs from issue-log IDs (e.g. `M-2` is from the
audit, `M2` would be from the issue log).

**Ops checklist.** `docs/MAINTAINER-FOLLOWUPS.md` is **not** absorbed by this
roadmap — it tracks one-shot maintainer chores (tag pushes, stale-branch
deletion, draft-release publication) that are operational, not findings. That
file self-deletes when the listed actions are executed; it is intentionally
out of scope here.

## Status legend

- ✅ Landed (in `main`)
- 🚧 In progress (PR open)
- ⏳ Planned (next up)
- 💭 Deferred (in scope but not soon)

## Parallel stream map

The streams are partitioned by **file area** so they merge cleanly when run in
parallel. Streams marked **independent** touch disjoint files and can land in
any order. Streams marked **light-coupling** share one or two files with a
sibling stream and need a brief sequencing decision (noted per-stream).

Severity counts include all absorbed Section 13 / audit findings. Issue lists
in each stream-detail block are authoritative; this table is a quick read-out.

| # | Stream | Severity mix (post-absorb) | Coupling | Status |
|---|---|---|---|---|
| 1 | CLI Safety & Confirmation Gates | 4 critical · 1 high · 7 med · 6 low | independent | ⏳ |
| 2 | Config Schema Validation | 6 high · 5 med · 6 low · 2 info | independent | ⏳ |
| 3 | Packaging & Build / CI Coherence | 6 high · 9 med · 10 low · 1 info | independent | ⏳ |
| 4 | Events Daemon & Network Backends | 3 high · 14 med · 13 low | light (shares `src/commands/events.rs` with Stream 7) | ⏳ |
| 5 | State Lock & Concurrency | 3 high · 5 med · 8 low | light (shares `src/state.rs` with Stream 9) | ⏳ |
| 6 | Panic Hardening | 2 high · 4 med · 3 low | independent | ⏳ |
| 7 | Error Handling & Logging Discipline | 8 med · 3 low | light (shares `src/commands/dhcp.rs` with Stream 8) | ⏳ |
| 8 | Resource Hygiene & Performance | 1 med-high · 3 med · 5 low · 3 info | light (DHCP file shared with Stream 7) | ⏳ |
| 9 | Security Surface Hardening | 6 med · 4 low · 1 info | light (polkit shared with Stream 3) | ⏳ |
| 10 | Docs / Wiki / Examples Drift + ⏳ frontier items | 4 high · 2 med · 2 low · 2 info | independent | ⏳ |

**First wave (zero file overlap; ship in parallel from day one):** Streams 1, 2, 3, 6, 10.

**Second wave (after first-wave merges land; each shares one or two files with a first-wave stream):** Streams 4, 5, 7, 8, 9.

## Suggested versioning

Flexible — the constraint is "stays under `0.4.x-beta`," not a specific release
schedule. One reasonable mapping:

| Version | Streams that ship |
|---|---|
| `v0.4.3-beta` | 1 + 3 + 6 (CLI safety + packaging + panic hardening — all independent, all high-impact) |
| `v0.4.4-beta` | 2 + 10 (config validation + docs / examples drift — both independent of v0.4.3) |
| `v0.4.5-beta` | 4 + 5 (events / backends + state lock — second wave, light-coupled) |
| `v0.4.6-beta` | 7 + 8 + 9 (logging discipline + perf + security surface) |
| `v0.4.7-beta` | overflow + ⏳ real-world testing fixes + independent review responses |

If a critical regression surfaces mid-cycle, cut a `v0.4.x.Y-beta` patch
out-of-cycle as the existing release flow already permits.

## Stream details

### Stream 1 — CLI Safety & Confirmation Gates ⏳

**Why critical:** four mutators run state-changing operations without honouring
`--yes`. Wrapping scripts that depend on the confirmation contract are silently
broken. This is the single highest-impact unfixed cluster.

**Files:** `src/cli/dispatch.rs`, `src/cli/actions.rs`, `src/cli/command.rs`,
`src/commands/watch.rs`, `src/commands/portal.rs`, `src/commands/dhcp.rs`
(apply / revert entry points only — see Stream 7 note),
`tests/integration/scenarios/` (new files only).

**Issues:** CL1, CL2, CL3, CL4, CL5, CL6, CL7, M1, N12.1, N12.2, N12.3,
NM2.7, NCMD2.1, NCMD2.5, NEV2.7.

**Work:**

- ⏳ Add `--yes` field to `unpin` action (N12.1) and to every mutator action
  that declares a `yes: bool` field today (CL3 — currently dead code).
- ⏳ Wire `--yes` through dispatch for Bluetooth / Hostname / DNS / Resolved /
  NTP / Portal (CL2), DHCP apply / revert (M1, N12.2), Portal mark / unmark /
  open (N12.3).
- ⏳ Reject `--interval 0s` in watch mode (CL1) and `<1ms` sleep granularities
  (CL7).
- ⏳ Add integration scenarios for the 24 untested subcommands (CL4).
- ⏳ Add `--json` flag to `resume` and `wiki` (non-search) for parity (CL6).
- ⏳ Document the prefix-collision risk in CLI changelog (CL5).
- ⏳ `proteus rotate --dry-run` preview: thread the configured OUI pool into
  `mac::plan::preview_mac` so the previewed MAC reflects the persona, not a
  hardcoded LAA placeholder (NM2.7).
- ⏳ `proteus doctor` exit code: split warn-only vs fail; reserve exit 1 for
  `fail > 0`, map warn-only to exit 0 (or a distinct code) so CI wrappers
  don't block on warnings (NCMD2.1).
- ⏳ `proteus status --json`: skip non-UTF-8 sysfs entries with a `debug!`
  line, or include them with explicit `valid_utf8: false` (NCMD2.5).
- ⏳ NM dispatcher `rc=70` branch: log captured stderr at `info!` (or split
  the exit code) so "backend unavailable" doesn't mask "missing nft / nl80211
  / CAP_NET_ADMIN" (NEV2.7).

**Acceptance:** end-to-end script that calls every mutator without `--yes` and
asserts exit code 64 (`CONFIRMATION_REQUIRED`); run watch with `--interval 0s`
and assert exit 64 (rejection) instead of CPU burn.

### Stream 2 — Config Schema Validation ⏳

**Why high-impact:** silent acceptance of nonsense config (zero-interval
rotates, unknown profile names, multibyte panic in duration parser) gives users
a posture they did not ask for.

**Files:** `src/config.rs`, `src/per_ssid.rs`, `src/persona/load.rs`.

**Issues:** V1–V12, N12.4, N12.5, N12.12, P1, P7, NMOD.4, NTEST.1.

**Work:**

- ⏳ Reject zero / empty rotation intervals at config-load time (V1).
- ⏳ Validate profile names and persona IDs against the known catalogue at
  load time (V2, V6) with closest-match suggestions.
- ⏳ Validate `quorum_n <= quorum_total` (V3).
- ⏳ Bound second-precision durations (V4); bound `tx_power_reduction_db` (V5).
- ⏳ Validate `pin_mac` format at load (V7); validate persona OUI pool (V11).
- ⏳ Fix `parse_duration` overflow (N12.4) and the multibyte-trailing-char
  panic in `is_valid_per_ssid_duration` (N12.5) — ship together.
- ⏳ Constrain `clap` `u32` flags to a sane range (N12.12).
- ⏳ Replace `split_at` on potentially-empty duration strings (P1) and the
  `get_mut("mac").unwrap()` test path (P7).
- ⏳ Round-trip test coverage expansion for arrays, numerics, enums (V10).
- ⏳ SSID-key TOML-special-character coverage (V12).
- ⏳ Persona `schema_check` now also renders the template through
  `validate_hostname` (and equivalents) so unusable personas like
  `lg-tv-2023` are caught at load (NTEST.1, **High**). Fix the existing
  `data/personas/lg-tv-2023.toml` template (`[LG]_webOS_TV_{word}` →
  `lg-webos-tv-{word}` or similar) and add a regression test that exercises
  every shipped persona's `hostname_template` through the validator.
- ⏳ Defensive guard / assert that `OWNER_POOL` is non-empty before indexing
  in `persona::template::pick_owner` (NMOD.4).

**Acceptance:** new test module `config::validation_tests` loads each malformed
example and asserts the specific error variant; full `cargo test --release`
passes (catches `panic = abort` regressions immediately).

### Stream 3 — Packaging & Build / CI Coherence ⏳

**Why high-impact:** `proteus-events.service` ships disabled or wrong-pathed in
**every** package (RPM, Debian, Gentoo, Alpine, install.sh). The events daemon
is unreachable on every install path that is not `cargo install`.

**Files:** `install.sh`, `uninstall.sh`, `dist/rpm/proteus.spec`,
`dist/debian/rules`, `dist/gentoo/proteus-0.1.0.ebuild`,
`dist/alpine/APKBUILD`, `dist/void/template`,
`dist/systemd/proteus-events.service`,
`dist/networkmanager/dispatcher.d/01-proteus`, `.github/workflows/ci.yml`,
`.github/workflows/release.yml`, `Cargo.toml` (description-length only),
`build.rs`, `scripts/check.sh`.

**Issues:** B1–B15, N12.8, N12.9, M4, audit I‑2, NPKG.3, NPKG.4, NPKG.6,
NPKG.7, NPKG.8, NPKG.9, NPKG.14.

**Work:**

- ⏳ Wire `proteus-events.service` into install.sh `enable / start` ladder
  (B1); add `%post` / `%preun` hooks in RPM (B2), `dh_installsystemd` in
  Debian (B3), `systemd_dounit` in Gentoo (B4), Alpine post-install
  trigger (B5).
- ⏳ Reconcile `/usr/local/bin` (install.sh) vs `/usr/bin` (distro
  packages) so the unit's `ExecStart=` resolves on every path (B10, N12.8).
- ⏳ Add `KillMode=mixed` and `TimeoutStopSec=10s` to
  `proteus-events.service` (N12.9).
- ⏳ POSIX-ify NM dispatcher shebang (B6); validate install.sh with `sh -n`
  (B13).
- ⏳ Add top-level `permissions:` block to `ci.yml` (B7).
- ⏳ Pin `softprops/action-gh-release@v2` to commit SHA (B9).
- ⏳ Add `--locked` to Alpine / Void / Gentoo cargo invocations (B8).
- ⏳ Wire a real `[features]` table in `Cargo.toml` to back the Gentoo USE
  flags (B14); restrict polkit policy (B15 — coordinate with Stream 9).
- ⏳ Replace `build.rs::panic!` with actionable errors (B12); add `:?`-guard
  pattern to `uninstall.sh` (B11); shorten `Cargo.toml` description below
  256 chars (M4).
- ⏳ Wire `cargo audit` into the release workflow with `Cargo.lock` as the
  target; fail the release on any open advisory in `zbus`, `clap`, `tokio`,
  `toml`, `toml_edit`, `serde`, `tracing`, `getrandom` (audit I‑2).
- ⏳ Populate `sha512sums` in `dist/alpine/APKBUILD` (NPKG.7) and `checksum`
  in `dist/void/template` (NPKG.8); add a guard rejecting the literal
  `"SKIP"` placeholder so an early packager-build cannot ship without
  integrity validation. **Supply-chain gap.**
- ⏳ Replace `build.rs` `panic!` on wiki-file read errors with an actionable
  `expect("…")` message and `cargo:warning=` line (NPKG.3); emit
  `cargo:rerun-if-changed` per file rather than per directory so deletions
  invalidate correctly (NPKG.4).
- ⏳ Bump `dist/debian/control` `debhelper-compat` to 14 once `release.yml`
  passes (NPKG.6).
- ⏳ Document the `rpmbuild --without check` bypass risk in
  `dist/rpm/README.md` (NPKG.9); add a `%global _without_check 0` guard.
- ⏳ Fix `install.sh` polkit `sed` rewrite: use a defensive delimiter
  (e.g. `#`) and pin the annotate string in a `read -r` heredoc (NPKG.14).

**Acceptance:** spin up Fedora 43, Debian 13, Gentoo, Alpine 3.20 in CI
containers; `install` the package; assert `systemctl is-enabled
proteus-events.service` returns `enabled` and `systemctl status` returns
`active`.

### Stream 4 — Events Daemon & Network Backends ⏳

**Why high-impact:** N1 means the events daemon documented to "rotate on
trigger" only logs the trigger — the rotation never happens. Combined with
N12.11 (per-SSID stub returns `None`), large parts of the v0.3 event-driven
feature are non-functional.

**Files:**
`src/events/source/{nm_connection_up,link_flap,reg_domain,portal_auth}.rs`,
`src/events/mod.rs`, `src/commands/events.rs` (sequence Stream 4 first;
Stream 7's logging changes rebase cleanly), `src/nm/mod.rs`,
`src/nm/apply.rs`, `src/backend/nm.rs`, `src/backend/mock.rs`,
`src/captive_portal/mod.rs`, `src/mac/factory.rs`.

**Issues:** N1–N14, N12.7, N12.11, N12.19, R4, audit N‑1, NM2.1, NM2.5,
NCMD2.3, NCMD2.4, NSUB.1, NSUB.2, NEV2.2, NEV2.5, NBE.3, NBE.4, NBE.5,
NBE.7, NBE.8, NBE.10, NTEST.2.

**Work:**

- ⏳ Make `RotateOnTriggerHandler` actually rotate (N1) — the most important
  single fix in the whole roadmap; closes a documented-but-broken security
  feature.
- ⏳ Fix `factory::permanent_address` `Option` → `Result` (N2, N12.19) so
  I/O failure is distinguishable from "no factory MAC".
- ⏳ Validate the `iface` argument before `EthtoolBin::permanent` calls
  `ethtool -P <iface>` (audit N‑1) — reuse the existing
  `crate::ipv6::validate_iface_name` helper or lift a shared
  `crate::mac::iface::validate` to match the `is_safe_iface` posture L‑3
  established for `iw` / `ip`.
- ⏳ Probe NM DBus interface version (N3); preserve method / path on zbus
  errors (N4); fix connection lookup id / uuid mixing (N6).
- ⏳ Implement per-trigger debounce on link-flap detector (N8); subscribe to
  `DeviceAdded` (N12).
- ⏳ Captive portal: validate TLS, follow redirects (N9); fix `Host:`
  header for IPv6 literals (N12.7); reorder `to_socket_addrs` to v4-first
  (N10).
- ⏳ Reload captive-portal config on `SIGHUP` (R4); per-SSID stub returns the
  real SSID (N12.11).
- ⏳ Test coverage: full `GetSettings → Update` with PSK round-trip (N5);
  factory MAC fallback failure path (N7); mock-backend mutex-poisoning
  recovery (N13).
- ⏳ Init-system detection paths beyond hardcoded list (N11).
- ⏳ Per-SSID policy debounce vs concurrent CLI rotate (N14).
- ⏳ Drop the `&& opts.pool.len() > 1` guard in
  `mac::generator::generate_with_probe` so single-token persona pools (e.g.
  `oui_pool = ["apple"]`) reset `consecutive_collisions` on every retry
  rather than running out the 64-attempt budget on the same OUI (NM2.1,
  **High**). Stalls events daemon under sustained collision conditions.
- ⏳ Add a doc comment to `generate_for_vendor` stating the caller must
  validate the returned MAC; today both callers do, but the postcondition is
  undocumented (NM2.5).
- ⏳ Run a single `systemctl daemon-reload` at the end of `apply::run()`
  after dns / stack / resolved / ipv6 drop-ins are written, so the
  documented effect actually materializes without a manual reload (NCMD2.3).
- ⏳ Validate cached NM uuids against the live `Settings.ListConnections`
  before invoking restore in `revert`; drop missing uuids with `warn!`,
  reject restore when uuid is present but the SSID/id has changed (NCMD2.4
  — guards against NM uuid recycling silently corrupting an unrelated
  profile). Builds on the N6 fix.
- ⏳ Surface `warn!` when `read_sysctl(key)` returns `None` during `[stack]`
  apply (kernel doesn't expose the key) and skip writing the drop-in
  (NSUB.1). At revert time, re-probe each cached key and restore only those
  that exist now; log orphans at `info!` (NSUB.2).
- ⏳ Captive portal classifier: reject `expected_response = ""` at config
  load with a wiki-linked error, or treat empty `expected_body` paired with
  empty body as `Unknown` rather than `Clear` (NEV2.2).
- ⏳ Bluetooth name length: query adapter capabilities and cap BLE-only
  adapters at ~30 bytes; `warn!` if the configured alias would be truncated
  by the controller (NEV2.5).
- ⏳ DHCP DUID/IAID asymmetry on rotate: document the tradeoff and add a
  config knob to keep IAID persistent (or pin DUID + IAID derivation so the
  asymmetry goes away). Today rotation breaks DHCPv6-only networks (NBE.3).
- ⏳ Honor `[dhcp] suppress_vendor_class = true` over the persona's
  `vendor_class_identifier` write — user suppression should win (NBE.4).
- ⏳ Extend the existing enterprise-Wi-Fi mock test to round-trip a
  connection with `private-key-password` set; assert it's preserved
  post-rotate (NBE.5).
- ⏳ NM `Reapply` race: read the connection's current `version_id` and pass
  it to `Reapply` so concurrent `nmcli connection modify` surfaces as a DBus
  conflict instead of a silent stale-write (NBE.7).
- ⏳ Resolve backend devices by interface name at every method call rather
  than caching the NM Device object path at enumeration time (NBE.8).
- ⏳ `ethtool -P` parser: match against both `permanent address:` and
  `permanent mac address:` (Linux 6.3+ Intel iwlwifi variant). Add a
  fixture-based test (NBE.10) — relevant because incorrect parsing means
  factory-MAC capture silently falls back to the live address.
- ⏳ Replace `persona-effectiveness.sh`'s fixed `sleep 5` with a poll-until
  loop on `proteus current --json` (MAC + DHCP lease timestamp), with a
  generous timeout, so slow CI runners don't conflate baseline and persona
  variants (NTEST.2).

**Acceptance:** new test scenario `events_rotate_actually_rotates.rs` that
spins the events daemon, fires a `ConnectionUp` signal, and asserts the
backend mock saw a `set_cloned_mac` call afterwards. Today this test would
fail — that is the bug.

### Stream 5 — State Lock & Concurrency ⏳

**Files:** `src/state.rs` (sequence Stream 5's lock work first; Stream 9's
quarantine logging rebases cleanly), `src/state_lock.rs`,
`src/commands/revert.rs`, `src/commands/apply.rs` (subprocess-timeout
sites only).

**Issues:** C1–C9, N12.13, N12.15, N12.16, N12.17, S4 (coordinated with
Stream 9), S1, audit N‑3 (residual), NEV2.3, NMOD.1, NMOD.2, NBE.6.

**Work:**

- ⏳ Lift the `HELD` mutex out of the retry-sleep loop (C1, N12.13) — the
  highest-frequency contention pin.
- ⏳ Use monotonic clock for cooldown (C2) instead of wall-clock-skew
  vulnerable `SystemTime`.
- ⏳ Add subprocess timeouts to `apply` and `revert` (C3); SIGTERM handler
  in events daemon (C4).
- ⏳ State quarantine rename: surface failures (C5, S4); chmod-after-write
  race (N12.16); UUID case-folding (N12.17); `lock_path_for` fallback for
  bare filenames (N12.15).
- ⏳ Bound `PROTEUS_LOCK_TIMEOUT_MS` (C8); document UUID-key cross-system
  migration behaviour (C9); restore handler-panic visibility (C7); make
  mock backend actually flock for test honesty (C6).
- ⏳ Replace `std::env::set_var` / `remove_var` in `uninstall.rs` test setup
  with a serialized-test harness (S1).
- ⏳ Apply `.custom_flags(libc::O_NOFOLLOW)` to the `state_lock` `OpenOptions`
  call (audit N‑3 residual). The `mode(0o600)` half landed in PR #310; the
  symlink-follow gap is what stayed open. Posture should match
  `write_atomic` (`O_CREAT | O_EXCL | O_NOFOLLOW`, `0o600`, RAII cleanup).
- ⏳ Reorder `apply::run` to load and validate config **before** acquiring
  the state lock (NMOD.1, **High**). Today the lock is held across config
  validation; combined with C1 / N12.13 (HELD mutex held across retry sleep)
  this can starve the rotate timer up to the full 5 s budget on a
  misconfigured per-SSID block. Concretely: move
  `Config::default_or_loaded` and `validate_ranges` ahead of
  `acquire_state_lock_or_print` in `src/commands/apply.rs:67-81`.
- ⏳ Pair NMOD.1 with moving the `require_yes` gate behind config validation
  (NMOD.2): users with config typos see the typo error before the
  confirmation prompt, restoring the "confirmation = mutation imminent"
  invariant.
- ⏳ Wrap `systemd-hostnamed` DBus calls
  (`set_static_hostname` / `set_pretty_hostname` / `set_hostname`) in
  `tokio::time::timeout(Duration::from_secs(5), …)` and surface `TimedOut`
  as a recoverable error (NEV2.3). A stalled hostnamed currently pins the
  NM dispatcher synchronously; document the bound in the wiki.
- ⏳ Add `mac.validate_assignable()` to `MockBackend::set_cloned_mac` so
  unit tests catch validator-edge-case bugs that production NM would
  reject (NBE.6).

**Acceptance:** stress test with 16 concurrent `acquire_state_lock` callers;
assert no thread blocks more than 5 s; assert no `panic = abort` is
triggered.

### Stream 6 — Panic Hardening ⏳

**Files (disjoint from all other streams):** `src/hostname/mod.rs`,
`src/diff/mod.rs`, `src/commands/mod.rs` (SHA verification path only),
`src/probe/mod.rs`, `src/captive_portal/mod.rs::body_slice` (line-bounded;
Stream 4 does not touch this function).

**Issues:** P2–P6, N12.10, NMOD.3, NTEST.3.

**Work:**

- ⏳ Empty-label hostname validator: replace bounds-panic with structured
  error (P2).
- ⏳ `.file_name().unwrap()` sites in diff and SHA verify: handle `..` /
  trailing-slash paths (P3, P4).
- ⏳ Off-by-one in CRLF body slice (P5); probe `as u8` truncation guard
  (P6).
- ⏳ `proteus diff` reads target files unbounded → cap to 64 MiB and
  surface a clear error past that (N12.10).
- ⏳ `proteus diff`: cross-reference `state.json`'s tracked-paths set
  against the filesystem and emit a "missing" entry per absent file in the
  diff report (NMOD.3). Currently `compute_managed_file_drift` only walks
  the filesystem, so files Proteus once managed but the operator deleted
  are silently invisible.
- ⏳ `tests/realworld/probe.sh`: pre-check `[ -d /sys/class/net ]` and skip
  with a clear message when `/sys` is not mounted (NTEST.3) — currently the
  loop runs zero times and the script "passes" with no probing.

**Acceptance:** fuzzer-style unit tests with property-based inputs (empty
strings, all-dots paths, oversize inputs).

### Stream 7 — Error Handling & Logging Discipline ⏳

**Files (coordinate with Stream 4 on `events.rs`, Stream 8 on `dhcp.rs`):**
`src/commands/apply.rs` (logging-only sites), `src/commands/events.rs`
(rebase after Stream 4), `src/commands/show_config.rs`,
`src/commands/dhcp.rs` (E5 dispatch only; rebase after Stream 8),
`src/nm/mod.rs::GetSecrets` (rebase after Stream 4),
`src/commands/config_cmd.rs`, `src/commands/doctor.rs`, `src/dns/mod.rs`
(audit-target only), `src/logging.rs`.

**Issues:** E1–E10, NEV2.4.

**Work:**

- ⏳ Demote info-level success-path events in `apply` (E1) and events
  daemon hot path (E2) to `debug`.
- ⏳ Surface `RUST_LOG` parse failures (E3); show-config permission errors
  at `error!` not `warn!` (E4).
- ⏳ Replace `Ok(exit::GENERIC_ERROR)` pattern with typed error returns
  (E5).
- ⏳ Stop swallowing NM `GetSecrets` failures (E6); stop `unwrap_or_default`
  on `read_to_string` results (E7).
- ⏳ Doctor probe error breadcrumbs (E8); unify `Config::default_or_loaded`
  fallback (E9); audit `.unwrap()` shared between dns prod and tests (E10).
- ⏳ Bluetooth adapter-disappeared: match on the underlying zbus error and
  log at `warn!` (continue) for `NotFound` / `UnknownObject`; propagate
  other variants as today (NEV2.4). Avoids spurious `error!` lines on
  benign hot-unplug.

**Acceptance:** snapshot test of stderr at default verbosity for the success
path of every mutator; assert empty.

### Stream 8 — Resource Hygiene & Performance ⏳

**Files:** `src/nft/mod.rs`, `src/wiki.rs`, `src/commands/dhcp.rs::renew`
(R3, R7 — sequence before Stream 7's E5), `src/kill_switch/mod.rs`
(validators only), `src/dns/mod.rs::lossy` clones (non-overlapping with
Stream 7's E10 audit).

**Issues:** R1–R8, N12.14, M3, NBE.1, NBE.2.

**Work:**

- ⏳ Close nft script stdin before `wait_with_output` (R1).
- ⏳ Stream-parse `nft list table` instead of fully-buffered (R2); add max
  request-size cap to captive portal HTTP path (N12.14).
- ⏳ Reuse a single DBus connection for DHCP status calls (R3); reduce
  `RenewOutcome` allocations to `&'static str` (R7).
- ⏳ Wiki search index — hash terms once, scan pages once (R6).
- ⏳ Drop redundant `lossy().into_owned()` on known-ASCII paths (R5).
- ⏳ Subprocess fd-close audit comments (R8); shell-metacharacter validators
  on iface names (M3 — coordinate with Stream 9).
- ⏳ Cache an `Arc<Connection>` on `NmBackend` so trait methods share a
  single `zbus::Connection::system()` per command invocation rather than
  re-authenticating on every method call (NBE.1) — same family as R3 but
  in a different file.
- ⏳ Cache the last availability check on the backend struct so
  `select_auto()` and `availability_matrix()` don't double-probe networkd
  via `/run/systemd/netif` syscalls (NBE.2).

**Acceptance:** `bench/` micro-benchmarks for nft-table parse and wiki search
(p50 / p99 budget); soak test for DHCP fd accumulation.

### Stream 9 — Security Surface Hardening ⏳

**Files:** `src/captive_portal/mod.rs::Location` (S2),
`src/state_lock.rs::lock_open` (S3 — sequence after Stream 5's C1),
`src/dns/mod.rs::resolved_dropin_cleanup` (S5),
`src/captive_portal/mod.rs::request_line_builder` (S6),
`dist/polkit/com.kit3713.proteus.policy` (S7, B15 — coordinate with
Stream 3), `src/events/source/reg_domain.rs::OwnedFd` (S9),
`src/nm/mod.rs::GetSettings/GetSecrets` log values (S8 — rebase after
Stream 4), `src/mac/generator.rs` separator parser (S10).

**Issues:** S2, S3, S5, S6, S7, S8, S9, S10, B15, audit M‑2 / N‑0, audit
L‑3 (residual), audit I‑1, NEV2.1.

**Work:**

- ⏳ Validate `Location` header per RFC (S2); enforce `mode(0o600)` on
  lock file (S3).
- ⏳ Open-by-fd then `unlinkat` for resolved drop-in cleanup, closing the
  TOCTOU (S5); join host + path through a single percent-encoder for HTTP
  request line (S6).
- ⏳ Restrict polkit policy to `unix-group:wheel` / `sudo` and add a runtime
  check in `proteus doctor` (S7, B15 — Stream 3 owns the file, Stream 9
  owns the policy text).
- ⏳ Expand the safety comment on `OwnedFd::from_raw_fd` (S9); strict MAC
  separator parser (S10).
- ⏳ Sanitize NM dict values before tracing (S8) — coordinate with Stream
  4's error-context preservation (N4).
- ⏳ Harden `Layout::from_env()` in `src/commands/uninstall.rs` so
  `PROTEUS_CONFIG_DIR` / `PROTEUS_STATE_DIR` / `PROTEUS_SYSTEMD_DIR` cannot
  steer `remove_dir_all` against `/etc` or anywhere outside an explicit
  allowlist (audit M‑2 / N‑0). Two acceptable shapes per the audit
  recommendation: gate the env reads on `#[cfg(any(test,
  feature = "test-overrides"))]` with hardcoded production paths, or refuse
  any path outside `{/etc/proteus, /var/lib/proteus, /etc/systemd/system}`
  plus tempdir-prefixed test variants. This is the highest-severity
  unresolved security finding on `main`.
- ⏳ Insert `--` before user-influenced positional args in every `iw` / `ip`
  / `ethtool` invocation (audit L‑3 residual). The `is_safe_iface` guard
  blocks shell metacharacters but does not block `iface = "-h"` flag-parse
  confusion. Sweep `src/rf/`, `src/kill_switch/`, and `src/mac/factory.rs`.
- ⏳ Consolidate the four hand-rolled SHA-256 implementations
  (`src/dns/apply.rs`, `src/stack/sha256.rs`, `src/diff/sha256.rs`,
  `src/ipv6/mod.rs`) into a single `crate::hash::sha256` module (audit
  I‑1). Document in `wiki/dns.md` that the resulting digest is a
  tamper-evidence marker, not a security property — anyone with write
  access to the drop-in can recompute the digest.
- ⏳ Validate the `name` argument in `wiki::get_page(name)`: reject `/`,
  `\`, and `..`; require `^[a-zA-Z0-9_-]+$` (NEV2.1). Today the embedded
  `include_dir` archive doesn't strictly enforce path canonicalisation
  either, so this is the defense-in-depth gate against any future caller
  forwarding user-supplied page names.

**Acceptance:** `cargo audit`; manual review of every new validator; test
for the polkit-missing path that asserts a clear error message.

### Stream 10 — Docs / Wiki / Examples Drift + ⏳ frontier items ⏳

**Why high user-facing impact:** every example config in `examples/`
documents fields the schema doesn't have. New users follow examples, get
nothing they think they configured, blame the tool. This is the
silent-deception class.

**Files (zero overlap with code streams):** `examples/*.toml` (all 7 files),
`wiki/cli.md`, `wiki/personas.md`, `wiki/troubleshooting.md`, `README.md`,
`src/error.rs` (docstring / wiki-hint additions; no logic),
`docs/security/dbus-surface.md`, `docs/realworld-test-log.md` (new).

**Issues:** D1–D8, N12.6, NPKG.13, plus the four ⏳ items carried forward
from [`ROADMAP-v0.3.md`](ROADMAP-v0.3.md):

- ⏳ Audit pass: every error string in `src/error.rs` and every `bail!` /
  `anyhow!` callsite carries a `wiki <page>` hint.
- ⏳ Bypass hardening pass: review every place we shell out.
- ⏳ Real-world testing on diverse Wi-Fi (coffee shops, hotels,
  conferences, airports).
- ⏳ Independent security review against `docs/security/dbus-surface.md`.

**Work:**

- ⏳ Fix `[discovery]`, `[rotation]`, `[mac]`, `[probes]` sections in every
  example to use real schema field names (D1–D4) — single sweep.
- ⏳ Document exit code 75 in `wiki/cli.md` (D5); correct doctor exit code
  reference (D6); recount wiki pages (D7); add `config set-profile`
  section (D8).
- ⏳ Fix `display_string` length-clamp to count output graphemes, not input
  chars (N12.6).
- ⏳ Wiki-hint audit pass on every `bail!` / `anyhow!` site (frontier item).
- ⏳ Bypass-hardening pass: enumerate every `Command::new` site and confirm
  argument-array form (no shell interpolation) (frontier item).
- ⏳ Real-world testing log: maintain `docs/realworld-test-log.md` with one
  entry per network type tested, pulling bugs into Streams 1–9 as they
  surface (frontier item).
- ⏳ Solicit independent review against `docs/security/dbus-surface.md`;
  track responses in a new `docs/security/external-review.md` (frontier
  item).
- ⏳ Document the `proteus-rotate.timer` ±75 min effective jitter
  (`RandomizedDelaySec=30min` + `AccuracySec=45min`) in
  `wiki/rotation.md`'s tuning section (NPKG.13) so operators tuning rotation
  cadence don't refile this as a bug. Info-only.

**Acceptance:** `examples/` files round-trip through the loader without
warnings; CI step that runs `proteus config validate examples/*.toml` and
fails on any non-zero exit.

## Existing utilities to reuse (do not reinvent)

- `crate::backend::mock::MockBackend` (`src/backend/mock.rs`) — already
  lifted to the production tree for unit tests. Stream 1's integration
  tests and Stream 4's events test should drive through this, not a new
  fake.
- `crate::persona::active_for` (`src/persona/mod.rs`) — Stream 4's per-SSID
  stub fix should resolve through this helper, not duplicate the
  precedence logic.
- `crate::nm::update_with_secrets` (added in v0.2.8-alpha for issue #207) —
  any Stream 4 NM-write site should call this, not re-implement secret
  merging.
- `crate::write_atomic` (`src/commands/mod.rs`) — every state-file write
  goes through this; Stream 5's chmod-race fix (N12.16) should not add a
  second chmod path.
- `crate::probe::{ArpProbe, NdProbe}` traits — Stream 4's collision
  handling uses these; do not bypass into raw socket code.
- `crate::cli::actions::HostnameAction::yes` and the parallel
  `yes: bool` fields — Stream 1 should activate these *existing* fields
  rather than introduce a parallel mechanism.
- `crate::events::source::*` mock variants — every Stream 4 source has a
  mock twin already; reuse the pattern rather than introducing new test
  scaffolding.

## Verification — end-to-end

1. **Per-stream**: each stream's acceptance block above (cargo tests,
   integration scenarios, container builds, snapshot tests).
2. **Cross-stream regression**: full `cargo test --release` (catches
   `panic = abort` regressions Stream 2's N12.5 protects against).
3. **Issue-log gating**: `docs/ISSUES.md` gains a new column `fixed-in`
   pointing to the version each item lands in. The roadmap is "done" when
   every Critical and High row has a non-empty `fixed-in` cell within
   `0.4.x-beta`.
4. **Real-world** (⏳ fold-in): `docs/realworld-test-log.md` accumulates
   entries from coffee-shop / hotel / conference / airport runs; any
   High-severity regression surfaced cuts a `v0.4.x.Y-beta` patch.
5. **Container matrix in CI**: `nm` / `networkd` / `raw` scenarios on
   Fedora 43, Debian 13, Gentoo, Alpine 3.20, Arch — assert
   `proteus apply` is clean and `systemctl is-enabled
   proteus-events.service` reports `enabled`.
6. **Mission check**: `nmap -O` against the host before / after `proteus
   persona use iphone-15` continues to produce diverging detections. The
   persona feature is the headline of v0.3 and must not regress through
   any Stream 4 refactor.

## How to help

- **Real-world testing** — `proteus doctor` + `proteus apply` on
  coffee-shop / hotel / conference / airport networks; report bugs via the
  issue template (highest-value contribution right now).
- **Independent security review** — eyes on `wiki/threat-model.md` and the
  DBus surface enumerated in `docs/security/dbus-surface.md`.
- **Persona contributions** — `data/personas/*.toml` is open for community
  PRs to grow the catalogue.
- **Distro packaging** — Alpine, Void, Gentoo packagers needed, plus AUR /
  Copr / Debian unstable submission sponsors.
- **Wiki** — pages are landed but always improvable; voice should match
  `wiki/intro.md`.
- **Code** — see [`CONTRIBUTING.md`](../CONTRIBUTING.md). Pick any ⏳ row
  in any stream above; multiple contributors can work in parallel because
  the streams are partitioned by file area.

## Things explicitly NOT on the roadmap

The mission is local controllable fingerprint reduction. These items live
on another tool's layer or are physical limits:

- **TLS / browser fingerprint** (JA3/JA4 ClientHello, font / canvas /
  WebGL fingerprints) — use Tor Browser, librewolf, Brave's randomization.
- **Wireshark-class payload-content analysis** — persona mode shapes
  packet *headers* and protocol *fingerprints*, not payloads.
- **DNS resolution policy beyond ECS strip** — use dnscrypt-proxy,
  NextDNS, AdGuard Home, Pi-hole.
- **Tracker blocking** — Pi-hole, NextDNS, uBlock Origin.
- **Traffic correlation defenses** — Tor, Mullvad.
- **SSH client fingerprint (HASSH)** — your `ssh_config` is yours.
- **Hardware-baked RF fingerprints** (oscillator drift, DAC nonlinearity,
  IQ imbalance) — physically impossible without a hardware swap; see
  `wiki/rf-fingerprinting.md`.
- **Telemetry, update checks, analytics** — no telemetry, ever.
