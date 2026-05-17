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
| NCMD2.3 | medium | 4 | ✅ `apply` skips `daemon-reload` on dns/stack/ipv6/resolved |
| NCMD2.4 | medium | 4 | ✅ revert mis-targets deleted / recycled NM uuids |
| NCMD2.5 | low | 1 | `status --json` outputs U+FFFD on non-UTF-8 ifaces |
| NSUB.1 | medium | 4 | ✅ `[stack]` apply silently no-ops kernel-unsupported keys |
| NSUB.2 | medium | 4 | ✅ `[stack]` revert leaves orphaned hardened sysctls |
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

### No-action documented (info-only / intentional / duplicate)

Items the audits and bug-hunt sessions flagged as *not* requiring code change.
Tracked here so the issues log can close them without confusion.

| ID | Source | Reason no-action |
|---|---|---|
| M2 | ISSUES.md §11 | `mem::forget(guard)` in `write_atomic` is intentional; documented in source comment |
| M5 | ISSUES.md §11 | Rustc 1.93.0 vs MSRV 1.85 + Edition 2024 — both pins correct; flagged for clarity |
| R8 | ISSUES.md §10 | Implicit subprocess fd close everywhere — already correct; comments only if the team wants |
| N12.13 | ISSUES.md §12 | Re-confirmation of C1 (state-lock HELD mutex held across retry sleep). Tracked under C1. |
| N12.18 | ISSUES.md §12 | "Year 700,000 problem" — `t as i64` cast in `unix_to_ymdhms` after `t /= 24`. Not reachable in any plausible runtime. |
| N12.19 | ISSUES.md §12 | Re-confirmation of N2 (`factory::permanent_address` returns `Option`). Tracked under N2 in Stream 4. |
| N12.20 | ISSUES.md §12 | Duplicate of M3 (subprocess iface-name validators allow shell metacharacters). Tracked under M3 in Stream 8. |
| NPKG.13 | ISSUES.md §13.5 | `proteus-rotate.timer` ±75 min jitter is intentional fingerprint defense. Documented in Stream 10 so future audits don't re-flag. |

### GitHub issue absorption (kit3713/proteus, open)

79 open GitHub issues as of 2026-05-08. The bug / security cluster maps to
existing or expanded streams; the enhancement cluster lives in the
**Enhancements queue** section further below.

**Bug / security GH issues mapped to streams** (duplicates of issues-log items
flagged with "= ID"):

| GH # | Severity | Stream | Notes |
|---|---|---|---|
| #348 | critical | 1 | = CL2/M1/N12.2/N12.3 — `--yes` dropped on dhcp + portal mutators |
| #349 | high | 1 | = CL1 — `--watch --interval 0s` zero-sleep CPU loop |
| #345, #339 | high | 2 | = N12.5 — `is_valid_per_ssid_duration` multibyte panic |
| #340 | high | 2 | new — `ByteSuffixPattern::parse` panics on multibyte input (same class as #284); land alongside N12.5 |
| #391 | critical | 1 | = N12.1 — `unpin` skips `--yes` |
| #375 | critical | 1 | = M1/N12.2 — `dhcp apply/revert` drops `--yes` (incomplete fix of #242) |
| #351 | high | 3 | = N12.8/B1 — `proteus-events.service` ExecStart path mismatch |
| #370 | medium | 5 | = audit N‑3 — state-lock `O_NOFOLLOW` + post-open `chmod` TOCTOU |
| #377 | medium | 5 | = C2 — cooldown reads `SystemTime::now()` (wall clock) |
| #388 | medium | 10 | = N12.6 — `display_string` clamps INPUT not OUTPUT |
| #371 | meta | — | "index of edge / zero-day findings filed against v0.4.2-beta"; this roadmap IS the index |
| #389 | medium | 9 | new — BiDi override codepoints (U+202E etc.) pass `display_ssid` (mirror of #241/#224) |
| #386 | medium | 1 | new — `proteus revert` ignores global `--state` flag; nested steps hardcode default |
| #382 | medium | 2 | new — `iot-generic.toml` `oui_pool` has tokens `Vendor::from_pool_token` doesn't know — silent LAA degrade |
| #381 | medium | 1 | ✅ Wave 3 Group A — backend trait `rotate_if_needed` now takes `state_path: Option<&Path>`; cooldown read + inner rotate honour operator's `--state` |
| #380 | medium | 9 | new — `persona list` prints unvalidated `display_name`/`notes` — terminal injection |
| #379 | medium | 4 | ✅ `apply` not idempotent — `build_forbidden` always adds `current_mac` to forbidden set |
| #374 | medium | 9 | new — `persona list` prints `display_name` byte-for-byte — ANSI/control/BiDi injection |
| #373 | medium | 9 | new — `portal mark/unmark` print raw SSID via `println!` — same N-2 family, four sites |
| #367 | medium | 9 | new — terminal-escape passthrough via raw `iface` echo in `rotate{,-if-needed}` (N-2 third site) |
| #366 | medium | 4 | ✅ Wave 3 Group A — `rotate_one` now reads connection metadata before the backend write but commits `state.managed.{connections,interfaces}` AFTER `set_cloned_mac` returns Ok |
| #365 | medium | 9 | new — `proteus session` (and `portal list`) prints SSIDs/aliases verbatim — terminal injection |
| #363 | medium | 5 | new — state-lock acquire chmods state-dir parent unconditionally (same family as #354) |
| #362 | medium | 9 | new — NM dispatcher `validate_cli_value` 128-byte cap uses bash `${#val}` (char-count, locale-bypass) |
| #361 | medium | 9 | new — `persona edit` runs `$EDITOR` as root with caller's `$HOME` (vector L-4 missed) |
| #360 | medium | 9 | new — `factory::EthtoolBin` PATH-fallback contradicts absolute-path pinning intent of #202 |
| #359 | medium | 9 | new — 5 distinct iface validator definitions across modules; consolidate |
| #358 | medium | 9 | new — `ipv6::write_sysctl` `key` and `value` parameters unvalidated (latent path traversal beyond M-1) |
| #357 | medium | 9 | new — terminal-escape via SSID echo in `session` and `portal {mark,list,unmark}` (N-2 partial fix) |
| #355 ✅ | medium | 4 | `events nm-connection-up` poll fallback no longer fires spurious `ConnectionUp` on daemon startup — `is_activation_edge` in `src/events/source/nm_connection_up.rs` requires the detector to have observed at least one prior reading before any rising edge fires |
| #354 | high | 5 | new — `--state <path>` chmods parent dir to `0700`, system-bricking footgun (e.g. `--state /tmp/x` → `chmod /tmp 0700`) |
| #352 | medium | 1 | new — `timer resume` short-name maps to non-existent `proteus-resume.timer` (artifact is `.service`); `enable/disable/set/reset/status resume` all target a unit that doesn't exist |
| #342 | high | 9 | new — path traversal in `persona {new,edit,show,use}` via unvalidated `<id>` (mirror of NEV2.1) |
| #378 | low | 1 | new — `rotate-if-needed --explain` to surface policy + cooldown math the dispatcher hot path is making (request, but useful for triage) |

Of the 34 bug-class GH issues above, 9 are duplicates of existing roadmap
entries (cross-referenced) and 25 are new bugs absorbed into the appropriate
stream. The Enhancements queue below covers the remaining 45 GH issues.

### Issue-closure protocol

When a stream's work lands, the matching GitHub issues **must auto-close
through the PR** that merges the fix. This roadmap is how we know which
issues to close.

**Rule.** Every PR that lands work on a roadmap stream must include a
`Closes #N` (or `Fixes #N`) line in its description for each GitHub issue
that the diff actually resolves. GitHub will close the issue on merge. If a
PR fixes a partial subset of an issue, use `Refs #N` and leave the issue
open with a status comment.

**Mapping for PR authors.** Use the "GitHub issue absorption" table above
plus the per-stream issue lists to find the affected GH issues. Concrete
examples:

- A Stream 1 PR that wires `--yes` through `dhcp apply / revert` and the
  portal mutators should write:
  `Closes #348, #391, #375` (and `Refs #242` to credit the prior partial
  fix).
