# Roadmap — v0.4 "Beta: bug + vulnerability hunt"

The v0.3 cycle closed at `v0.3.2-alpha` with **83 ✅ / 1 💭**. This
cycle is **bug-fixing and vulnerability-hunting only**: every code
change in `main` between this cycle's start and the `v0.4-beta` tag
must be either a fix for a beta-intake issue or a regression test
locking that fix in place.

**No new features land.** New-feature proposals queue under "v0.5
ideas" via the issue-tracker `proposal` label; they will not block
beta.

The beta intake doc is the single source of truth for scope, severity
rubric, and triage: see [`BETA-INTAKE.md`](BETA-INTAKE.md).

For the v0.3 cycle's history see
[`ROADMAP.md`](ROADMAP.md). For per-version release notes see
[`../CHANGELOG.md`](../CHANGELOG.md).

## Status legend

- ✅ Resolved (fix landed, regression test in place)
- 🚧 In progress (fix has a PR open)
- ⏳ Triaged, accepted, awaiting fix
- 🔍 Reported, awaiting triage
- 💭 Deferred to v0.5

## Cycle exit criteria

These are reproduced from `BETA-INTAKE.md` for visibility. `v0.4-beta`
ships when **all** hold:

1. Zero open issues with severity `critical` or `high`.
2. Severity `medium` issues either fixed or accepted-for-v0.5.
3. `cargo clippy --all-targets` clean.
4. All 830+ lib tests pass.
5. `tests/integration/run.sh` green on Fedora 43+, Debian 12+,
   Alpine 3.19+, Arch.
6. `docs/security/v0.4-beta-findings.md` enumerates every accepted
   issue, the fix, and the regression test.
7. The roadmap and README update to point at the v0.5 cycle.

## Hunt scope

Reproduced from `BETA-INTAKE.md` for completeness:

1. Code paths under every `proteus help` subcommand.
2. Configuration handling (TOML schema, env vars, per-SSID resolver,
   persona load + schema check).
3. Privileged DBus surface (catalogued in `docs/security/dbus-surface.md`).
4. State invariants (sacred-originals, advisory flock, schema-version
   migration ladder).
5. Filesystem I/O (atomic writes, drop-in semantics, state quarantine).
6. Threat-model claims in `wiki/threat-model.md` and
   `wiki/personas.md`.
7. Distro-support claims in `wiki/distro-support.md`.
8. Real-world testing via `tests/realworld/`.

## Triage queue

Issues land here once accepted. The list is empty at the start of the
cycle; populated as the hunt progresses.

### 🔴 Critical
_(none open)_

### 🟠 High
_(none open)_

### 🟡 Medium
_(none open)_

### 🟢 Low
_(none open)_

### 💭 Deferred to v0.5
_(none open)_

## Findings record

Every accepted issue lands a row in `docs/security/v0.4-beta-findings.md`
once fixed: id, title, severity, root cause, fix summary, regression
test name, the `wiki/*.md` page that gained a hint (if any). The
findings doc is the artifact external reviewers should read against —
not the source.

The bypass-hardening pass for v0.3 already produced two regression
fixes (`#BH-1`, `#BH-2`) plus a new-shellouts-go-through-`crate::process`
discipline (`#BH-3`). Treat that doc
(`docs/security/bypass-hardening-pass.md`) as the v0.4 findings-doc
template.

## What's not in this cycle

- **New features.** Persona expansions, additional backends, new
  fingerprint vectors — all queue for v0.5.
- **API breakage.** The `Config` schema, state-file format, and CLI
  surface stay frozen for the cycle. A v0 → v1 migration step has
  shipped; we want it exercised, not extended.
- **Distro packaging.** Maintainer-side work (Debian sponsor, Copr
  upload, AUR submission) is not gated on this cycle. If a
  distro-side bug surfaces, it lands here as a normal triage item.

## How to help

Open issues with the `v0.4-beta-intake` label. The intake form +
severity rubric live in [`BETA-INTAKE.md`](BETA-INTAKE.md). Real-world
testing on diverse Wi-Fi (cafés, hotels, conferences, airports) is the
single highest-leverage contribution right now.
