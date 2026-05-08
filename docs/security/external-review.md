# External security review — coordination scaffold

**Status:** open. No external reviewer has been engaged yet. This file is
the soliciting / tracking artifact; once a reviewer is engaged, every
finding gets a row in the table at the bottom.

The internal audits live alongside this file:

- `docs/security/dbus-surface.md` — full enumeration of every DBus
  surface the privileged binary touches. **Primary review artifact.**
- `docs/security/SECURITY-AUDIT-2026-05-07.md` — internal audit, May 2026.
- `docs/security/SECURITY-AUDIT-2026-05-07-followup.md` — follow-up
  notes / remediation status from the May 2026 audit.

The roadmap pin for this work is `docs/ROADMAP.md` Stream 9 ("Security /
Audit Follow-Through") and Stream 10's frontier item ("Independent
security review against `docs/security/dbus-surface.md`").

## Reviewer ask

We are looking for a reviewer who can answer the following, in priority
order:

1. **Privilege model.** Proteus runs as `uid=0` for mutating commands
   (gated by polkit) and as the invoking user for read commands. Are
   there *read* commands that consume privileged DBus surface in a way
   that lets an unprivileged caller exfiltrate operator-only data, or
   exercise side effects through DBus introspection?
2. **DBus contract.** Walk through every method/property/signal in
   `docs/security/dbus-surface.md` and flag any:
   - argument we accept from a remote (NM, BlueZ, hostnamed, systemd1)
     and pass to a local mutator without re-validation;
   - method whose error path leaks information to a less-privileged
     audience (journald → syslog → unprivileged readers);
   - signal whose handler triggers a mutating call without
     re-checking the originating identity.
3. **Subprocess hardening.** Every place we shell out (`Command::new`)
   should pass arguments as a fixed array, never a single shell-parsed
   string, and every interface name reaching such a call must be
   validated by `is_safe_iface`. The bypass-hardening checklist below
   is the exhaustive enumeration; please confirm or refute each row.
4. **Threat-model coverage.** `wiki/threat-model.md` and
   `wiki/security-checklist.md` describe what Proteus claims to protect
   against. Identify gaps between the claim and the implementation.

## Scope explicitly out of scope

- `cargo audit` results — already run in CI; reviewer may skim but not
  reproduce.
- The wiki content itself (44 pages) — readability / accuracy review is
  a separate ask.
- Cryptographic primitive choice — Proteus does not implement crypto;
  it consumes kernel / NM / wpa_supplicant / BlueZ randomness.

## Bypass-hardening checklist (Stream 10 frontier item)

Every `Command::new` site in `src/`, with a one-line claim about its
shape. Reviewer should confirm the claim or flag a discrepancy. As of
this scaffold (2026-05-08), the audit found 33 invocations across 23
files. Pattern claim per row:

- **A**: argument-array form (`.args([...])`), no shell, fixed-binary name.
- **B**: argument-array form, binary path is a const helper (`iw_bin()`
  etc.) that resolves to a hardcoded absolute path.
- **C**: argument-array form, binary name is `&editor` from
  `$VISUAL`/`$EDITOR` — accepted because the user opted in by setting
  the env var.
- **D**: argument-array form, args include `--` separator before any
  user-influenced positional (recommended; required after the audit
  L-3 residual fix).

| File | Line | Binary | Pattern | Validation gate | Notes |
|---|---|---|---|---|---|
| `src/commands/apply.rs` | 464 | `systemctl` | A | none needed (fixed args) | unit names are crate-internal constants |
| `src/commands/config_cmd.rs` | 141 | `&editor` | C | env-supplied path | `$VISUAL`/`$EDITOR`/`vi` |
| `src/commands/dns.rs` | 237 | `systemctl` | A | none needed | fixed args |
| `src/commands/ntp.rs` | 224 | `systemctl` | A | none needed | fixed args |
| `src/commands/persona.rs` | 396 | `&editor` | C | env-supplied path | `$VISUAL`/`$EDITOR`/`vi` |
| `src/commands/resolved.rs` | 220 | `systemctl` | A | none needed | fixed args |
| `src/commands/revert.rs` | 221 | `program` (variable) | A | call-site enumerated | wrappers feed fixed `program` strings |
| `src/commands/session.rs` | 433 | `systemctl` | A | none needed | fixed args |
| `src/commands/session.rs` | 449 | `systemctl` | A | none needed | fixed args |
| `src/commands/stack.rs` | 222 | `sysctl` | A | none needed | fixed args |
| `src/commands/timer.rs` | 296 | `journalctl` | A | timer-name validated upstream | unit name from fixed map |
| `src/commands/timer.rs` | 337 | `systemctl` | A | timer-name validated upstream | unit name from fixed map |
| `src/commands/timer.rs` | 400 | `systemctl` | A | timer-name validated upstream | unit name from fixed map |
| `src/commands/timer.rs` | 413 | `systemctl` | A | timer-name validated upstream | unit name from fixed map |
| `src/commands/timer.rs` | 427 | `systemctl` | A | timer-name validated upstream | unit name from fixed map |
| `src/commands/uninstall.rs` | 232 | `program` (variable) | A | call-site enumerated | wrappers feed fixed `program` strings |
| `src/dns/mod.rs` | 159 | `systemctl` | A | none needed | fixed args |
| `src/dns/mod.rs` | 193 | `ss` | A | none needed | fixed args |
| `src/ipv6/mod.rs` | 210 | `sysctl` | A | none needed | fixed args |
| `src/kill_switch/mod.rs` | 187 | `ip` | D | `--` separator already in place | first positional is `--`, then args |
| `src/mac/factory.rs` | 143 | `bin` (`ethtool`) | A | `is_safe_iface` upstream | `iface` validated before reaching here |
| `src/nft/mod.rs` | 234 | `nft` | A | none needed | fixed args / stdin |
| `src/nft/mod.rs` | 259 | `nft` | A | none needed | fixed args |
| `src/nft/mod.rs` | 325 | `nft` | A | none needed | reads ruleset from stdin |
| `src/ntp/mod.rs` | 168 | `systemctl` | A | none needed | fixed args |
| `src/rf/mod.rs` | 127 | `iw` | B | `is_safe_iface` upstream | iface gated before invocation |
| `src/rf/mod.rs` | 144 | `iw` | B | `is_safe_iface` upstream | iface gated; `mbm` is integer |
| `src/rf/mod.rs` | 162 | `iw` | B | none needed | `iw reg get` is fixed |
| `src/rf/mod.rs` | 186 | `iw` | B | `is_safe_iface` upstream | iface gated |
| `src/rf/mod.rs` | 442 | `iw` | B | `is_safe_iface` upstream | iface gated |
| `src/rf/mod.rs` | 458 | `iw` | B | `is_safe_iface` upstream | iface gated |
| `src/rf/mod.rs` | 579 | `dmesg` | A | none needed | fixed args |
| `src/rf/mod.rs` | 633 | `iw` | B | `is_safe_iface` upstream | iface gated |

**Reviewer task:** for every row above, please mark `confirmed`,
`flagged`, or `unsure-needs-source-walk`. Append a `Verdict` column to
the table when delivering review notes.

**Open follow-up from the audit (cross-reference `docs/ROADMAP.md`):**

- Insert `--` before user-influenced positional args in **every** `iw`
  / `ip` / `ethtool` invocation, not just `kill_switch`. Audit L-3
  residual; Stream 9 carries the work.
- `is_safe_iface` blocks shell metacharacters but does not block
  `iface = "-h"` flag-parse confusion. The `--` insertion closes that.
- Re-audit any `Command::new(program)` site where `program` comes from
  state on disk rather than a const. Currently `revert.rs:221` and
  `uninstall.rs:232` accept a variable but call-sites only feed fixed
  strings; verify by code inspection.

## Findings table

To be populated by the external reviewer. One row per finding.

| Date | Severity | Finding | Affected paths | Reviewer | Status | Resolution |
|---|---|---|---|---|---|---|
| _(pending engagement)_ | | | | | | |

Severity uses the same scale as `docs/MAINTAINER-FOLLOWUPS.md`:
**Critical / High / Medium / Low / Info**. Status:
**open / acknowledged / fixed-in-vX.Y.Z / wontfix-with-rationale**.

## Engagement log

| Date | Reviewer / org | Scope | Outcome |
|---|---|---|---|
| _(pending)_ | | | |

## Cross-refs

- `docs/security/dbus-surface.md` — primary review artifact.
- `docs/security/SECURITY-AUDIT-2026-05-07.md` — most recent internal
  audit baseline.
- `docs/MAINTAINER-FOLLOWUPS.md` — open security-related follow-ups.
- `docs/ROADMAP.md` — Stream 9 and Stream 10 frontier items.
- `wiki/threat-model.md` — what Proteus claims to defend against.
- `wiki/security-checklist.md` — operator-facing hygiene checklist.