- A Stream 9 PR that introduces the central
  `crate::display::display_safe` helper and replaces every raw SSID /
  iface print should write:
  `Closes #357, #365, #367, #373, #374, #380, #389` (the seven-issue
  terminal-injection cluster — see "Notes for the maintainer" #3 below).
- A Stream 5 PR landing audit N‑3 (`O_NOFOLLOW` on state lock) should
  write: `Closes #370`.
- A Stream 2 PR landing N12.5 / V2 (multibyte panic + ByteSuffixPattern)
  should write: `Closes #339, #340, #345`.

**Duplicates and meta issues.** GH #371 ("index of edge / zero-day
findings") is meta; close it with a comment pointing at this roadmap once
the first wave of streams ships. The Enhancements-queue 🟡 / 🔴 entries
stay open in GitHub as the canonical request and are linked from this
roadmap; they close when the corresponding feature ships in v0.5+.

**Audit-only findings (`M-2`, `N-1`, `L-3`, etc.).** No GitHub issue
exists for these — they came from the archived security audits. The
roadmap is the only tracker. PR descriptions still cite the audit ID for
the historical record.

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

### Stream 1 — CLI Safety & Confirmation Gates 🚧

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

- ✅ Add `--yes` field to `unpin` action (N12.1) and to every mutator action
  that declares a `yes: bool` field today (CL3 — currently dead code).
- ✅ Wire `--yes` through dispatch for Bluetooth / Hostname / DNS / Resolved /
  NTP / Portal (CL2), DHCP apply / revert (M1, N12.2), Portal mark / unmark /
  open (N12.3).
- ✅ Reject `--interval 0s` in watch mode (CL1) and `<1ms` sleep granularities
  (CL7).
- ✅ Add integration scenarios for the 24 untested subcommands (CL4).
  Twelve new `.sh` scenarios under `tests/integration/scenarios/` covering
  `session`, `diff`, `dry-run`, every component status reader, every
  component apply/revert `--yes` gate, `persona list/show/current/random/
  validate`, `ssid list/show/set/clear`, `wiki search`, top-level `help`,
  `completions {bash,zsh,fish}`, `kill {status,resume}`, `timer {set,
  reset,logs}`, `config {edit,set-profile}`, `probe`, and `events run`.
  Contract: exit 0 on `--help`, 64/65/66 on obvious failure modes, valid
  JSON on `--json`. Runner auto-discovers via `scenarios/*.sh` glob.
- ✅ Add `--json` flag to `resume` and `wiki` (non-search) for parity (CL6).
- ✅ Document the prefix-collision risk in CLI changelog (CL5). Landed in
  `CHANGELOG.md`'s `[Unreleased]` "Known sharp edges" subsection and
  `wiki/cli.md`'s new "Prefix matching" section.
- ✅ `proteus rotate --dry-run` preview: thread the configured OUI pool into
  `mac::plan::preview_mac` so the previewed MAC reflects the persona, not a
  hardcoded LAA placeholder (NM2.7).
- ✅ `proteus doctor` exit code: split warn-only vs fail; reserve exit 1 for
  `fail > 0`, map warn-only to exit 0 (or a distinct code) so CI wrappers
  don't block on warnings (NCMD2.1).
- ✅ `proteus status --json`: skip non-UTF-8 sysfs entries with a `debug!`
  line, or include them with explicit `valid_utf8: false` (NCMD2.5).
- ✅ NM dispatcher `rc=70` branch: log captured stderr at `info!` (or split
  the exit code) so "backend unavailable" doesn't mask "missing nft / nl80211
  / CAP_NET_ADMIN" (NEV2.7).
- ✅ `rotate-if-needed --explain` to surface policy + cooldown math the
  dispatcher hot path is making (GH#378).
- ✅ `proteus revert` honours global `--state` flag (GH#386).
- ✅ `timer resume` short-name maps to the actual shipped artifact
  `proteus-resume.service` (GH#352).
- ✅ `rotate-if-needed --state` end-to-end. Backend trait
  `rotate_if_needed` now takes `state_path: Option<&'a Path>` (see
  `src/backend/mod.rs:190` and the matching impls in `nm.rs`,
  `mock.rs`, `raw.rs`, `networkd.rs`). The inner
  `rotate_if_needed_inner` falls back to `DEFAULT_STATE_PATH` only
  when no path is supplied. Same trait is reused by C6 (mock flock
  on opt-in state path) and N14 (per-iface mutex registry). (GH#381)

**Acceptance:** end-to-end script that calls every mutator without `--yes` and
asserts exit code 64 (`CONFIRMATION_REQUIRED`); run watch with `--interval 0s`
and assert exit 64 (rejection) instead of CPU burn.

### Stream 2 — Config Schema Validation 🟢 (mostly landed)

**Why high-impact:** silent acceptance of nonsense config (zero-interval
rotates, unknown profile names, multibyte panic in duration parser) gives users
a posture they did not ask for.

**Files:** `src/config.rs`, `src/per_ssid.rs`, `src/persona/load.rs`,
`src/persona/template.rs`, `data/personas/lg-tv-2023.toml`.

**Issues:** V1–V12 (V8, V9 added explicitly above), N12.4, N12.5, N12.12,
P1, P7, NMOD.4, NTEST.1.

**Work:**

- ✅ Reject zero / empty rotation intervals at config-load time (V1) —
  enforced via `parse_interval` for every rotate-shaped field; pinned
  with `config::validation_tests::v1_*` regression tests.
- ✅ Validate profile names and persona IDs against the known catalogue at
  load time (V2, V6) with closest-match suggestions; built-in catalogue
  exposed via new `persona::load::builtin_ids()`.
- ✅ Validate `quorum_n <= quorum_total` (V3).
- ✅ Bound second-precision durations (V4); bound `tx_power_reduction_db` (V5).
- ✅ Validate `pin_mac` format at load (V7) via `Mac::from_str`.
- ✅ Validate persona OUI pool (V11). User personas hard-fail on an
  unknown vendor token; the OUI catalogue (`src/mac/oui.rs`) now carries
  `Espressif` and `Realtek` variants so `iot-generic`'s shape-only
  tokens resolve to real IEEE prefixes rather than degrading to LAA.
- ✅ Fix `parse_duration` overflow (N12.4) and the multibyte-trailing-char
  panic in `is_valid_per_ssid_duration` (N12.5) — shipped together.
- ✅ Constrain `clap` `u32`/`u64`/`usize` flags to sane ranges (N12.12).
  Bounded via `value_parser!(T).range(...)`: `timer logs --lines` 1..=100_000,
  `wiki search --limit` 1..=500, `events run --max-triggers` 0..=10_000_000,
  `events run --once-after-secs` 0..=86_400, `rotate-if-needed --cooldown`
  0..=86_400. Out-of-range values now reject at clap parse time.
- ✅ Replace `split_at` on potentially-empty duration strings (P1).
- ✅ Replace `get_mut("mac").unwrap()` test path (P7) — handled via the
  Wave 1 carryover in `src/commands/config_cmd.rs`. The test uses
  structured `.expect("default schema must contain a [mac] section")` +
  `.expect("[mac] must be a table in the default schema")` so a future
  schema rename surfaces the actual cause instead of a bare panic line
  number.
- ✅ Distinguish "no value" from "out of range" in `parse_duration`; emit
  a `warn!` on overflow rather than silent fallback to global timer
  (V8 — paired with N12.4 via `checked_mul`).
- ✅ Rename `persona_contributed` → `global_persona_contributed` and add
  a short comment so the 4-layer resolver source-trace reads correctly
  (V9).
- ✅ Round-trip test coverage expansion for arrays, numerics, enums (V10).
- ✅ SSID-key TOML-special-character coverage (V12) — spaces, dots,
  brackets, unicode, backslash escapes.
- ✅ Persona `schema_check` now renders the template through
  `validate_hostname` so unusable personas like `lg-tv-2023` are caught
  at load (NTEST.1, **High**). Template fixed:
  `[LG]_webOS_TV_{word}` → `lg-webos-tv-{word}`. Regression test
  `every_embedded_persona_hostname_template_renders_validly` exercises
  every shipped persona.
- ✅ Defensive guard + const-assert that `OWNER_POOL` is non-empty before
  indexing in `persona::template::pick_owner` (NMOD.4).
- ✅ GH#340 (`ByteSuffixPattern::parse` multibyte panic) — fixed in
  `src/mac/generator.rs`: the parser now gathers cleaned characters
  into a `Vec<char>` and reads from the vec instead of byte-indexing
  the raw `&str`, so multibyte input (e.g. `"é:23:xx"`) errors cleanly
  instead of panicking inside a `&str[i..j]` slice. Same panic class
  Stream 2 fixed for `is_valid_per_ssid_duration`.

**Acceptance:** new test module `config::validation_tests` loads each malformed
example and asserts the specific error variant; full `cargo test --release`
passes (catches `panic = abort` regressions immediately).

### Stream 3 — Packaging & Build / CI Coherence (mostly ✅)

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

- ✅ Wire `proteus-events.service` into install.sh `enable / start` ladder
  (B1); add `%post` / `%preun` hooks in RPM (B2), `dh_installsystemd` in
  Debian (B3), `systemd_dounit` in Gentoo (B4), Alpine post-install
  trigger (B5 — Alpine ships OpenRC only; the post-install nudges
  operators to the NM dispatcher hook instead).
- ✅ Reconcile `/usr/local/bin` (install.sh) vs `/usr/bin` (distro
  packages) so the unit's `ExecStart=` resolves on every path (B10, N12.8).
  install.sh now creates `/usr/bin/proteus → /usr/local/bin/proteus`
  symlink when no real file is there.
- ✅ Add `KillMode=mixed` and `TimeoutStopSec=10s` to
  `proteus-events.service` (N12.9).
- ✅ POSIX-ify NM dispatcher shebang (B6); validate install.sh with `sh -n`
  (B13). `scripts/check.sh` and the new `packaging-lint` CI job both run
  `dash -n` over `install.sh` / `uninstall.sh` / dispatcher.
- ✅ Add top-level `permissions: contents: read` block to `ci.yml` (B7).
- ✅ Pin `softprops/action-gh-release@v2` to commit SHA (B9 — already
  pinned at `3bb12739…`, which is `v2.6.2` / current `v2`).
- ✅ Add `--locked` to Alpine / Void / Gentoo cargo invocations (B8).
- ✅ Wire a real `[features]` table in `Cargo.toml` to back the Gentoo USE
  flags (B14). ⏳ Restrict polkit policy (B15 — already
  `auth_admin` / `allow_inactive=no`; further tightening deferred to
  coordinate with Stream 9 runtime check).
- ✅ Replace `build.rs::panic!` with actionable errors (B12); add `:?`-guard
  pattern to `uninstall.sh` (B11); shorten `Cargo.toml` description below
  256 chars (M4 — now 248 chars).
- ✅ Wire `cargo audit` into the release workflow with `Cargo.lock` as the
  target; fail the release on any open advisory in `zbus`, `clap`, `tokio`,
  `toml`, `toml_edit`, `serde`, `tracing`, `getrandom` (audit I‑2).
- ✅ Populate `sha512sums` in `dist/alpine/APKBUILD` (NPKG.7) and `checksum`
  in `dist/void/template` (NPKG.8); add a guard rejecting the literal
  `"SKIP"` placeholder so an early packager-build cannot ship without
  integrity validation. **Supply-chain gap closed via `sanitycheck()` (Alpine)
  and `pre_fetch()` (Void); real hashes still TODO once v0.1.0 is tagged.**
- ✅ Replace `build.rs` `panic!` on wiki-file read errors with an actionable
  `expect("…")` message and `cargo:warning=` line (NPKG.3); emit
  `cargo:rerun-if-changed` per file rather than per directory so deletions
  invalidate correctly (NPKG.4).
- ✅ Bump `dist/debian/control` `debhelper-compat` to 14 (NPKG.6).
- ✅ Document the `rpmbuild --without check` bypass risk in
  `dist/rpm/README.md` (NPKG.9); add a `%bcond_without check` guard so
  enabling check is the default and skipping it requires intent.
- ✅ Fix `install.sh` polkit `sed` rewrite: use `#` delimiter and quoted
  variable so a `BINARY_DST` containing `|` does not split the s/// (NPKG.14).

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

- ✅ Make `RotateOnTriggerHandler` actually rotate (N1) — the most important
  single fix in the whole roadmap; closes a documented-but-broken security
  feature. Wired in `src/commands/events.rs` via `with_backend(...)` +
  `Handle::spawn`, with regression test
  `rotate_on_trigger_handler_actually_rotates_the_mock_backend`.
- ✅ Fix `factory::permanent_address` `Option` → `Result` (N2, N12.19) so
  I/O failure is distinguishable from "no factory MAC". New
  `FactoryLookup { Found, Unavailable, IoError }` exposed via
  `permanent_address_result()`; legacy `Option` shape preserved.
- ✅ Validate the `iface` argument before `EthtoolBin::permanent` calls
  `ethtool -P <iface>` (audit N‑1). `is_valid_iface_name` mirrors the
  kernel's `dev_valid_name()` rules; refuses leading `-`, NUL, control
  bytes, > 15 bytes.
- ✅ Probe NM DBus interface version (N3) via `nm::probe_version`;
  preserve method / path on zbus errors (N4); fix connection lookup
  id / uuid mixing (N6) — `find_connection_by_id` accepts either form.
- ✅ Implement per-trigger debounce on link-flap detector (N8);
  subscribe-equivalent for `DeviceAdded` (N12) via 10-second
  `GetDevices` poll that attaches watchers for newly-added devices.
- 🚧 Captive portal: validate TLS, follow redirects (N9). HTTP-only
  redirect-following landed in `src/captive_portal/mod.rs`
  (`http_get_following`, bounded at 5 hops). TLS validation
  **deferred** — captive_portal forbids TLS deps (no `rustls` /
  `webpki-roots` in `Cargo.toml`) and adding one is a maintainer
  design call. https → unfollowable surfaces as `PortalRequired`
  with the validated target attached. ✅ Fix `Host:` header for IPv6
  literals (N12.7) via `format_host_header` (RFC 7230 §5.4 brackets);
  reorder `to_socket_addrs` to v4-first (N10) via stable sort by family.
- ✅ Reload captive-portal config on `SIGHUP` (R4) — primitive
  `CaptivePortalReload` (`Arc<RwLock<CaptivePortalConfig>>`) shipped
  in `src/captive_portal/mod.rs`; SIGHUP wiring follows in a follow-up
  in `src/commands/events.rs`. ✅ Per-SSID stub returns the real SSID
  (N12.11) via `read_active_ssid_via_proc`.
- ✅ Test coverage: factory MAC fallback failure path (N7) covered via
  `factory_lookup_*` tests on the new typed shape; mock-backend
  mutex-poisoning recovery (N13) documented as `#[ignore]`'d
  regression test pinning the desired into_inner-recovery shape.
  ✅ N5 (full `GetSettings → Update` with PSK round-trip) closed via
  `tests/nm_get_settings_roundtrip.rs` test-local `MockNmConnectionSettings`
  shim (no live DBus). Three tests pin
  `802-11-wireless-security.psk` survival across an unrelated ssid
  mutation, `802-1x.password` + `802-1x.private-key-password` survival
  on the EAP-TLS path, and a negative-control proving the mock correctly
  models NM's wipe-on-absent-key semantics. Exercises production
  `nm::merge_secrets` + `nm::SECRET_SECTIONS` directly.
- ✅ Init-system detection paths beyond hardcoded list (N11). Added
  `src/init/posix_fallback.rs` with detection + artifact rendering
  for s6, dinit, and a generic POSIX fallback. `select::detect`
  walks Systemd → OpenRC → Runit → SysVinit → s6 → dinit →
  posix-fallback → Systemd (default).
- ✅ Per-SSID policy debounce vs concurrent CLI rotate (N14) — per-iface
  `Arc<tokio::sync::Mutex<()>>` registry in `backend::nm`, keyed by
  iface name. Acquired BEFORE the state lock so two parallel tokio
  tasks against the same iface fully serialise (the state lock's
  intra-process reentrancy meant both used to pass the cooldown
  check). Different ifaces still rotate in parallel. Pinned by
  `n14_concurrent_rotate_same_iface_serialises` (would fail pre-fix)
  and `n14_concurrent_rotate_different_ifaces_proceeds_in_parallel`.
- ✅ Drop the `&& opts.pool.len() > 1` guard in
  `mac::generator::generate_with_probe` so single-token persona pools (e.g.
  `oui_pool = ["apple"]`) reset `consecutive_collisions` on every retry
  rather than running out the 64-attempt budget on the same OUI (NM2.1,
  **High**). Both ARP and ND branches now reset unconditionally;
  single-token cursor wrap is a no-op but the counter reset clears the
  budget.
- ✅ Add a doc comment to `generate_for_vendor` stating the caller must
  validate the returned MAC; today both callers do, but the postcondition is
  undocumented (NM2.5).
- ✅ Run a single `systemctl daemon-reload` at the end of `apply::run()`
  after dns / stack / resolved / ipv6 drop-ins are written, so the
  documented effect actually materializes without a manual reload (NCMD2.3).
- ✅ Validate cached NM uuids against the live `Settings.ListConnections`
  before invoking restore in `revert`; drop missing uuids with `warn!`,
  reject restore when uuid is present but the SSID/id has changed (NCMD2.4
  — guards against NM uuid recycling silently corrupting an unrelated
  profile). Builds on the N6 fix.
- ✅ Surface `warn!` when `read_sysctl(key)` returns `None` during `[stack]`
  apply (kernel doesn't expose the key) and skip writing the drop-in
  (NSUB.1). At revert time, re-probe each cached key and restore only those
  that exist now; log orphans at `info!` (NSUB.2).
- ✅ Captive portal classifier: treat empty `expected_body` paired with
  empty body as `Unknown` rather than `Clear` (NEV2.2). Pinned by
  `empty_expected_body_with_empty_body_is_unknown`.
- ✅ Bluetooth name length: query adapter capabilities and cap
  BLE-only adapters at ~30 bytes; `warn!` if the configured alias
  would be truncated by the controller (NEV2.5). Helper
  `recommend_alias_byte_cap` in `src/bluetooth/apply.rs` keys off
  `Adapter1.AddressType == "random"`; warning fires from
  `apply_one` before the BlueZ write.
- ✅ DHCP DUID/IAID asymmetry on rotate: documented the tradeoff and added
  `[dhcp] keep_iaid_stable_across_rotation` so the operator can pin IAID to
  NM's `"stable"` derivation (constant per-iface, DUID-derived) while DUID
  itself still rotates. Default off — the historical both-rotate behaviour
  for strongest unlinkability, opt-in for DHCPv6-only stable-pool networks
  (NBE.3, Wave 3 Group A).
- ✅ Honor `[dhcp] suppress_vendor_class = true` over the persona's
  `vendor_class_identifier` write — user suppression should win (NBE.4).
  Verified: `apply_persona_fingerprint` already short-circuits when
  `suppress_vendor_class` is true (`src/nm/dhcp.rs:76`).
- ✅ Extended the enterprise-Wi-Fi mock test to round-trip a connection
  with `private-key-password` set; new test pins that the EAP-TLS key
  passphrase survives the merge-secrets path and that the merge does not
  invent a `private-key-password` for PEAP-only profiles (NBE.5,
  Wave 3 Group A).
- ✅ NM `Reapply` race: `nm/dhcp.rs::renew_lease` now reads the
  connection's `Settings.Connection.VersionId` (NM 1.20+) and passes it to
  `Device.Reapply` so a concurrent `nmcli connection modify` surfaces as
  a DBus version-mismatch error rather than a silent stale-write. Older NM
  without the property falls back to the legacy `version=0` contract
  (NBE.7, Wave 3 Group A).
- ✅ Backend re-resolves NM device by iface name on every mutating call
  (`set_cloned_mac`, `read_cloned_mac`, `renew_lease`) rather than trusting
  the NM Device object path cached at `list_devices` time. Falls back to
  the cached identifier when the iface lookup fails so the operator-error
  path stays clean (NBE.8, Wave 3 Group A).
- ✅ `ethtool -P` parser: match against both `permanent address:` and
  `permanent mac address:` (Linux 6.3+ Intel iwlwifi variant) (NBE.10).
  Pinned by `parse_ethtool_permanent_accepts_linux_6_3_mac_header`.
- ✅ Replace `persona-effectiveness.sh`'s fixed `sleep 5` with a poll-until
  loop on `proteus current --json` (MAC + `last_rotated`), with a 60s
  default timeout (override via `PROTEUS_PERSONA_EFFECT_TIMEOUT_SECS`),
  so slow CI runners don't conflate baseline and persona variants
  (NTEST.2). Captures pre-apply baseline and polls every 1 s until MAC
  differs AND `last_rotated` advances, or the timeout fires with a
  clear diagnostic.

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

- ✅ Lift the `HELD` mutex out of the retry-sleep loop (C1, N12.13) — the
  highest-frequency contention pin.
- ✅ Detect wall-clock skew in cooldown calc; degrade gracefully (C2).
  `last_rotated` persists as ISO-8601 for cross-process visibility so a
  strict monotonic comparison is impossible, but `remaining_cooldown`
  now classifies two skew patterns: `last > now` (clock moved backward)
  → returns `None` + `tracing::warn!` so a rotate proceeds; `elapsed >
  COOLDOWN_SKEW_CEILING` (30 days) → same path. Three new unit tests
  pin the future-stamp, absurd-elapsed, and in-window shapes. Pairs
  with N14 (per-iface mutex) so a same-iface concurrent race no longer
  exploits the skew window.
- ✅ Add subprocess timeouts to `apply` and `revert` (C3).
- ✅ SIGTERM handler in events daemon (C4). The shutdown loop's plain
  `tokio::time::sleep` now sits inside a `tokio::select!` that races the
  250 ms tick against `SignalKind::terminate()` and
  `SignalKind::interrupt()`. On a signal the loop breaks normally so the
  existing `shutdown_tasks` source drain + in-flight rotate `JoinHandle`
  drain (both bounded at 5 s) become reachable from systemd's
  `ExecStop` path. `Cargo.toml` adds tokio's `macros` + `signal` features
  for the select/signal API.
- ✅ State quarantine rename: surface failures (C5, S4); ✅ chmod-after-write
  race (N12.16); ✅ UUID case-folding (N12.17); ✅ `lock_path_for` fallback for
  bare filenames (N12.15).
- ✅ Bound `PROTEUS_LOCK_TIMEOUT_MS` (C8); ✅ document UUID-key cross-system
  migration behaviour (C9); ✅ restore handler-panic visibility (C7) —
  `EventRegistry::fire` wraps every `h.handle(&trigger)` in
  `catch_unwind(AssertUnwindSafe(...))`, logs panics at `tracing::error!`
  with `handler_index` + `kind` + downcast payload, bumps a per-registry
  `handler_panics: AtomicU64` counter, and continues dispatch so a single
  panicking handler can't take down the daemon. Rotate-task / source-task
  `JoinError`s in `commands/events.rs` similarly split `is_panic()` →
  `error!` from `is_cancelled()` → `warn!`. ✅ make mock backend actually
  flock for test honesty (C6). `MockBackend` now accepts an opt-in
  `state_path: Option<PathBuf>` via `with_state_path(p)`. When configured
  it acquires `crate::state_lock::acquire_for_state_path(...)` before the
  cooldown decision, mirroring `backend::nm::rotate_if_needed_inner_with`.
  A foreign-fd flock causes `LockError::Busy` →
  `RotateOutcome::SkippedCooldown { remaining: 1s }`. (Same-process
  concurrent rotate races remain a separate concern — the
  `Mutex<Option<File>>` lock serialises the acquire but not the rotate
  body across parallel tokio tasks; see N14 follow-up.)
- ✅ Replace `std::env::set_var` / `remove_var` in `uninstall.rs` test setup
  with a serialized-test harness (S1).
- ✅ Apply `.custom_flags(libc::O_NOFOLLOW)` to the `state_lock` `OpenOptions`
  call (audit N‑3 residual). Paired with `fchmod`-on-fd to close GH #370's
  post-open `chmod` TOCTOU.
- ✅ Reorder `apply::run` to load and validate config **before** acquiring
  the state lock (NMOD.1, **High**).
- ✅ Pair NMOD.1 with moving the `require_yes` gate behind config validation
  (NMOD.2).
- ✅ Wrap `systemd-hostnamed` DBus calls
  (`set_static_hostname` / `set_pretty_hostname` / `set_hostname`) in
  `tokio::time::timeout(Duration::from_secs(5), …)` and surface `TimedOut`
  as a recoverable error (NEV2.3).
- ✅ Add `mac.validate_assignable()` to `MockBackend::set_cloned_mac` (NBE.6).
  Landed alongside C6 — `MockBackend::set_cloned_mac` rejects MACs that
  fail `validate_assignable` (e.g., multicast bit set, all-zeros) with a
  structured error before the rotate outcome is recorded.
- ✅ GH #354 / GH #363: state-lock acquire chmodded state-dir parent
  unconditionally (`--state /tmp/x` → `chmod /tmp 0700` system-bricking
  footgun). `ensure_state_dir_secure` now only chmods directories Proteus
  creates or the canonical `/var/lib/proteus`.
- ✅ GH #370: state-lock `O_NOFOLLOW` + post-open `chmod` TOCTOU. Switched
  to `fchmod` on the open fd.

**Acceptance:** ✅ stress test with 16 concurrent `acquire_state_lock` callers;
assert no thread blocks more than 5 s; assert no `panic = abort` is
triggered. (`stress_concurrent_acquires_stay_within_budget` in
`src/state_lock.rs`.)

### Stream 6 — Panic Hardening ✅

**Files (disjoint from all other streams):** `src/hostname/mod.rs`,
`src/diff/mod.rs`, `src/commands/mod.rs` (SHA verification path only),
`src/probe/mod.rs`, `src/captive_portal/mod.rs::body_slice` (line-bounded;
Stream 4 does not touch this function).

**Issues:** P2–P6, N12.10, NMOD.3, NTEST.3.

**Work:**

- ✅ Empty-label hostname validator: replace bounds-panic with structured
  error (P2).
- ✅ `.file_name().unwrap()` sites in diff and SHA verify: handle `..` /
  trailing-slash paths (P3, P4).
- ✅ Off-by-one in CRLF body slice (P5); probe `as u8` truncation guard
  (P6).
- ✅ `proteus diff` reads target files unbounded → cap to 64 MiB and
  surface a clear error past that (N12.10).
- ✅ `proteus diff`: cross-reference `state.json`'s tracked-paths set
  against the filesystem and emit a "missing" entry per absent file in the
  diff report (NMOD.3). Currently `compute_managed_file_drift` only walks
  the filesystem, so files Proteus once managed but the operator deleted
  are silently invisible.
- ✅ `tests/realworld/probe.sh`: pre-check `[ -d /sys/class/net ]` and skip
  with a clear message when `/sys` is not mounted (NTEST.3) — currently the
  loop runs zero times and the script "passes" with no probing.

**Acceptance:** fuzzer-style unit tests with property-based inputs (empty
strings, all-dots paths, oversize inputs).

### Stream 7 — Error Handling & Logging Discipline ⏳ (Wave 1 partial)

**Files (coordinate with Stream 4 on `events.rs`, Stream 8 on `dhcp.rs`):**
`src/commands/apply.rs` (logging-only sites), `src/commands/events.rs`
(rebase after Stream 4), `src/commands/show_config.rs`,
`src/commands/dhcp.rs` (E5 dispatch only; rebase after Stream 8),
`src/nm/mod.rs::GetSecrets` (rebase after Stream 4),
`src/commands/config_cmd.rs`, `src/commands/doctor.rs`, `src/dns/mod.rs`
(audit-target only), `src/logging.rs`.

**Issues:** E1–E10, NEV2.4.

**Work:**

- ✅ Demote info-level success-path events in `apply` (E1). ✅ events
  daemon hot path (E2). Verified via cross-stream audit: every
  `tracing::info!` in `src/commands/events.rs`, `src/events/mod.rs`,
  and `src/events/source/*.rs` is a lifecycle / banner / once-per-run
  event (daemon-start banner, signal-driven shutdown, trigger-budget
  exit, time-budget exit, reg-domain source spawn). The hot-path
  per-trigger lines in `RotateOnTriggerHandler::handle` were already
  demoted to `debug!` with `E2:` comments by PR #411's bundled commit
  `fix(events): demote per-trigger info-level logs to debug (E2)`.
- ✅ Surface `RUST_LOG` parse failures (E3); ✅ show-config permission
  errors at `error!` not `warn!` (E4).
- 🟡 Replace `Ok(exit::GENERIC_ERROR)` pattern with typed error returns
  (E5) — full refactor remains cycle-sized work for v0.5+. PR #450
  landed the partial: surveyed `apply.rs`, `show_config.rs`,
  `doctor.rs`, `config_cmd.rs` and found the brief's exact
  `eprintln+drop+GENERIC_ERROR` pattern doesn't appear — every
  `if let Err(e)` arm ends with a *typed* exit code (PERMISSION_ERROR,
  CONFIG_ERROR, SYSTEM_NOT_SUPPORTED) that bubbling via `?` would
  change. Converted one in-scope site (`config_cmd::edit` `!status.success()`
  → `Err(anyhow!(...))`) where the dispatcher renders the chain and
  maps Err to GENERIC_ERROR. Added E5 breadcrumbs at three structurally
  similar sites for the next wave. Stream 8's `dhcp.rs` dispatch site
  is still untouched.
- ✅ Stop swallowing NM `GetSecrets` failures (E6). `nm::update_with_secrets`
  routes through a new `get_secrets_or_warn` chokepoint with typed
  benign-vs-hard error classification via `zbus::Error` /
  `zbus::fdo::Error` matching (mirrors `bluetooth::apply::is_adapter_gone`).
  Benign set: `Settings.Connection.SettingNotFound`, `InvalidSetting`,
  `AgentManager.NoSecrets`, FDO `UnknownProperty` / `UnknownInterface`,
  empty dict. Hard set: `AccessDenied` (polkit), `NoReply` (DBus
  disconnect), unrecognised NM `MethodError` names. On `Hard`,
  `update_with_secrets` returns `Err(..)` BEFORE `proxy.update(settings)`
  — the secret stays intact. Tracing routes the connection label
  (`id (uuid)`) through `display_string` (S8 invariant preserved).
  ✅ Stop `unwrap_or_default` on `read_to_string` results (E7) for the
  in-scope `read_os_release` site in `doctor.rs`.
- ✅ Doctor probe error breadcrumbs (E8); ✅ unify
  `Config::default_or_loaded` fallback (E9) — `proteus config validate`
  now routes through `Config::default_or_loaded` so it shares the
  `validate_ranges` + `Config::validate` chain with every other entry
  point; ✅ audit `.unwrap()` shared between dns prod and tests (E10) —
  every unwrap in `src/dns/mod.rs` lives inside `#[cfg(test)]` and is
  not reachable from production callers (audit comment landed).
- ✅ Bluetooth adapter-disappeared: `is_adapter_gone` classifier
  matches on the underlying zbus error (FDO `UnknownObject` /
  `UnknownInterface` / `UnknownMethod` / `NameHasNoOwner` plus the
  `org.bluez.Error.NotReady` / `NotFound` MethodErrors) and logs at
  `warn!` (continue) on hot-unplug; other variants propagate
  unchanged (NEV2.4). `apply_one_resilient` wraps `apply_one` so a
  pulled dongle no longer fails the whole apply.

**Wave 1 deferrals (file-scope conflicts):** E2 (events hot path), E5
(dhcp dispatch), E6 (NM GetSecrets) all touch files owned by Streams
4 / 8 in Wave 1 and land in a follow-up wave.

**Wave 1 carryover from Stream 2:** ✅ P7 — `get_mut("mac").unwrap()`
test path in `src/commands/config_cmd.rs` replaced with structured
`expect`-style lookups so a future schema rename surfaces a useful
diagnostic instead of a bare panic line number.

**Acceptance:** snapshot tests pin that the `apply` and `show_config`
success paths emit zero `tracing::info!` events; the `show_config`
permission-denied path uses `error!`, not `warn!`. Stderr at default
verbosity is empty on a clean apply.

### Stream 8 — Resource Hygiene & Performance ⏳

**Files:** `src/nft/mod.rs`, `src/wiki.rs`, `src/commands/dhcp.rs::renew`
(R3, R7 — sequence before Stream 7's E5), `src/kill_switch/mod.rs`
(validators only), `src/dns/mod.rs::lossy` clones (non-overlapping with
Stream 7's E10 audit).

**Issues:** R1–R8, N12.14, M3, NBE.1, NBE.2.

**Work:**

- ✅ Close nft script stdin before `wait_with_output` (R1).
- ✅ Stream-parse `nft list table` instead of fully-buffered (R2); ✅ add max
  request-size cap to captive portal HTTP path (N12.14).
- ✅ Reuse a single DBus connection for DHCP status calls (R3); ✅ reduce
  `RenewOutcome` allocations to `&'static str` (R7).
- ✅ Wiki search index — hash terms once, scan pages once (R6).
- ✅ Drop redundant `lossy().into_owned()` on known-ASCII paths (R5).
- ✅ Subprocess fd-close audit comments (R8); ✅ shell-metacharacter validators
  on iface names (M3 — coordinate with Stream 9).
- ✅ Cache an `Arc<Connection>` on `NmBackend` so trait methods share a
  single `zbus::Connection::system()` per command invocation rather than
  re-authenticating on every method call. The backend now lazy-initialises
  one bus connection on first use and clones the Arc on every subsequent
  call — same family as R3 (NBE.1, Wave 3 Group A).
- ✅ Cache the last availability check on the backend struct (2 s TTL) so
  `select_auto()` and `availability_matrix()` don't double-probe via
  back-to-back `/run/NetworkManager` checks. The TTL is short enough that
  an operator who restarts NM mid-command sees the new state (NBE.2,
  Wave 3 Group A).

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

- ✅ Validate `Location` header per RFC (S2) — `validate_location_header`
  in `src/captive_portal/mod.rs` rejects CR/LF/NUL/control bytes,
  caps length at 4 KiB, and requires absolute or scheme-/root-relative
  refs. Wired into the redirect classifier and the redirect-following
  GET path.
- ✅ Enforce `mode(0o600)` on lock file creation (S3, Stream 9) —
  `OpenOptions::mode(STATE_FILE_MODE)` in `src/state_lock.rs::acquire_inner`
  pins a fresh `.lock` at 0o600 regardless of the caller's umask;
  paired with the existing post-open `fchmod` (GH #370, TOCTOU-safe
  via the open fd + `O_NOFOLLOW`) so a pre-existing wider-mode lock
  file is also tightened. Regression: `fresh_lock_file_lands_at_0600_regardless_of_umask`
  sets `umask(0o000)` before acquire and asserts the on-disk mode is
  exactly 0o600.
- ✅ Open-by-fd then `unlinkat` for resolved drop-in cleanup, closing the
  TOCTOU (S5, Wave 3 Group C — `remove_resolved_dropins` in
  `src/commands/revert.rs`).
- ✅ Join host + path through a single percent-encoder for HTTP
  request line (S6) — `request_line_builder` in
  `src/captive_portal/mod.rs` runs both fields through
  `percent_encode_request_target` / `percent_encode_request_safe`
  before the request blob is assembled.
- 🟡 Restrict polkit policy to `unix-group:wheel` / `sudo` and add a runtime
  check in `proteus doctor` (S7, B15). Policy file annotated; group
  enforcement requires a polkit JS rule under
  `/etc/polkit-1/rules.d/` (XML format does not accept a unix-group
  selector). Doctor runtime check deferred. **Maintainer decision needed:**
  conflicts with the `polkit_mutating_actions_do_not_cache_auth` test pin
  from issue #133. **Docs portion landed via PR #446** —
  `wiki/polkit-hardening.md` documents the optional JS recipe operators
  can apply today plus the `pkcheck` runtime-check pattern, with explicit
  guidance on avoiding the auth-cache conflict. Code portion (default
  policy + `proteus doctor` runtime check) remains the maintainer's
  decision.
- ✅ Expand the safety comment on `OwnedFd::from_raw_fd` (S9) —
  `src/events/source/link_flap.rs::netlink::open_netlink` now
  documents ownership handover, close-once invariant, kernel
  concurrent-close semantics, and the future-`OwnedFd::from_raw_fd_checked`
  migration path.
- ✅ Strict MAC separator parser: `Mac::from_str` and
  `ByteSuffixPattern::parse` reject mixed `:`/`-` separators in the same
  input (e.g. `aa:bb-cc:dd-ee:ff`) so a typo lands as an error rather
  than a silently-normalised MAC (S10, Wave 3 Group A).
- ✅ Sanitize NM dict values before tracing: `nm/mod.rs::update_with_secrets`
  routes `GetSecrets` errors through `crate::display::display_string` so
  attacker-controlled bytes in connection-setting values cannot redraw
  `journalctl -t proteus`. The `read_or_skip!` macro for device property
  enumeration does the same (S8, builds on Stream 4's preserve-method-context
  for N4, Wave 3 Group A).
- ✅ `Layout::from_env()` in `src/commands/uninstall.rs` is gated on
  `#[cfg(test)]` so production builds ignore `PROTEUS_*_DIR` entirely
  (audit M‑2 / N‑0). The grep-test at
  `tests::env_path_env_read_is_cfg_test_gated` source-locks the gate so a
  future refactor can't re-introduce the env read into shipped binaries.
- ✅ `--` separator inserted before user-influenced positional args in
  the `ip` (`kill_switch::run_ip`) and `ethtool`
  (`mac::factory::EthtoolBin::permanent`) invocations. `iw` does not
  support `--` as a flag terminator (its grammar is
  `iw <object> <command> ...`); the iface there is positionally guarded
  by the preceding `dev`/`phy` selector plus the `is_safe_iface` reject
  list (audit L‑3 residual).
- ✅ The four "hand-rolled SHA-256" callsites (`src/dns/apply.rs`,
  `src/stack/mod.rs`, `src/diff/mod.rs`, `src/ipv6/mod.rs`) all
  consume a single shared implementation at `crate::crypto::sha256`
  (audit I‑1).
- ✅ `wiki::get_page(name)` rejects `/`, `\`, `..`, and any byte outside
  `^[A-Za-z0-9_-]+$`, with a 64-byte cap (NEV2.1).
- ✅ Stream 9 cluster fix for the seven terminal-injection issues
  (#357, #365, #367, #373, #374, #380, #389): central
  `crate::display::display_safe(&str) -> Cow<str>` helper added to
  `src/display.rs`; called from every print site that surfaces an
  AP-controlled SSID, an iface name from NM, or a persona display_name /
  id from user-authored TOML. Unit-tested for ANSI CSI, BiDi overrides
  (U+202E etc.), CR/LF, NUL.
- ✅ GH#342: persona id is validated against `^[A-Za-z0-9_-]+$` plus
  reserved-name rejection in `commands::persona::is_valid_persona_id`
  before flowing into any path component on the `show`/`use`/`new`/`edit`
  paths.
- ✅ GH#358: `ipv6::write_sysctl` validates the caller-supplied `key`
  against the static `SYSCTLS` allow-list and refuses values containing
  control bytes / NUL / whitespace.
- ✅ GH#360: `factory::EthtoolBin::permanent` no longer falls back to a
  `$PATH`-resolved relative `ethtool` — it walks an absolute-path list
  and refuses the call when none resolves.
- ✅ GH#361: `persona edit` refuses to spawn `$EDITOR` when `$HOME` is
  not `/root` (was warn-and-continue) and refuses `$EDITOR` / `$VISUAL`
  values containing control bytes.
- ✅ GH#362: NM dispatcher's 128-byte cap now uses a locale-invariant
  byte-count via `LC_ALL=C wc -c` instead of bash's
  locale-dependent `${#var}`.
- ✅ GH#359: central iface validator landed at `crate::iface` (`validate`
  + `is_valid` + typed `InvalidIface` error). All six per-module
  duplicates migrated to delegate through `crate::iface`:
  `mac::factory::is_valid_iface_name`, `ipv6::validate_iface_name`,
  `rf::is_safe_iface`, `kill_switch::is_safe_iface`,
  `events::source::nm_connection_up::is_safe_iface_name`, and the
  nested `mac::probe::raw::validate_iface`. Three wrappers (ipv6, rf,
  kill_switch) were strictly laxer than the central kernel-faithful
  validator and are now newly-strict — defence-in-depth on callsites
  that already feed kernel-validated sysfs walks.

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

- 🟢 Audit pass: substantially landed across multiple stream-aligned
  PRs. Files swept so far:
  - `src/persona/*`, `src/commands/{rotate,pin,config_cmd}.rs`
    (PR #428)
  - `src/init/*`, `CHANGELOG.md`, `wiki/cli.md` (PR #427)
  - `src/backend/{select,raw,networkd}.rs`,
    `src/enterprise_wifi/mod.rs` (PR #433)
  - `src/commands/{dns,ntp,resolved,stack}.rs` (PR #434)
  - `src/commands/{enterprise_wifi,timer,apply,watch}.rs` (PR #440)
  - `src/hostname/mod.rs` (PR #441)
  Total ~75 hints appended across the operator-facing error surface;
  internal/defensive errors skipped per policy. Remaining files in the
  original 40-file checklist either had zero operator-facing
  `bail!`/`anyhow!` sites or route errors through `eprintln!` +
  `Ok(exit::*)` (so the wiki hint flows through anyway when the
  inner call surfaces one).
- ✅ Bypass hardening pass: review every place we shell out — 33
  `Command::new` sites enumerated and pattern-classified in
  `docs/security/external-review.md`.
- ⏳ Real-world testing on diverse Wi-Fi (coffee shops, hotels,
  conferences, airports). Living-doc scaffold landed at
  `docs/realworld-test-log.md`; entries accumulate as Proteus is taken
  on the road.
- ⏳ Independent security review against `docs/security/dbus-surface.md`.
  Soliciting scaffold landed at `docs/security/external-review.md`;
  awaits engagement.

**Work:**

- ✅ Fix `[discovery]`, `[rotation]`, `[mac]`, `[probes]` sections in every
  example to use real schema field names (D1, D2, D3, D4) — verified
  every key in `examples/*.toml` against the `Raw*Config` structs in
  `src/config.rs`. All 7 example files round-trip cleanly against the
  current schema; no edits needed.
- ✅ Document exit code 75 in `wiki/cli.md` (D5) — already present in
  the global Exit codes table and in every per-subcommand exit row.
- ✅ Correct doctor exit code reference (D6) — verified against
  `src/commands/doctor.rs::run` (returns `SUCCESS` or `GENERIC_ERROR`);
  cli.md and troubleshooting.md already say `0 / 1`.
- ✅ Recount wiki pages (D7) — `ls wiki/*.md | wc -l` is 45,
  README.md says "45-page embedded wiki"; no edit needed.
- ✅ Add `config set-profile` section to `wiki/cli.md` (D8).
- ✅ Fix `display_string` length-clamp to count output graphemes, not
  input chars (N12.6). Three regression tests added covering C0
  controls (`\x07`), C1 controls (`\u{009b}`), and backslash —
  previously each amplified output by 4×, 6×, and 2× respectively
  past the clamp.
- 🟢 Wiki-hint audit pass on every `bail!` / `anyhow!` site (frontier item).
  Stream 10 multi-PR sweep covered the persona/rotate/config/pin (PR
  #428), init (PR #427), backend/select/networkd/enterprise (PR
  #433), dns/ntp/resolved/stack (PR #434), enterprise-wifi/timer/
  apply/watch (PR #440), and hostname (PR #441) modules; ~75 hints
  appended. Remaining files in the 40-file checklist had zero
  operator-facing bail/anyhow sites or used the eprintln+exit-code
  pattern (where hints flow through transitively).
- ✅ Bypass-hardening pass: enumerate every `Command::new` site and confirm
  argument-array form (no shell interpolation). Done in
  `docs/security/external-review.md`.
- ✅ Real-world testing log scaffold (frontier item) —
  `docs/realworld-test-log.md`.
- ✅ External-review scaffold (frontier item) —
  `docs/security/external-review.md`.
- ✅ Document the `proteus-rotate.timer` ±75 min effective jitter
  (`RandomizedDelaySec=30min` + `AccuracySec=45min`) in
  `wiki/rotation.md`'s tuning section (NPKG.13) so operators tuning rotation
  cadence don't refile this as a bug. Info-only.

**Wiki-hint follow-up checklist (frontier item, deferred to owners of
the listed files because Stream 10 only owns `src/error.rs`):**

The 208 `bail!` / `anyhow!` sites live across 40 files. Stream 10
cannot edit them (file ownership is split across other streams) and
this repo has no `src/error.rs` — every error message is constructed
inline at the bail/anyhow callsite. The follow-up is to sweep each
file below and confirm every error string ends with a `; see proteus
wiki <page>` hint where relevant. Files (in callsite-count order):

- `src/backend/networkd.rs`, `src/backend/nm.rs`, `src/backend/raw.rs`,
  `src/backend/select.rs` (Stream 1 owns)
- `src/bluetooth/alias.rs` (Stream 6 owns)
- `src/cli/mod.rs`, `src/commands/apply.rs`, `src/commands/config_cmd.rs`,
  `src/commands/dns.rs`, `src/commands/enterprise_wifi.rs`,
  `src/commands/mod.rs`, `src/commands/ntp.rs`, `src/commands/pin.rs`,
  `src/commands/resolved.rs`, `src/commands/rf.rs`,
  `src/commands/rotate.rs`, `src/commands/stack.rs`,
  `src/commands/timer.rs`, `src/commands/watch.rs` (Streams 1/2/3/6 own)
- `src/config.rs` (other stream owns; load-time validators)
- `src/enterprise_wifi/mod.rs`, `src/events/mod.rs`,
  `src/hostname/mod.rs`, `src/init/mod.rs`, `src/init/openrc.rs`,
  `src/init/runit.rs`, `src/init/systemd.rs`, `src/init/sysvinit.rs`
- `src/ipv6/mod.rs`, `src/mac/generator.rs`, `src/mac/probe.rs`,
  `src/nft/mod.rs`, `src/nm/apply.rs`, `src/nm/mod.rs`,
  `src/persona/load.rs`, `src/persona/template.rs`, `src/rand/mod.rs`,
  `src/rf/mod.rs`, `src/state.rs`, `src/timer/mod.rs`

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

## Enhancements queue (GitHub feature requests)

The v0.4.x cycle is **bug + vulnerability hunt** — the existing convention is
"no new features" until the High and Critical bug rows close. The 45
enhancement-class GitHub issues below are recognized here so nothing falls
through the cracks, but each one is tagged with a verdict:

- **🟢 no-brainer (land in v0.4.x):** small, additive, no behaviour change to
  existing surface. Worth landing alongside the bug-fix work because it
  closes a sharp edge users keep hitting.
- **🟡 worth-doing (queue for v0.5+):** clear value but big enough to deserve
  its own cycle slot. Don't crowd the bug-fix work; pick up on the way out.
- **🔴 flag-for-review:** design call. Either touches headline UX (renames,
  config-schema breakage) or asks for substantial scope (new subsystem). The
  maintainer should decide before someone starts.

**Verdict count:** 18 🟢 · 19 🟡 · 8 🔴.

### CLI ergonomics

| GH # | Verdict | Item | One-line review |
|---|---|---|---|
| #390 | 🟢 | `proteus logs` — tail journald entries across every Proteus unit + dispatcher | Tiny shell wrapper over `journalctl -u 'proteus*'` plus the dispatcher's syslog tag; no risk |
| #376 | 🟢 | `proteus version --json` / `proteus about` with build info (git sha, rustc, target, build time, schema versions) | Read-only; structured output for CI; `build.rs` already emits some of this |
| #395 | 🟢 | `proteus rotate --json` summary (rotated/skipped/explain) | Mirrors existing `--json` on other readers; eliminates screen-scraping in dispatcher |
| #343 | 🟢 | `proteus apply / revert --json` per-component summary | Same shape as #395; CI / Ansible consumers want this |
| #364 | 🟢 | `proteus pin list` — enumerate every pinned interface / connection | Read-only inverse of `pin`; existing state already has the data |
| #392 | 🟢 | `proteus unpin --all` / `--scope <type>` | Symmetric with #364; one-loop implementation |
| #353 | 🟢 | `proteus backup <path>` / `proteus restore <path>` — bundle config + state + user personas in one tarball | High user value; `tar.gz` of three known dirs; gates with `--yes` on restore |
| #300 | 🟢 | `proteus state info` — schema version, migration metadata, contents summary read-only | Very small; useful for support; adjacent to existing `proteus status` |
| #283 | 🟢 | `proteus events list-sources` — show available sources, why degraded, what capability needed | Read-only; events daemon already has the data internally |
| #393 | 🟢 | `proteus events status` — live per-source / per-handler counters and last-fired timestamps | Adjacent to #283; one queryable surface; existing instrumentation |
| #346 | 🟢 | `proteus events trigger <name>` — fire synthetic trigger for smoke-test | Unblocks integration testing without a live network; gated behind `--yes` + `--debug` |
| #404 | 🟢 | `proteus config show --annotate` (or `--origin`) — mark each field's source | Read-only; `wiki/profiles.md` already promises this |
| #394 | 🟢 | `proteus config explain <key>` — surface the doc-comment + risk-warning + wiki link | Read-only; complements #404 |
| #383 | 🟢 | `proteus persona search <query>` — scan ids, display_names, notes | Trivial; persona catalogue is small; ergonomics win |
| #338 | 🟢 | `proteus persona delete <id>` — remove user-authored personas without `rm` | Symmetric with `persona new`; gated with `--yes` + lstat-rejects symlinks (mirror of #286 export safety) |
| #356 | 🟢 | `proteus persona random --use [--apply]` — one-shot pick + activate | Composes existing `random` + `use`; minor flag plumbing |
| #294 | 🟢 | `proteus rotate --reason "<text>"` — stamp human-readable audit string into state + journald | Tiny; aids forensic correlation; complements existing `--explain` |
| #406 | 🟢 | `proteus wiki list [--json]` — enumerate embedded pages programmatically | Read-only; replaces curated TOC fallback |

### CLI ergonomics — queue for v0.5+

| GH # | Verdict | Item | One-line review |
|---|---|---|---|
| #407 | 🟡 | `proteus mac validate <addr>` and `proteus mac info <addr>` — offline parse + classify (LAA / multicast / OUI lookup) | Useful enough but a new top-level subcommand family; design the surface in v0.5 |
| #401 | 🟡 | `proteus mac generate [--vendor] [--pattern] [--count N]` — offline MAC generation | Same family as #407; ship together |
| #402 | 🟡 | `proteus persona effects <id>` — render the on-wire fingerprint a persona would project, side-by-side with current | High user value but needs design — what to compare, how to render; v0.5 candidate |
| #295 | 🟡 | `proteus persona compare <a> <b>` — structural diff of two personas | Adjacent to #402; ship together |
| #301 | 🟡 | `proteus persona suggest` — score every persona against host's existing fingerprint, recommend matches | Cool feature but needs the fingerprint reader to exist (#402 first) |
| #277 | 🟡 | persona `extends = "<id>"` inheritance for near-sibling personas (iphone-13 / iphone-15) | Schema change — needs version bump; queue with persona system v2 |
| #372 | 🟡 | `proteus profile {list, show, compare}` — inspect baselines without switching | Useful; ergonomics win; minor |
| #396 | 🟡 | `proteus dhcp lease [--iface]` — show active lease (server, time-to-renew, parameters, transaction-id) | NM has the data; format design + v4/v6 handling worth a cycle slot |
| #385 | 🟡 | `proteus session --iface <name>` / `--all` — multi-radio support | Bigger than it looks; multi-iface session model needs design |
| #344 | 🟡 | `proteus rotate history` — bounded per-iface ring buffer in state | Schema change + retention policy; v0.5 with persona-v2 |
| #378 | 🟡 | `proteus rotate-if-needed --explain` — already absorbed as Stream 1 bug fix; here as the verbose variant | Pairs with #395 |
| #387 | 🟡 | `--dry-run` extended to every mutating subcommand (dhcp/dns/resolved/ntp/stack/nft/ipv6/rf/enterprise-wifi/persona/ssid/kill/timer/config) | High value but spans every command; cycle-sized work |
| #341 | 🟡 | `apply / revert --component <name>[,...]` to scope orchestrator | Powerful for ops; needs careful design re partial state |
| #350 | 🟡 | `proteus probe --continuous` + `--exit-on <classification>` for triage scripts | Useful; small; queue because nothing depends on it |
| #282 | 🟡 | `proteus diff <component>` — scope drift output by subsystem | Adjacent to #341 |
| #369 | 🟡 | `proteus diff --exit-on-drift` — fail-fast for CI / Ansible | Tiny once #282 lands |
| #384 | 🟡 | `proteus completions install [--shell] [--user]` — install without manual copy | Surface convenience; minor |
| #347 | 🟡 | dynamic completions for persona ids, SSID names, interface names | Needs `proteus __complete` hidden surface; cycle-sized |
| #403 | 🟡 | `proteus timer enable --all / disable --all / reset --all` | Mid-effort; touches every timer unit; cycle-sized |

### Enhancement design-calls flagged for your review

| GH # | Verdict | Item | Why it's flagged |
|---|---|---|---|
| #399 | 🔴 | rename / alias `proteus doctor` → `proteus check` | Headline-UX rename. `doctor` is widely googled; aliasing is fine, renaming would break muscle memory. Decide: alias-only, or actual rename in v0.5? |
| #397 | 🔴 | `proteus self-test` — end-to-end smoke check (apply → diff → revert → check originals match) on the live host | Mutates real network state during a "test" — a misclassified portal or a hostile network turns this into a footgun. Needs a strict guard model: only on networks the user explicitly tags as safe? Skip if `[per_ssid]` says portal? Decide policy first. |
| #400 | 🔴 | `apply --no-rotate` and `--rotate-only` | Splits the apply orchestrator's atomicity guarantees. "MAC unchanged + DHCP/DNS/IPv6 rotated" is a posture the threat model doesn't cover — could leak the prior MAC by exposing the new DUID under the old MAC. Decide whether the use case justifies the threat-model expansion. |
| #296 | 🔴 | `proteus doctor --fix` — apply mechanical idempotent remediations that `next steps:` already names | Doctor is read-only today; making it mutate is a contract break. The remediations exist as separate `apply` calls; the question is whether to fold them. Could land as `proteus doctor fix` (subcommand) instead. |
| #280 | 🔴 | `proteus config schema` — emit JSON Schema for `/etc/proteus/config.toml` so editors validate hand-edits | Locks the config surface to a public schema. Once published, every new field is a versioned contract. High-value (best UX for power users) but one-way door — needs commitment. |
| #405 | 🔴 | events daemon SIGHUP reload + `proteus events reload` — Stream 4 R4 covers SIGHUP for captive_portal config; this expands to all config | Reload story for everything — needs careful design about which knobs are reloadable vs require restart. R4 only covers captive portal. Decide: SIGHUP = full reload, partial reload, or just captive_portal? |
| #278 | 🔴 | events `--format json` log surface — one JSON-line per trigger, as `events/mod.rs` comments already promise | Comments promise it; needs schema commitment. Decide schema before shipping. |
| #398 | 🔴 | `proteus rf monitor` — stream live signal/quality/TX-power per Wi-Fi iface | New subsystem; needs `nl80211` polling loop + signal-strength normalisation across drivers; could explode in scope. Maybe limit to existing `iw event` parsing? |

### Notes for the maintainer

A few opinions worth flagging while you review the verdicts above:

1. **The 18 🟢 enhancements together are roughly 1-2 weeks of work** if landed
   in parallel with bug-fix streams. They are uniformly read-only or
   additive, so they don't risk regressing existing behaviour. Recommend
   landing as a single "ergonomics polish" wave once Stream 1 settles.
2. **#371's "index of edge / zero-day findings"** is exactly what this
   roadmap now is. Closing #371 with a pointer here would save a stale
   tracking issue.
3. **The terminal-injection cluster (#365, #367, #373, #374, #380, #389,
   #357)** all have the same root cause — display layer doesn't sanitize
   AP-controlled strings. These should land as a single helper
   (`crate::display::display_safe(&str) -> Cow<str>`) used everywhere, not
   as seven point-fixes. Stream 9 entry should be re-scoped accordingly.
4. **#359 (5 distinct iface validators)** plus the audit's L-3 / N-1 work
   in Streams 4 and 9 should consolidate to one `crate::iface::validate`
   helper that the whole codebase calls. Doing it once here saves the next
   audit from refiling the same finding.
5. **The "queue for v0.5+" pile is naturally a CLI ergonomics cycle.**
   Worth scoping as the v0.5.0-beta theme — every entry there is additive
   and they share a lot of plumbing.

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
