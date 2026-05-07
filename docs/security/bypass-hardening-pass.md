# Bypass hardening pass

Roadmap Milestone 6 deliverable. Independent of the May 2026 DBus
surface audit (see `docs/security/dbus-surface.md`), this pass walks
every place Proteus shells out and every parser added since the May
2026 audit, asks "could a hostile actor with control of one of the
inputs (PATH, env, sysfs file, kernel output) divert this to an
attacker-controlled binary or crash the process?", and applies the
fixes inline.

The pass produces both a findings record (this doc) and a code change.
Treat this doc as the single source of truth for which surfaces have
been audited and which fixes landed; review against this artifact
rather than against the source so the next reviewer doesn't waste time
re-walking sites that already have a documented control.

---

## Threat model

The Proteus binary runs in three contexts that matter for hardening:

1. **Interactive root shell** — the operator runs `sudo proteus apply`.
   `$PATH` inherits from the operator's shell. A user-writable directory
   ahead of `/usr/bin` in `$PATH` becomes a privilege-escalation surface
   any time we let `Command::new("foo")` walk `$PATH`.
2. **systemd unit** (`proteus-rotate.service`, `proteus-events.service`,
   the dispatcher hook). Unit files set a clean `PATH=/usr/sbin:/usr/bin:/sbin:/bin`,
   so the PATH-injection surface is small but non-zero — out-of-tree
   distro packagers may forget the explicit `Environment=PATH`.
3. **NM dispatcher script** (`dist/networkmanager/dispatcher.d/01-proteus`)
   already pins `PATH=/usr/sbin:/usr/bin:/sbin:/bin` before invoking
   `proteus`, so the shell side is covered. The proteus binary itself
   is the one we're hardening here.

Out of scope:
- Sandbox escapes. `RestrictAddressFamilies`, `ProtectSystem`,
  `AmbientCapabilities` etc. are configured in the systemd unit; the
  binary itself doesn't sandbox.
- DBus surface. Covered separately in `docs/security/dbus-surface.md`.

---

## Shellout audit

Catalog of every `Command::new(...)` site in `src/` (test code excluded).
Layered into three classes by binary-resolution control:

### Class A — already pinned to absolute path (predates this pass)

