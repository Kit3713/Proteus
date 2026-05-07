# Beta intake — v0.4 cycle

**Goal:** ship `v0.4-beta` with zero open critical/high findings and a
published findings record. Bug fixing and vulnerability hunting only.
**No new features** until this cycle closes.

This document is the single source of truth for the v0.4 cycle: scope,
intake form, severity rubric, triage cadence, and what is explicitly
out of scope. All other docs (the v0.3 ROADMAP, README "what's next",
CHANGELOG entries) point here.

## In scope

Everything Proteus ships in `v0.3.2-alpha` is fair game:

1. **Code paths** — every command in `proteus help`, every backend
   impl (`nm` / `networkd` / `raw`), every parser, every shellout,
   every state-mutation site.
2. **Configuration handling** — TOML schema validation, config-file
   reload, the per-SSID resolver, persona load + schema check, env
   var parsing (`RUST_LOG`, `NO_COLOR`, `PROTEUS_LOCK_TIMEOUT_MS`,
   `EDITOR`).
3. **Privileged DBus surface** — every method called against
   NetworkManager / systemd-resolved / BlueZ / timesyncd /
   networkd / nl80211 (catalogued in
   `docs/security/dbus-surface.md`).
4. **State invariants** — sacred-originals (cached factory MACs +
   pre-Proteus hostnames must never be overwritten), advisory flock
   contention, schema-version migration ladder.
5. **Filesystem I/O** — atomic write semantics (mode 0o600,
   `O_CREAT | O_EXCL`, parent fsync), state quarantine on JSON
   parse failure, drop-in writes under `/etc/systemd/*.conf.d/`.
6. **Threat model claims** — assertions in `wiki/threat-model.md` and
   `wiki/personas.md` should be falsifiable with experiment. If a
   stealth persona doesn't actually cover what it claims to cover,
   that is a finding.
7. **Distro support claims** — every entry in `wiki/distro-support.md`
   should be tested on at least one host.
8. **Real-world testing** — `tests/realworld/` checklist runs on
   coffee shop / hotel / conference / airport networks. Field
   findings drop straight into this intake.

## Explicitly out of scope

These are the same boundaries `wiki/threat-model.md` documents. Items
filed under any of them are closed `out-of-scope` with a pointer:

- **TLS / browser fingerprint** (JA3/JA4, font/canvas/WebGL).
  Proteus is L2-L4 + DHCP/mDNS/RF. Use Tor Browser, librewolf,
  Brave's randomization.
- **Wireshark-class payload-content analysis**. Persona shapes
  *headers* and *protocol fingerprints*; payload content is the
  app-layer's responsibility.
- **DNS resolution policy beyond the ECS-strip drop-in.** Use
  dnscrypt-proxy / NextDNS / AdGuard Home / Pi-hole.
- **Tracker blocking.**
- **Traffic correlation defenses** — Tor / Mullvad layer.
- **SSH client fingerprint (HASSH).** Your `ssh_config` is yours.
- **Hardware-baked RF fingerprints** (oscillator drift, DAC
  nonlinearity, IQ imbalance) — physically impossible without a
  hardware swap.
- **Telemetry / update checks / analytics.** None ever, by design.
- **New features.** Anything in this category is filed under "v0.5
  ideas" — see `docs/V0.5-IDEAS.md` if it exists, or the issue
  tracker's `proposal` label otherwise.

## Severity rubric

Use these definitions when filing or triaging. The first matching
class wins.

| Severity | Definition | SLA |
|---|---|---|
| 🔴 **Critical** | Sensitive data leak (PSK, IPSec key, MAC of a different network), kernel-level privilege escalation, persistent change to a non-Proteus-managed file (CVE-class). | Cut a `v0.3.x` patch release out-of-cycle. |
| 🟠 **High** | Lost work for the operator (wiped state, bricked NM connection, undismissable kill-switch state); data corruption that survives `proteus revert`; a stealth persona that doesn't actually cover what it claims. | Block beta; fix before `v0.4-beta` tags. |
| 🟡 **Medium** | Crash / panic / `unreachable!` reachable from operator-controllable input; a quirky-distro feature that defers when it should apply (or vice-versa); incorrect `proteus diff` output. | Fix in beta if the change is bounded; defer to `v0.5` if it needs new architecture. |
| 🟢 **Low** | Cosmetic, documentation drift, harmless `tracing::warn` noise, suboptimal-but-correct behavior on a single distro. | Batch into the rolling bug-fix queue. |

## Intake form

File an issue with the `v0.4-beta-intake` label. Required fields:

```
**Severity (your guess):** [critical | high | medium | low]
**Component:** [config | persona | per-ssid | rotate | dhcp | ipv6 |
                dns | resolved | ntp | nft | rf | bluetooth | events |
                kill-switch | doctor | docs | other]
**Distro / kernel:** uname -srv + /etc/os-release
**Backend:** nm | networkd | raw  (from `proteus doctor`)
**Init:** systemd | openrc | runit | sysvinit
**Reproduction:** numbered steps, copy-pastable
**Expected:** what you thought would happen
**Actual:** what happened (paste verbatim, including ANSI)
**Findings (optional):** root cause if you've identified it
**Patch (optional):** PR link if you have one
```

The `tests/realworld/probe.sh` output dump is welcome attached.

## Triage cadence

Weekly review of every open `v0.4-beta-intake` issue. Output of triage
is one of:

- **Accepted** — severity confirmed, component owner assigned, target
  release set (`v0.3.x` patch / `v0.4-beta`).
- **Closed `out-of-scope`** — points at the boundary doc.
- **Closed `wontfix`** — explicit reason in the close message.
- **Needs-info** — questions back to the reporter; auto-close after 30 days
  of silence.

## Exit criteria

`v0.4-beta` ships when **all** of these hold:

1. Zero open issues with severity `critical` or `high`.
2. Severity `medium` issues either fixed or have an accepted target of
   `v0.5` documented in this file's "deferred to v0.5" appendix.
3. `cargo clippy --all-targets` clean.
4. All 800+ lib tests pass.
5. `tests/integration/run.sh` green on at least Fedora 43+, Debian 12+,
   Alpine 3.19+, Arch.
6. A findings doc lives at `docs/security/v0.4-beta-findings.md`
   enumerating every accepted issue, the fix, and the regression test.
7. The roadmap and README update to point at the v0.5 cycle.

## Hunt suggestions

The bypass-hardening pass already audited every `Command::new` site
and every recent parser. Higher-leverage hunt areas the pass *did
not* cover:

- **fuzz the CLI parser**: clap derive accepts a lot of input shapes.
  Hand-roll a few `cargo test --release` style fuzz cases against the
  config-key parser and the timer-interval parser.
- **state migration replay**: feed a v0 / v1 / v2 / v3 `state.json`
  through the migration ladder and assert the post-migration shape.
  The schema_version field exists for this; exercising it is fair
  game.
- **DBus argument validation**: the May 2026 audit covered method
  names. Argument-level validation (e.g. NM rejects a malformed
  byte-array in `cloned-mac-address`) is worth its own pass.
- **race conditions**: `state_lock::HELD` is now `Mutex<Option<File>>`,
  but the apply path's read-modify-write of `state.json` between lock
  acquisition and lock release is theoretically racy with another
  process that lock-fails and falls through to the read-only path.
- **unicode handling**: `parse_duration` had a multi-byte panic.
  Other parsers (`hostname::validate_label`, `mac::oui::parse_literal_prefix`)
  warrant the same treatment.
- **file descriptor leaks**: the `proteus events run` daemon holds
  several long-lived subscriptions; verify clean teardown under
  `--once-after-secs` runs and SIGTERM.

Each of these is worth a focused issue under `v0.4-beta-intake`.