| Site | Binary | Pin pattern |
|---|---|---|
| `src/mac/factory.rs:106-118` | `ethtool` | `ETHTOOL_ABS_PATH = "/usr/sbin/ethtool"` then PATH fallback (issue #202) |
| `src/rf/mod.rs:127–583` (8 sites) | `iw` | `IW_ABS_PATHS = ["/usr/bin/iw", "/sbin/iw"]` then PATH fallback |

### Class B — newly hardened by this pass (`crate::process::resolve_bin`)

`crate::process::resolve_bin(name, abs_paths)` returns the first
existing absolute path or falls back to the bare name (so the
`Command::new` then walks `$PATH`). Adopted at every Class-B site so a
pristine production install picks the canonical path; Nix / Alpine /
custom-prefix installs still work via the PATH fallback.

| Site | Binary | New behavior |
|---|---|---|
| `src/nft/mod.rs` (3 sites) | `nft` | `process::nft()` → `/usr/sbin/nft` or `/sbin/nft` |
| `src/kill_switch/mod.rs:179` | `ip` | `process::ip()` → `/usr/sbin/ip` / `/sbin/ip` / `/usr/bin/ip` |
| `src/ipv6/mod.rs:209` | `sysctl` | `process::sysctl()` → `/usr/sbin/sysctl` / `/sbin/sysctl` |
| `src/commands/stack.rs:221` | `sysctl` | same |
| `src/ntp/mod.rs:172` | `systemctl is-active` | `process::systemctl()` |
| `src/dns/mod.rs:159` | `systemctl is-active` | same |
| `src/commands/{ntp,dns,resolved,session,timer,apply}.rs` (8 sites) | `systemctl` | same |
| `src/commands/timer.rs:256` | `journalctl` | `process::journalctl()` |
| `src/dns/mod.rs:193` | `ss` | `process::ss_bin()` |
| `src/rf/mod.rs:563` | `dmesg` | `process::dmesg()` |
| `src/commands/uninstall.rs:265` | varies (`run_quiet` dispatch) | resolves `systemctl` / `nft` / `sysctl` / `ip` / `semanage` per-program |
| `src/commands/revert.rs:208` | varies (`run_quiet` dispatch) | resolves `systemctl` / `nft` / `sysctl` / `ip` per-program |

### Class C — operator-controlled by design

| Site | Notes |
|---|---|
| `src/commands/config_cmd.rs:141` (`$EDITOR`) | The operator's editor binary. Honoring `$EDITOR` is the documented behavior; pinning would break the feature. The variable is read once and not written; no command injection is possible because `Command::new` does not invoke a shell. |
| `src/commands/persona.rs:360` (`$EDITOR`) | same |

### Class D — out of scope

None. Every `Command::new` site fits one of the three classes above.

---

## Parser audit

Parsers added since the May 2026 audit. Each is a candidate for
input-driven crash / overflow / unintended state if the input source is
attacker-controlled (kernel output, /sys/class/* file, user config).

| Parser | Input source | Risk surface | Status |
|---|---|---|---|
| `mac::oui::parse_literal_prefix` (`src/mac/oui.rs:257`) | persona `oui_pool` strings | Bounded input length; `as_bytes()` walk; no panics | OK |
| `per_ssid::parse_duration` (`src/per_ssid.rs:131`) | `[per_ssid."<ssid>"].rotate_interval` config string | Two bugs found by this audit: panic on non-ASCII trailing char, silent wrap on `n * 86_400` overflow | **FIXED** by this pass — see "Findings" below |
| `mac::factory::parse_ethtool_permanent` (`src/mac/factory.rs:147`) | `/usr/sbin/ethtool -P` stdout | Anchored on canonical "Permanent address: <mac>" header; canonical-MAC-shape guard (#206-E) | OK |
| `mac::arp::parse_arp_*` (`src/mac/arp.rs:50-143`) | `/proc/net/arp` (kernel) | Column-count guard; `.parse::<Mac>()` is total | OK |
| `rf::parse_iw_dev_info_txpower` (`src/rf/mod.rs:290`) | `iw dev <iface> info` stdout | Requires literal `txpower ` prefix (issue #160 hardening); `f32::parse` is total; NaN cast saturates | OK |
| `rf::parse_iw_reg_get_max_mbm` (`src/rf/mod.rs:327`) | `iw reg get` stdout | Requires power-tuple anchor (issue #160 hardening); `i32::parse` is total | OK |
| `rf::parse_iw_dev_info_phy_and_type` (`src/rf/mod.rs:471`) | `iw dev <iface> info` stdout | Pure `split_whitespace` walk; no parses to fail | OK |
| `rf::parse_iw_phy_capabilities` (`src/rf/mod.rs:503`) | `iw phy` stdout | Token-presence checks only; no integer parses | OK |
| `logging::parse_rust_log` (`src/logging.rs:89`) | `RUST_LOG` env var | Operator-controlled; comma-split + level-name match; no panics | OK |
| `persona::load::parse_file` (`src/persona/load.rs:139`) | `/etc/proteus/personas/<id>.toml` (operator-authored) | Schema-checked via `Persona` derive + post-parse `schema_check`; bad files surface a wiki-linked error | OK |
| `state::State::load` (`src/state.rs:247`) | `/var/lib/proteus/state.json` | Bad JSON triggers `quarantine_path` rename + degrade-to-empty (issue #119 pattern) | OK |

---

## Findings

### Finding #BH-1 (medium): `per_ssid::parse_duration` panics on non-ASCII trailing char

**File**: `src/per_ssid.rs:131`

**Symptom**: `parse_duration("30é")` triggered
`split_at(s.len() - 1)` not on a UTF-8 char boundary, hitting an
unwrap-style panic in `str::split_at`. Reachable from
`[per_ssid."<ssid>"].rotate_interval` in operator-authored
config — a hand-edited config with a pasted Unicode
suffix could crash the resolver.

**Severity**: medium. The config is operator-controlled and the
crash isn't an exploit, but it's a quality-of-implementation hit
that violates the "fall through gracefully" contract documented
at the parser's docstring.

**Fix**: walk by `char` instead of byte. Concretely, take the last
`char` via `chars().next_back()`, advance the unit-start cursor by
that char's `len_utf8()`, and split at the resulting byte index.

**Regression test**: `parse_duration_handles_multi_byte_trailing_character`
covers `30é`, `é`, and `30秒`.

---

### Finding #BH-2 (low): `per_ssid::parse_duration` overflow wrap

**File**: `src/per_ssid.rs:131`

**Symptom**: `parse_duration` multiplied the parsed `u64` by
`86_400` (days), `3600` (hours), or `60` (minutes) without
overflow checks. Debug builds would panic on
`n.checked_mul(86_400).unwrap()` semantics; release builds would
silently wrap, producing a duration that doesn't reflect the
operator's intent.

**Severity**: low. Wrapping doesn't cause memory corruption; the
worst-case is a config-driven cooldown that's much shorter (or
longer) than the operator wrote. Not exploitable.

**Fix**: replace `n * 86_400` etc. with `n.checked_mul(86_400)?`
so an overflowing duration cleanly returns `None`. Resolver
falls back to the global timer, which is the documented contract
when the per-SSID `rotate_interval` is malformed.

**Regression test**: `parse_duration_returns_none_on_overflow`
covers `u64::MAX` for each unit and pins that `u64::MAX` *seconds*
parses successfully (in-range).

---

### Finding #BH-3 (low): broad `$PATH` reliance for privileged binaries

**Sites**: every Class-B entry in the table above (15 sites across
12 modules).

**Symptom**: Proteus relied on `Command::new("nft")` etc. to walk
`$PATH`. In an interactive root shell with a user-writable directory
ahead of `/usr/bin` in `$PATH`, an attacker who controlled that
directory could ship a `nft`-named lookalike that reads the firewall
ruleset Proteus is about to install.

**Severity**: low — every documented production path (NM dispatcher,
systemd unit) already pins `$PATH`. The risk is interactive `sudo
proteus apply` from a tampered shell, which is a smaller surface but
still worth closing.

**Fix**: introduce `crate::process::resolve_bin(name, abs_paths)`
plus convenience accessors (`process::nft()`, `process::ip()`,
`process::systemctl()`, etc.). Adopted at every Class-B site. PATH
fallback remains so Nix and Alpine layouts that ship `nft` outside
`/usr/sbin` still work.

**Regression test**: `process::tests::resolve_bin_returns_bare_name_when_nothing_exists`
plus `paths_tables_are_nonempty_and_absolute` pins the lookup contract
and the table well-formedness.

---

## Out-of-scope items / next steps

- **Suid hardening**: Proteus is not a suid binary. If a future
  packaging path adds a suid wrapper, every Class-B site needs to
  refuse the PATH fallback entirely.
- **Capability assertions**: `proteus-events.service` already pins
  `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW`. The runtime
  doesn't currently assert what it has — a hostile mirror could spoof
  by stripping caps. Tracked separately in roadmap M6 "security review".
- **Re-audit cadence**: any new parser added to `src/` should append
  a row to the parser-audit table above. Any new `Command::new` site
  that doesn't go through `crate::process::resolve_bin` should justify
  itself in a code comment.
