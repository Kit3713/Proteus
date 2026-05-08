# Issues Log — Bug Hunt Session 2026-05-08

This file tracks issues discovered during a focused bug-hunt session against
the `claude/bug-hunt-session-buqhU` branch (tip of `main` at the time:
`1cc5cc5`). Severity follows the convention used in `CHANGELOG.md`:
**critical / high / medium / low / info**.

Each entry has:

- **Severity**
- **Area** (subsystem)
- **File:line** (or file paths if cross-cutting)
- **Description** (what's wrong)
- **Impact** (what can go wrong at runtime / who's affected)
- **Suggested fix** (one-line direction)

The findings below are reported observations, not patches. None of the bugs
have been fixed in this commit; that's left to whoever picks up the queue.

## Highlights — Top 10 to fix first

Ranked by impact × likelihood. Each links to the section with full detail.

1. **N1** — `proteus events run` does NOT actually rotate; handler only logs. Major doc/code mismatch with security implications.
2. **CL2 / M1** — `--yes` flag silently dropped on `bluetooth/hostname/dns/resolved/ntp/portal/dhcp` mutators. Mutators run without confirmation.
3. **D1** — every example config uses `[discovery]` field names (`mdns_responder`, `llmnr`, `ntp_normalize`, ...) that don't exist in the schema. New users get nothing they think they configured.
4. **D2** — `[rotation]` section in every example does not exist in the config schema. Documented as "wins over `mac.rotation_interval`" — it doesn't, because it's silently ignored.
5. **D4** — `[mac]` examples use `exclude_gateways`, `exclude_arp_table`, `per_connection` — none of which exist in `RawMacConfig`.
6. **C1** — state-lock `HELD` Mutex is held across the retry-sleep loop; concurrent acquirers serialize on the in-process Mutex, starving tasks for up to 5 s.
7. **CL1** — `proteus status --watch --interval 0s` is accepted; busy-loops a CPU.
8. **V1 / V2** — per-SSID `rotate_interval = "0s"` accepted (continuous rotation); unknown profile names silently fall through.
9. **B1-B5** — `proteus-events.service` unit is missing from install.sh, RPM `%post`, Debian rules, Gentoo ebuild, and Alpine APKBUILD. The events daemon is unreachable on every install path.
10. **E6** — NM `GetSecrets` PermissionDenied collapses to "no secrets"; the subsequent Update can wipe a user-owned connection's PSK.

## Numbers

- 6 background hunting agents + 1 manual pass.
- 11 sections, ~85 distinct issues recorded (all verified against the actual
  source where the agents made specific code claims).
- 1 false positive caught and excluded (build agent's "edition 2024 not on
  stable" claim — Rust 1.85+ supports it; toolchain is pinned at 1.93.0).
- File of record: `docs/ISSUES.md` (this file).

---

## Section 1 — Security audit (src/)

### S1. unsafe `set_var`/`remove_var` in tests without serialization
- **Severity**: high (test-only, but causes flaky CI under `cargo test --jobs N`)
- **Area**: test suite / process environment
- **File**: `src/commands/uninstall.rs:309-326`
- **Description**: `unsafe { std::env::set_var(...) }` and `remove_var` mutate process env without a serialization mutex. Other tests reading the same env vars in parallel observe inconsistent state.
- **Impact**: Intermittent test failures; failures attributed to wrong cause; flaky CI for the uninstall test suite.
- **Fix**: Wrap with a static `Mutex<()>` guard (mirror `state_lock::TEST_SERIAL`).

### S2. `Location` header from captive-portal probe surfaced unvalidated
- **Severity**: medium
- **Area**: captive_portal / HTTP
- **File**: `src/captive_portal/mod.rs:168-175`
- **Description**: The HTTP `Location` header is returned verbatim as `DetectionOutcome::redirect_target` and printed to stdout/JSON. While `is_request_safe()` defends the *outbound* request, the response header is exposed without re-validation.
- **Impact**: A hostile portal can plant control characters (CR/LF) in `redirect_target`, breaking downstream JSON consumers or terminals. No RCE, but log poisoning / display corruption.
- **Fix**: Run `Location` through `parse_http_url()` (or at least strip `\r\n\t\0` and reject non-ASCII) before storing.

### S3. State lock file lacks explicit `mode(0o600)`
- **Severity**: low
- **Area**: state_lock / file permissions
- **File**: `src/state_lock.rs:168-174`
- **Description**: `OpenOptions::new().read(true).write(true).create(true)` opens `/var/lib/proteus/.lock` without `.mode(0o600)`. Inconsistent with `write_atomic()` in `src/commands/mod.rs:220` which sets explicit modes.
- **Impact**: World-readable lock file (dependent on umask); not secret data, but inconsistent hygiene flagged by hardening review.
- **Fix**: Add `.mode(0o600)` via `OpenOptionsExt` on Unix.

### S4. State quarantine rename failure swallowed
- **Severity**: medium
- **Area**: state.rs / error handling
- **File**: `src/state.rs:261-269`
- **Description**: When `state.json` parse fails, `fs::rename(path, &quarantine)` is invoked and the result discarded with `let _ = ...`. If the rename itself fails (permissions, ENOSPC, race), the next `apply` overwrites the corrupt-but-preserved evidence with a fresh save.
- **Impact**: Loses forensic evidence of the corruption; user can't tell `proteus doctor` why state was reset.
- **Fix**: Log the rename error at `warn!` level and continue; or `bail!` if the parent dir is unwritable.

### S5. DNS resolved drop-in cleanup TOCTOU window
- **Severity**: low
- **Area**: dns / file iteration
- **File**: `src/dns/mod.rs:340-350`
- **Description**: `read_dir() → entry.path() → is_symlink()` introduces a TOCTOU window: if a regular file is replaced with a symlink between the dirent and the metadata stat, the symlink survives the filter.
- **Impact**: Privileged write to attacker-controlled target via symlink. Limited because the cleanup loop only deletes; still worth tightening.
- **Fix**: Use `fs::symlink_metadata` immediately and consider `O_NOFOLLOW` for the open.

### S6. Captive-portal HTTP request-line builder defends host+path independently
- **Severity**: medium
- **Area**: captive_portal / HTTP protocol
- **File**: `src/captive_portal/mod.rs:268-288`
- **Description**: `is_request_safe()` checks `host` and `path` separately for CR/LF/NUL/C0. The `format!()` that assembles the full request line is not re-validated, so a future change that splits on `?` or appends a query string risks reintroducing injection if the new field isn't checked.
- **Impact**: Future-proofing only today; today's code is safe.
- **Fix**: Add a single post-format `assert!(!req.contains("\r\n\r\n…"))`-style guard plus a unit test.

### S7. Polkit actions not validated at runtime
- **Severity**: low
- **Area**: polkit
- **File**: `dist/polkit/com.kit3713.proteus.policy` (no runtime checker)
- **Description**: `lib.rs` has *compile-time* tests that the policy file is correct. There's no runtime check that the policy is installed (e.g., `proteus doctor` doesn't probe `pkaction --action-id com.kit3713.proteus.rotate`).
- **Impact**: A user with a partial install (binary present, polkit policy not deployed by the package) sees opaque "permission denied" instead of an actionable doctor warning.
- **Fix**: Add a `doctor` check that calls `pkaction --action-id …` for each expected id and warns if missing.

### S8. NM `GetSettings`/`GetSecrets` values not sanitized for log paths
- **Severity**: medium
- **Area**: nm
- **File**: `src/nm/mod.rs:150-160` (and any tracing site that interpolates a setting)
- **Description**: Strings returned from NM over DBus are deserialized via zbus and trusted by structure but not by content. If they're ever interpolated into a `tracing::error!` line, an attacker who controls a connection profile name or SSID can plant `\r\n` and inject log lines.
- **Impact**: Log injection / fake-event spoofing in journald.
- **Fix**: Either escape via `Debug` formatter (`{:?}`) on every log site that touches NM strings, or wrap them in a `Sanitized<&str>` newtype.

### S9. `unsafe { OwnedFd::from_raw_fd(...) }` in netlink source under-documented
- **Severity**: low
- **Area**: events / regulatory-domain source
- **File**: `src/events/source/reg_domain.rs:81-85`
- **Description**: The fd is checked `>= 0` before wrap, but the safety comment is sparse. Future refactors may reorder the check.
- **Fix**: Replace with a `// SAFETY:` block stating the precondition and the source of the invariant.

### S10. Lenient MAC separator parsing (cosmetic / docs)
- **Severity**: info
- **Area**: mac / persona parsing
- **File**: `src/mac/generator.rs:30-33`
- **Description**: `01:23-45 67-89:ab` is accepted by the MAC parser. Mixing separators is unusual and likely not what users intended.
- **Fix**: Document leniency in `wiki/personas.md` or add a strict-mode toggle.

---

## Section 2 — Panic potential (production paths)

### P1. `split_at` on potentially empty duration string
- **Severity**: high
- **Area**: per_ssid / duration parsing
- **File**: `src/per_ssid.rs:136`
- **Description**: `let (num, unit) = s.split_at(s.len() - 1);` — if `s.is_empty()`, `s.len() - 1` underflows in release (silent wrap to `usize::MAX`) or panics in debug. Reachable via a per-SSID block with `rotate_interval = ""` in TOML or from a future caller that hasn't been audited.
- **Impact**: Panic / OOB slice on hot per-SSID resolution path, called in the rotate flow and the events daemon.
- **Fix**: `if s.is_empty() { return None; }` before `split_at`.

### P2. Empty-label bounds panic in hostname validator
- **Severity**: high
- **Area**: hostname
- **File**: `src/hostname/mod.rs:108`
- **Description**: `if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-'` — a hostname like `"a..b"` produces an empty label after splitting on `.`, and `bytes[0]` panics.
- **Impact**: `proteus hostname` apply path / persona-rendered hostnames crash the user command.
- **Fix**: `if label.is_empty() { return Err(...); }` before indexing `bytes[0]`.

### P3. `.file_name().unwrap()` on paths in diff
- **Severity**: medium
- **Area**: diff
- **File**: `src/diff/mod.rs:464`
- **Description**: `.map(|p| p.file_name().unwrap()...)` panics on paths ending in `/` or `..`. The diff command runs as part of `proteus apply` orchestration.
- **Fix**: `and_then(|p| p.file_name())` and skip entries that have none.

### P4. `.file_name().unwrap()` in commands/mod.rs
- **Severity**: medium
- **Area**: commands (apply orchestration)
- **File**: `src/commands/mod.rs:360-361`
- **Description**: Same pattern as P3. Two adjacent unwraps for `a` and `b`. Triggered by managed-file SHA verification path.
- **Fix**: Same as P3.

### P5. Off-by-one in HTTP body slice past CRLF separator
- **Severity**: low
- **Area**: captive_portal
- **File**: `src/captive_portal/mod.rs:364`
- **Description**: `&body[4.min(body.len())..]` hardcodes 4 (length of `\r\n\r\n`) without verifying that the splitter actually found that token at the offset. A corrupted or non-standard server response could mis-align.
- **Fix**: Take the offset returned by `find_double_crlf` directly and add `+ 4` only if the find succeeded.

### P6. Probe `as u8` truncation
- **Severity**: low (today; would silently underreport quorum if expanded)
- **Area**: probe
- **File**: `src/probe/mod.rs:182-183`
- **Description**: `results.len() as u8` and `.count() as u8` truncate above 255. Today the design caps generation/probe attempts well below 255, but the cast is silent — a future bump above the cap would silently produce wrong reporting.
- **Fix**: Use `u32` and validate the cap or use `try_into()` with explicit clamp.

### P7. `.unwrap()` on `get_mut("mac")` in `config_cmd` test (test-only)
- **Severity**: low (test-only)
- **Area**: config_cmd tests
- **File**: `src/commands/config_cmd.rs:693`
- **Description**: Test asserts the default config has a `[mac]` table; if the schema is reorganized this panics with no actionable message.
- **Fix**: `.expect("default config must have [mac] table")`.

---

## Section 3 — Concurrency / state-management

### C1. State-lock `HELD` Mutex held across `thread::sleep` retry
- **Severity**: high
- **Area**: state_lock
- **File**: `src/state_lock.rs:141-156` (acquire path)
- **Description**: The `HELD: Mutex<Option<File>>` guard is held for the entire 50× × 100ms retry budget. Concurrent callers serialize on the in-process Mutex even when the on-disk flock is the actual contention point, blocking other tokio tasks for up to 5 s.
- **Impact**: In the events daemon (multiple sources fire simultaneously), this serializes acquisitions and starves tasks. In the worst case a user-initiated `proteus apply` waits behind a stuck retry loop.
- **Fix**: Release the in-process Mutex before each sleep, re-acquire at the top of the loop. Or move retry to a tokio-friendly `tokio::time::sleep` and use `tokio::sync::Mutex`.

### C2. Cooldown bypass via wall-clock manipulation
- **Severity**: medium
- **Area**: backend / cooldown
- **File**: `src/backend/nm.rs:335-347`
- **Description**: `remaining_cooldown()` uses `SystemTime::now()`. An operator (or NTP-skew attack) that moves the clock backwards bypasses the cooldown.
- **Impact**: Burst rotations that would normally be rate-limited; not a security boundary but a UX/policy bypass.
- **Fix**: Persist a monotonic `Instant`-anchored offset alongside the stored timestamp and use the larger of the two.

### C3. Subprocess `.output()` calls have no timeout
- **Severity**: medium
- **Area**: apply / revert
- **File**: `src/commands/revert.rs:207-216`, `src/commands/apply.rs:454`
- **Description**: `Command::new(...).output()` blocks forever if the child hangs. `systemctl daemon-reload`, `nft`, `sysctl` are the typical victims when systemd/kernel are wedged.
- **Impact**: `proteus apply` / timer-fired apply hangs forever, lock held, no other instance can run.
- **Fix**: Wrap in a 30 s wall-clock timeout (helper that spawns + kills on expiry).

### C4. Events daemon: no SIGTERM handler
- **Severity**: medium
- **Area**: events / daemon
- **File**: `src/commands/events.rs:207-233`
- **Description**: The 250 ms poll loop only calls `t.stop.stop()` on the `--once-after-secs` deadline. A `systemctl stop proteus-events.service` SIGTERM aborts the runtime mid-flight, leaving DBus subscriptions and netlink sockets dangling for the kernel/dbus-daemon to clean up.
- **Impact**: Resource churn on rapid restart; potential to miss the next reconnect window.
- **Fix**: Spawn a `tokio::signal::unix::signal(SignalKind::terminate())` listener; on first signal, break out of the loop and call `stop()` on every task before exiting.

### C5. State quarantine rename is best-effort
- **Severity**: low
- **Area**: state.rs
- **File**: `src/state.rs:269`
- **Description**: `let _ = fs::rename(path, &quarantine);` silently swallows errors — see also S4 above. If quarantine fails (read-only fs, permission), the next load re-reads the same corrupt file and re-quarantines forever (or stays stuck).
- **Fix**: Return `Err` if rename fails; or fall back to `fs::remove_file` after logging.

### C6. Mock backend does not flock — masks test races
- **Severity**: medium (test-quality)
- **Area**: tests / backend
- **File**: `src/backend/mock.rs`
- **Description**: Mock impl runs without acquiring `state_lock`, so concurrent unit tests in `commands::rotate::tests` writing to the same TempDir don't observe lock-contention bugs. Real bugs in the locking layer can hide.
- **Fix**: Make `MockBackend` optionally take the same flock under a `cfg(test)` flag, or wrap mock-using tests with `state_lock::TEST_SERIAL`.

### C7. Event registry swallows handler panics via Mutex poison recovery
- **Severity**: low
- **Area**: events
- **File**: `src/events/mod.rs:150-177`
- **Description**: `unwrap_or_else(|e| e.into_inner())` recovers a poisoned mutex without surfacing the panic. The next handler runs as if nothing happened, but the panic is lost from logs.
- **Fix**: Wrap each handler call in `std::panic::catch_unwind` and explicitly log via `tracing::error!` before continuing.

### C8. `PROTEUS_LOCK_TIMEOUT_MS` not bounded
- **Severity**: low
- **Area**: state_lock
- **File**: `src/state_lock.rs:67-74`
- **Description**: Env var is parsed but not clamped. Setting `3600000` blocks `proteus apply` for an hour with no upper sanity limit.
- **Fix**: Clamp to `[100, 120_000]` ms with a `warn!` log if the user requested more.

### C9. UUID-keyed state reads from cross-system backups
- **Severity**: low (intentionally fail-safe)
- **Area**: state migration
- **File**: `src/state.rs:336-360`
- **Description**: When state.json is restored from a different host, UUIDs no longer match. Current behavior: silently drop unmatched entries and recapture. Safe — but undocumented.
- **Fix**: Add a one-line note in the schema-version migration comment so future maintainers don't think it's a bug.

---

## Section 4 — Config validation / per-SSID resolver

### V1. Zero / empty rotation interval accepted
- **Severity**: high
- **Area**: per_ssid / duration parsing
- **File**: `src/per_ssid.rs:131-145`
- **Description**: `parse_duration("0s")` returns `Some(Duration::ZERO)`. A zero-interval rotate triggers continuous rotation. Combined with P1, an empty string panics.
- **Fix**: Reject zero before dispatching; reject empty up front.

### V2. Unknown profile name in per-SSID silently falls through
- **Severity**: high
- **Area**: per_ssid / profile
- **File**: `src/per_ssid.rs:77`
- **Description**: `and_then(Profile::parse)` returns `None` on a typo (e.g. `aggressiveness_profile = "aggresive"`). Resolver picks the global default; user thinks they enabled an SSID-specific aggressiveness level.
- **Impact**: User-facing footgun — silently weakens or strengthens posture per their typo.
- **Fix**: Validate at config-load time. Either reject unknown names with a TOML parse error, or emit a `warn!` listing the SSIDs with unknown profiles. `proteus ssid show <ssid>` should also flag invalid policy values.

### V3. `quorum_n > quorum_total` not validated
- **Severity**: high
- **Area**: probes / config
- **File**: `src/config.rs:728-734`, `:392-396`
- **Description**: The two `Option<u8>` are resolved independently. A user can configure `quorum_n=5, quorum_total=4` — impossible for the probe path to ever return success.
- **Fix**: After resolution, `if cfg.probes.quorum_n > cfg.probes.quorum_total { bail!("…"); }` or clamp.

### V4. Unbounded second-precision durations
- **Severity**: medium
- **Area**: config (events / probe)
- **File**: `src/config.rs:785, 827-828`
- **Description**: `timeout_secs`, `portal_poll_secs`, `link_flap_window_secs` are raw `u64`. `0` produces busy-loops or instant-collapse windows; `u64::MAX` produces 584-year timeouts.
- **Fix**: Clamp at parse time — `timeout_secs ∈ [1, 3600]`, `portal_poll_secs ∈ [5, 300]`, `link_flap_window_secs ∈ [1, 60]` (or whatever the design intends). Document chosen bounds.

### V5. `tx_power_reduction_db` unbounded `u8`
- **Severity**: medium
- **Area**: rf / config
- **File**: `src/config.rs:792, 489`
- **Description**: Raw `u8` accepts 0..=255 dB. Hardware will clamp silently; user gets no feedback that 200 dB was meaningless.
- **Fix**: Clamp to a sane range (0..=30 dB) at load with a `warn!`.

### V6. Persona ID in per-SSID not validated against known personas
- **Severity**: medium
- **Area**: persona / per_ssid
- **File**: `src/per_ssid.rs:76, 91`
- **Description**: `persona = "nonexistent"` is accepted; resolver passes through; orchestrator silently falls back. User believes they applied `iphone-15` but actually applied nothing.
- **Fix**: Cross-validate at load time against the persona registry (built-ins + `/etc/proteus/personas/`). Reject or warn.

### V7. `pin_mac` format not validated at load
- **Severity**: medium
- **Area**: config / mac
- **File**: `src/config.rs:1072`
- **Description**: A typo like `pin_mac = "gg:hh:ii:jj:kk:ll"` survives load and fails at apply time with a non-obvious error.
- **Fix**: Validate via `mac::oui::parse_literal_prefix()` (or full-MAC equivalent) at load.

### V8. `parse_duration` overflow silently fails
- **Severity**: low
- **Area**: per_ssid
- **File**: `src/per_ssid.rs:137`
- **Description**: `num.parse::<u64>().ok()?` returns `None` on overflow. `99999999999999999d` falls back to global timer with no diagnostic.
- **Fix**: Distinguish "no value" from "out of range" — log a warn at the latter.

### V9. Source-trace semantics in 4-layer resolver are subtle
- **Severity**: low (cosmetic / docs)
- **Area**: per_ssid
- **File**: `src/per_ssid.rs:86-93`
- **Description**: `persona_contributed = per_persona.is_none() && config.persona.active.is_some()` is correct but easy to misread as "persona never affected this SSID" when actually per-SSID layered over.
- **Fix**: Rename to `global_persona_contributed`, add a comment.

### V10. Round-trip test only covers a few fields
- **Severity**: low (test gap)
- **Area**: config tests
- **File**: `src/config.rs:1441-1450`
- **Description**: `raw_config_round_trips_through_toml` checks `profile`, `mac.enabled`, `dhcp.enabled`. Misses arrays (`oui_pool`, `ntp_servers`), numeric fields (`quorum_n`), enum fields (`hostname.mode`).
- **Fix**: Extend the test to assert at least one field per type category.

### V11. Persona OUI pool not schema-validated at load
- **Severity**: low
- **Area**: persona / load
- **File**: `src/persona/load.rs:153-181`
- **Description**: `oui_pool = ["invalid-vendor", "gg:hh:ii"]` is accepted at load; warnings only fire at runtime when `mac::oui` tries to parse each entry.
- **Fix**: Move the `vendor name OR aa:bb:cc literal` validation into `schema_check()`.

### V12. SSID keys with TOML-special characters not exercised in tests
- **Severity**: low (test gap, possible bug)
- **Area**: config tests
- **File**: `src/config.rs:1637-1668`
- **Description**: `[per_ssid."Coffee Shop"]` works, but no test covers SSIDs containing `]`, `"`, `\`, leading/trailing whitespace, or non-UTF-8 sequences. SSIDs really can be these things on real APs.
- **Fix**: Add round-trip tests for at least `"]"`, `"\""`, `"\\"`, `"  leading-space"`.

---

## Section 5 — Build / packaging / CI

### B1. `proteus-events.service` missing from install.sh
- **Severity**: high
- **Area**: install.sh / systemd
- **File**: `install.sh:135-141` (unit-install loop) vs `dist/systemd/proteus-events.service`
- **Description**: The events service unit exists but the install loop installs only timers + companion services. Users who follow the install.sh path can't `systemctl enable proteus-events.service` because the unit isn't present in `/etc/systemd/system`.
- **Fix**: Add `proteus-events.service` to the install loop; rewrite `ExecStart=` to match the chosen `BIN_PATH` (mirroring the polkit rewrite at lines 160-182).

### B2. `proteus-events.service` missing from RPM `%post`/`%preun`
- **Severity**: high
- **Area**: rpm packaging
- **File**: `dist/rpm/proteus.spec:99-105`
- **Description**: Spec installs the unit file (line 68-70) but doesn't reference it in `%systemd_post` / `%systemd_preun` / `%systemd_postun_with_restart`. Result: ships disabled, never starts post-install.
- **Fix**: Add `proteus-events.service` to each `%systemd_*` macro invocation.

### B3. `proteus-events.service` missing from Debian rules
- **Severity**: high
- **Area**: debian packaging
- **File**: `dist/debian/rules`
- **Description**: `dh_auto_install` override explicitly enumerates units; `proteus-events.service` not listed. Debian package omits the daemon entirely.
- **Fix**: Add an explicit install line for the unit.

### B4. `proteus-events.service` missing from Gentoo ebuild
- **Severity**: high
- **Area**: gentoo packaging
- **File**: `dist/gentoo/proteus-0.1.0.ebuild:92-99`
- **Description**: `systemd_dounit` invocations enumerate units; events unit absent.
- **Fix**: Add `systemd_dounit dist/systemd/proteus-events.service`.

### B5. `proteus-events.service` missing from Alpine APKBUILD
- **Severity**: high
- **Area**: alpine packaging
- **File**: `dist/alpine/APKBUILD`
- **Description**: APKBUILD doesn't `install -Dm644` the events unit (Alpine commonly mirrors systemd units to OpenRC; either way the unit is unreferenced).
- **Fix**: Add the install line, or ship a runit/openrc equivalent for Alpine targets.

### B6. NM dispatcher uses `#!/bin/bash` despite POSIX policy
- **Severity**: medium
- **Area**: dist / NetworkManager
- **File**: `dist/networkmanager/dispatcher.d/01-proteus:1`
- **Description**: All other repo shell scripts are POSIX (`#!/bin/sh`) and run under dash. Only the dispatcher uses bash. Inconsistent and fails on minimal containers without `/bin/bash`.
- **Fix**: Convert to `#!/bin/sh`; verify no bashisms (the body looks POSIX-compatible already).

### B7. `ci.yml` missing top-level `permissions:` block
- **Severity**: medium
- **Area**: github actions
- **File**: `.github/workflows/ci.yml`
- **Description**: Default `GITHUB_TOKEN` in CI has every read+write scope. A compromised action step (e.g., a malicious dep) can write to the repo. `release.yml` correctly scopes; `ci.yml` doesn't.
- **Fix**: Add `permissions: { contents: read }` at the top.

### B8. `--locked` missing from Alpine, Void, Gentoo cargo invocations
- **Severity**: medium
- **Area**: distro packaging / reproducibility
- **File**: `dist/alpine/APKBUILD:57`, `dist/void/template:42`, `dist/gentoo/proteus-0.1.0.ebuild:78`
- **Description**: All three use `--frozen` (lock-file required) but not `--locked` (lock-file unchanged). A subtle resolver-state drift can produce different lockfiles between distro builds.
- **Fix**: Append `--locked` to each cargo command.

### B9. `softprops/action-gh-release@v2` not pinned to commit SHA
- **Severity**: low
- **Area**: github actions
- **File**: `.github/workflows/release.yml:477`
- **Description**: Major-version pin only. If the action's tag is moved (compromise or maintainer error), the next release uses tampered code with no warning.
- **Fix**: Replace with full commit SHA pin (40-char), document the version next to it.

### B10. install.sh + units use `/usr/local/bin/proteus`; distro packages use `/usr/bin/proteus`
- **Severity**: medium
- **Area**: install / packaging consistency
- **File**: `install.sh:19`, `dist/systemd/proteus-events.service:15` (and others), distro-side `%files`
- **Description**: Units in `dist/systemd/*.service` hard-code `/usr/bin/proteus`, but install.sh defaults to `/usr/local/bin/proteus` and only rewrites the polkit policy. Result: install.sh-installed services point to a path the binary isn't at.
- **Fix**: Either (a) extend the polkit-rewrite logic in install.sh to also `sed` `ExecStart=` paths in unit files, or (b) standardize on `/usr/bin/proteus` in units and have install.sh symlink `/usr/bin/proteus → /usr/local/bin/proteus`.

### B11. `uninstall.sh` `rm -rf "$STATE_DIR"` lacks `:?`-guard
- **Severity**: low
- **Area**: uninstall script
- **File**: `uninstall.sh:141, 145`
- **Description**: `rm -rf "$CONFIG_DIR"` and `rm -rf "$STATE_DIR"` proceed even if the variables somehow ended up empty. With `set -u` this would fail, but if `set -u` is removed in a future refactor the consequences are catastrophic.
- **Fix**: Use `rm -rf -- "${CONFIG_DIR:?must be set}"` and equivalent.

### B12. `build.rs` `panic!()` on missing env vars
- **Severity**: low
- **Area**: build.rs
- **File**: `build.rs:21, 23, 71, 127`
- **Description**: Cryptic panic message on `CARGO_MANIFEST_DIR` / `OUT_DIR` absence; users get a stack-trace, not "build invoked outside cargo".
- **Fix**: `env::var(...).map_err(|_| "build.rs requires CARGO_MANIFEST_DIR; run via cargo")?`.

### B13. `scripts/check.sh` validates install.sh with `bash -n` instead of `sh -n`
- **Severity**: low
- **Area**: scripts
- **File**: `scripts/check.sh:156`
- **Description**: `bash -n install.sh` accepts bashisms that `dash` would reject. install.sh is supposed to be POSIX; the syntax check should match.
- **Fix**: `sh -n install.sh`.

### B14. Cargo.toml has no `[features]` table; ebuild references features
- **Severity**: low (drift)
- **Area**: cargo / packaging
- **File**: `Cargo.toml:25-46`, `dist/gentoo/proteus-0.1.0.ebuild:64`
- **Description**: Gentoo ebuild includes feature-flag plumbing in comments, but the crate has no defined `[features]`. The ebuild's flag handling is a no-op.
- **Fix**: Either define real features (e.g. `events`, `bluetooth`) or drop the ebuild's references.

### B15. Polkit policy has no user restriction
- **Severity**: medium (defense-in-depth)
- **Area**: polkit
- **File**: `dist/polkit/com.kit3713.proteus.policy:9-26`
- **Description**: `<allow_active>auth_admin</allow_active>` without a `user="!root"` clause means root invoking `pkexec proteus …` re-prompts unnecessarily and is permitted. Not a vuln (root can do anything anyway), but harms the UX and is inconsistent with the principle of "polkit only matters for non-root".
- **Fix**: Document expected user context; consider explicit conditional logic.

---

## Section 6 — Error handling / logging

### E1. Info-level events on the success path of `apply`
- **Severity**: medium
- **Area**: apply / logging
- **File**: `src/commands/apply.rs:125`
- **Description**: `tracing::info!(driver = %driver, …, "apply: backend preflight ok")` fires unconditionally on success. The README/wiki contract is that read-only / success paths are silent at default verbosity.
- **Fix**: Demote to `debug!` (or remove).

### E2. Info-level events in events daemon hot path
- **Severity**: medium
- **Area**: events daemon / logging
- **File**: `src/commands/events.rs:100, 113, 195, 217`
- **Description**: The events daemon emits `info!` on connection-up / regular rotation. With JOURNAL_STREAM set, this clutters the journal at default level.
- **Fix**: Demote routine notifications to `debug!`; reserve `info!` for one-shot startup/shutdown.

### E3. RUST_LOG parse failures silently fall back
- **Severity**: low
- **Area**: logging
- **File**: `src/logging.rs:112`
- **Description**: `Targets::from_str(&directives.join(",")).unwrap_or_else(|_| Targets::new())` produces an empty filter when the user mistypes `RUST_LOG=proteus=invalid`. No diagnostic on stderr.
- **Fix**: Print a `warn!` to stderr (or before init, `eprintln!`) when the parse fails.

### E4. `show-config` permission error logs at `warn!` (invisible at default)
- **Severity**: medium
- **Area**: show_config
- **File**: `src/commands/show_config.rs:38`
- **Description**: When the config file is unreadable, the error path uses `tracing::warn!()`. At default verbosity that's filtered out — the user gets exit code 66 with no visible message.
- **Fix**: Use `eprintln!("proteus: cannot read {path}: permission denied")` and return PERMISSION_ERROR.

### E5. `Ok(exit::GENERIC_ERROR)` pattern mixes error reporting with exit
- **Severity**: medium
- **Area**: dispatch / commands
- **File**: `src/commands/dhcp.rs:71-73, 112-113, 142-143` (and similar in other `*_cmd.rs`)
- **Description**: Several commands print the error then return `Ok(exit::GENERIC_ERROR)`. Callers can't distinguish a permission failure (66) from a config error (65); top-level dispatch never sees the typed error.
- **Fix**: Either propagate `Err(anyhow::Error)` and let dispatch map to a code, or define a `ProteusError` enum that preserves the cause.

### E6. NM `GetSecrets` errors swallowed with `.ok()` even on permission denial
- **Severity**: medium
- **Area**: nm
- **File**: `src/nm/mod.rs:268`
- **Description**: A failed `GetSecrets` returns `None`, treated as "no secrets present". For user-owned wifi connections, secrets are intentionally hidden from a root agent; this collapses to the same return as "actually-empty secrets section" and the subsequent Update silently strips the PSK.
- **Impact**: Connection's PSK can be wiped after rotate, breaking reconnect for user-owned profiles.
- **Fix**: Match on the underlying DBus error: distinguish `NoSecrets`/`InvalidProperty` (treat as empty) from `PermissionDenied`/`Failed` (skip the Update or emit a clear error).

### E7. `fs::read_to_string` results swallowed with `unwrap_or_default()` after context
- **Severity**: medium
- **Area**: config_cmd
- **File**: `src/commands/config_cmd.rs:149, 167-170, 372-375, 388-391`
- **Description**: Reads use `with_context()` correctly but downstream callers `.unwrap_or_default()` the result, dropping the contextual error so the user can't tell why the config "looks empty".
- **Fix**: When falling back, log via `eprintln!` (or `warn!` to a dedicated terminal layer) before defaulting.

### E8. Inline doctor probes use `.ok()` / `unwrap_or_default()` without breadcrumb
- **Severity**: low
- **Area**: doctor
- **File**: `src/commands/doctor.rs:580, 1096, 1103`
- **Description**: When an os-release / sysfs probe fails, the diagnostic is dropped. Users see "unknown OS" with no actionable hint that EACCES caused it.
- **Fix**: Log the underlying io::Error at `debug!` before falling back — visible with `-v`.

### E9. Inconsistent `Config::default_or_loaded` fallback strategy
- **Severity**: medium
- **Area**: config / commands
- **File**: cross-cutting (e.g. `src/commands/events.rs:133` vs `src/commands/apply.rs`)
- **Description**: Some commands `?`-propagate config parse errors; others `unwrap_or_default()` and silently use defaults. Same misconfig produces different behavior depending on subcommand.
- **Fix**: Centralize via a helper `config::load_or_warn_then_default()` that always emits a `warn!` (or stderr line) when falling back.

### E10. DNS production paths share `.unwrap()` patterns with tests
- **Severity**: medium (audit-target)
- **Area**: dns
- **File**: `src/dns/mod.rs` (around 464-490)
- **Description**: Test setup uses bare `.unwrap()` on `fs::write`, but adjacent production code lacks `.with_context(...)` on the corresponding writes. Hard to tell at a glance which sites are test vs production.
- **Fix**: Audit each non-`#[cfg(test)]` `fs::*` site in `src/dns/mod.rs` and add path-bearing `.with_context()`.

---

## Section 7 — Docs vs code drift

### D1. `[discovery]` section in examples uses field names that don't exist
- **Severity**: high
- **Area**: examples / config
- **File**: `examples/standard.toml:64-72` (and the same block in `aggressive.toml`, `paranoid.toml`, `captive-portal-heavy.toml`, `disabled.toml`, `development.toml`, `minimal.toml`)
- **Verified**: `RawDiscoveryConfig` (`src/config.rs:719-724`) accepts only `mdns_silence`, `llmnr_silence`, `ssdp_block`, `wsd_block`. The examples write `mdns_responder`, `mdns_resolve`, `llmnr`, `netbios`, `wpad`, `ntp_normalize` — six unknown keys per file. With `#[serde(default)]` they are silently ignored at parse time.
- **Impact**: A user who copies `examples/standard.toml` to `/etc/proteus/config.toml` gets none of the silencing they think they configured. This is the primary onboarding path for new users.
- **Fix**: Update every example file to use the real field names; add `#[serde(deny_unknown_fields)]` or a soft-validate pass that emits warnings for unknown keys at load time so future drift surfaces immediately.

### D2. `[rotation]` section in examples does not exist in config schema
- **Severity**: high
- **Area**: examples / config
- **File**: `examples/standard.toml:133-139` (and at least 6 other example files)
- **Verified**: `RawConfig` (`src/config.rs:255-275`) has no `rotation` field. Examples document keys (`interval`, `on_probe_fail`, `on_link_change`, `on_ssid_change`) that do nothing.
- **Impact**: Users believe they configured rotation triggers; nothing happens. The behavior is governed elsewhere (`mac.rotation_interval` and the events daemon). Worse: the comment at line 134 says `[rotation] wins` over `mac.rotation_interval` — straightforwardly false.
- **Fix**: Either implement the section as documented (preferable, since the per-trigger granularity is useful) or remove from every example and rewrite the `# disagree, [rotation] wins` comment.

### D3. `[probes] enabled = true` in examples — no such field
- **Severity**: medium
- **Area**: examples / config
- **File**: `examples/standard.toml:88`
- **Verified**: `RawProbesConfig` (`src/config.rs:728-734`) has only `quorum_n`, `quorum_total`, `interval`, `cooldown`, `endpoints`. No `enabled`.
- **Fix**: Remove the line from each example, or add an `enabled` field if the toggle is intended.

### D4. `[mac]` examples use `exclude_gateways`, `exclude_arp_table`, `per_connection` — none exist
- **Severity**: high
- **Area**: examples / config
- **File**: `examples/standard.toml:30-32`
- **Verified**: `RawMacConfig` (`src/config.rs:662-666`) has only `enabled`, `rotation_interval`, `oui_pool`. The three keys above are silently ignored.
- **Impact**: User believes gateway-MAC and ARP-cache exclusion is on; it never was. `per_connection` likewise — users enabling persona-per-connection logic via this key get nothing.
- **Fix**: Either implement the three flags (looks like a real feature gap) or strip from examples and document in the wiki that they were never wired.

### D5. Exit code 75 (`LOCK_BUSY`) not documented in wiki/cli.md
- **Severity**: medium
- **Area**: wiki / cli docs
- **File**: `wiki/cli.md`
- **Description**: `lib.rs::exit::LOCK_BUSY = 75` (issue #211) is intentionally distinct from generic error so wrappers can retry. The wiki's exit-code lines for `rotate`, `apply`, `revert`, `pin`, `unpin`, etc. don't mention 75 — wrappers reading the wiki will treat lock contention as fatal.
- **Fix**: Add `· '75' state lock busy (another proteus instance running)` to every mutating-command exit-code line.

### D6. `proteus doctor` documented to return exit 2; never does
- **Severity**: low
- **Area**: wiki
- **File**: `wiki/cli.md:106`
- **Description**: Wiki claims `2` is "invalid args"; doctor only returns 0 or 1. Exit 2 is clap's default for argument errors but applies globally.
- **Fix**: Drop the `· '2' invalid args` clause, or move to the introduction as a global-clap note.

### D7. README "38-page wiki" undercounts current set
- **Severity**: info
- **Area**: README
- **File**: `README.md:13`
- **Verified**: `wiki/` contains 45 markdown files.
- **Fix**: Update to "45-page wiki" or use a non-numeric phrase.

### D8. `config set-profile` listed in flag exception, not documented as subcommand
- **Severity**: low
- **Area**: wiki/cli.md
- **File**: `wiki/cli.md:15`
- **Description**: The exception list in the global-flags description references `config set-profile`, but the `config` subcommand reference doesn't include it. Users searching for "set-profile" find a glancing mention with no usage / exit-code entry.
- **Fix**: Add a dedicated subsection under `config` describing `config set-profile <profile>`.

---

## Section 8 — CLI dispatch, --yes enforcement, watch mode

### CL1. Watch mode accepts `--interval 0s`, busy-loops at 100% CPU
- **Severity**: high
- **Area**: cli / watch
- **File**: `src/commands/watch.rs:67-85` (parse), `:52` (loop), test at `:94`
- **Verified**: `parse_interval("0s")` returns `Duration::from_secs(0)`. The watch loop calls `std::thread::sleep(Duration::ZERO)` and immediately re-renders, pegging a CPU.
- **Impact**: User typo or accidental `--interval 0s` burns a CPU. Trivial DoS against the user's own machine; not exploitable remotely but a clear UX regression.
- **Fix**: In `parse_interval`, reject zero (and probably anything below 50–100 ms). Add a test for the rejection.

### CL2. `--yes` flag silently dropped on Bluetooth / Hostname / Dns / Resolved / Ntp / Portal mutators
- **Severity**: critical (UX safety regression)
- **Area**: dispatch / actions
- **File**: `src/cli/dispatch.rs:140-249`
- **Verified**: dispatch arms for these actions destructure with `{ .. }` and call command functions that do not take a `yes` parameter:
    - `BluetoothAction::Apply { .. }` → `bluetooth_cmd::apply(state, config)`
    - `HostnameAction::Rotate { .. }` → `hostname::rotate(state, config)` (and `Pin`, `Revert`)
    - `DnsAction::Apply { .. }` → `dns::apply(config)` (and `Revert`)
    - `ResolvedAction::Apply { .. }` → `resolved::apply(config)` (and `Revert`)
    - `NtpAction::Apply { .. }` → `ntp::apply(config)` (and `Revert`)
    - `PortalAction::Mark { .. }`, `Unmark { .. }`, `Open { .. }` — all destructured but `yes` discarded
  Compare to `Ipv6 { yes }`, `Stack { yes }`, `EnterpriseWifi { yes }` which DO pass it through.
- **Impact**: A user running `proteus dns apply` *without* `--yes` will succeed without confirmation, contradicting the README's "mutating commands need `--yes`" contract. Every wrapping script that today refuses to call mutators absent `--yes` is bypassable.
- **Fix**: Update each dispatch arm to pass `yes` through, then have each command function call `require_yes(yes)?` (returns `CONFIRMATION_REQUIRED` / 65). Add an integration test per command verifying that absent `--yes`, the command exits 65 without effect.

### CL3. Action structs declare `yes: bool` for Portal, Hostname — fields are dead code
- **Severity**: medium (cleanup follow-up to CL2)
- **Area**: cli / actions
- **File**: `src/cli/actions.rs:83-106` (HostnameAction), `:116-136` (PortalAction)
- **Description**: Each action's `yes: bool` field is parsed by clap but never read by the dispatch or command functions. After fixing CL2, ensure these fields are actually used; if a maintainer decides certain mutators don't need confirmation, remove the dead fields.
- **Fix**: Tied to CL2.

### CL4. 24 of 37 subcommands have NO integration test scenario
- **Severity**: medium (test-quality)
- **Area**: tests / integration
- **File**: `tests/integration/scenarios/`
- **Description**: Subcommands without any scenario coverage: `bluetooth`, `completions`, `dhcp`, `diff`, `dns`, `dry-run`, `enterprise-wifi`, `events`, `help`, `hostname`, `kill`, `nft`, `ntp`, `persona`, `pin`, `portal`, `probe`, `resolved`, `resume`, `rf`, `session`, `ssid`, `stack`, `unpin`. CL2 went undetected for the same reason: no scenario asserts `--yes` is enforced.
- **Fix**: Add a smoke scenario per subcommand group. Minimum: read subcommands exit 0; mutators reject without `--yes` (after CL2 lands).

### CL5. Subcommand prefix collisions (`s`, `se`)
- **Severity**: low
- **Area**: cli
- **File**: `src/cli/command.rs:16-30`
- **Description**: `Status` has alias `s`. `Session` has no alias but is reachable by clap's prefix-match. `proteus se` is currently ambiguous. Adding any future `Settings` / `Set*` subcommand would silently break `proteus se` for users who picked it up from prefix-matching habit.
- **Fix**: Either disable clap's auto-prefix (set `infer_subcommands(false)` or add explicit alias `se` to `session`) and add a regression test for the desired behavior.

### CL6. `--json` flag distribution inconsistent across read commands
- **Severity**: low
- **Area**: cli
- **File**: `src/cli/command.rs` various
- **Description**: `--json` is on `status`, `session`, `current`, `original`, `show-config`, `show-defaults`, `diff`, `doctor`, `probe`. Missing from `resume` and from `wiki` (only `wiki search` has it). Script authors must special-case.
- **Fix**: Audit, document the policy ("read commands → --json; write commands → no --json"), enforce with a small clap-level macro or a code-gen step.

### CL7. `--interval 1ms` accepted; sleep granularity makes it effectively a busy loop
- **Severity**: low (related to CL1)
- **Area**: cli / watch
- **File**: `src/commands/watch.rs:99-101` (test)
- **Description**: The kernel scheduling resolution on stock kernels is ~1–10 ms; sleeping 1 ms loops nearly as tight as 0.
- **Fix**: Floor at 50–100 ms with a `warn!` if the user requested less.

---

## Section 9 — Network backends + events daemon

### N1. `RotateOnTriggerHandler` does not actually rotate
- **Severity**: high
- **Area**: events daemon
- **File**: `src/commands/events.rs:69-119`
- **Verified**: The handler increments a counter and emits an `info!` line that says `"events: trigger observed; rotating via backend"`, but the rotate body is gated behind a "follow-up" comment. `commands::rotate::run_with_backend` is imported but never called. README-level claim that `proteus events run` "subscribes to NM connection-up / link-flap / regulatory-domain / portal-auth events and re-applies the right policy per SSID" is **not implemented** — only logging fires.
- **Impact**: Users enabling `[events] enabled = true` in config see correct logs in journald, are convinced rotation is happening, but their MAC never rotates on those events. Significant doc-vs-code mismatch with security implications (a user thinks they have rotation-on-portal-auth; they don't).
- **Fix**: Either (a) finish the wire — design the runtime ownership story so `run_with_backend` can be called from the handler — or (b) ship a top-of-file warning to README and `wiki/per-ssid.md` and demote the trigger log lines accordingly.

### N2. `factory::permanent_address` returns `Option`, hides read errors
- **Severity**: high
- **Area**: backend / mac factory
- **File**: `src/backend/nm.rs:178-182`, `src/mac/factory.rs:50-58`
- **Description**: The function returns `Option<String>`, collapsing "no factory MAC available" with "sysfs read failed". Callers can't distinguish a transient permission/EIO from a structural absence; the rotate path silently chooses `NoFactoryMac`.
- **Fix**: Change return type to `Result<Option<String>>` and propagate the error chain. `rotate_if_needed` can then log the underlying error before deciding to skip.

### N3. NM DBus interface assumed without version probe
- **Severity**: medium
- **Area**: nm
- **File**: `src/nm/mod.rs:12-16`
- **Description**: Code uses `Device.Reapply` (NM ≥ 1.2). On older NM, fails with cryptic DBus error rather than a clear "your NetworkManager is too old; need ≥ 1.x".
- **Fix**: Probe version via `org.freedesktop.NetworkManager` `Version` property at startup; cache the result; fall back gracefully.

### N4. zbus errors stripped of method/path context
- **Severity**: medium
- **Area**: nm
- **File**: `src/nm/mod.rs:35-39` and many `.await?` sites
- **Description**: Plain `?` on `proxy.method().await` propagates a bare zbus error. Operators see "Method not found" with no info on which device path or method was attempted.
- **Fix**: Wrap each call site with `.with_context(|| format!("{interface}.{method} on {path}"))?`. A small macro helps.

### N5. Connection-mutation tests don't cover the full GetSettings → Update round-trip with PSK
- **Severity**: medium
- **Area**: tests / nm
- **File**: `src/nm/apply.rs:11-47` (set_cloned_mac), `:66-84` (set_scan_rand_mac), test at `:384-400`
- **Description**: The existing test only exercises the helper that merges `802-11-wireless-security` keys; it does not run the full GetSettings → mutate → Update flow against a fixture that holds a PSK. A regression in any of the four secrets-merge sites (issue #207) wouldn't be caught.
- **Fix**: Extend `MockBackend` to retain stored secrets keyed by uuid; add tests that round-trip the four sites: `set_cloned_mac`, `dhcp::update_connection`, `ipv6::apply_settings`, `enterprise_wifi::write_anonymous_identity`.

### N6. Connection lookup mixes `id` and `uuid` keys
- **Severity**: medium
- **Area**: nm
- **File**: `src/nm/mod.rs:213-231` (`find_connection_by_id`)
- **Description**: Issue #124 / #209 standardized on uuid as primary key, but `find_connection_by_id` still exists and uses `id`. If any caller mistakenly passes an `id` (the SSID name) where the system has two same-named connections (different security types), the wrong connection is mutated.
- **Fix**: Audit all callers of `find_connection_by_id`; if none remain, delete it. Otherwise add a `unique_by_id_or_error` helper that returns `Err` when multiple match.

### N7. Factory MAC fallback (#208) only tested on the happy fallback path
- **Severity**: medium (test gap)
- **Area**: tests / mac factory
- **File**: `src/mac/factory.rs:50-58, 264`
- **Description**: Test covers `addr_assign_type == 0` (factory). No test for missing `addr_assign_type` file, missing `address` file, or both.
- **Fix**: Add fixtures via a temp directory wrapper (or trait-injected sysfs).

### N8. Link-flap detector has no per-trigger debounce
- **Severity**: medium
- **Area**: events / link_flap
- **File**: `src/events/source/link_flap.rs:184-202`
- **Description**: Window-based count; when an AP genuinely flaps every 30 s, the source fires every 30 s. With N1 fixed (rotation actually happens), this would rotate the MAC every 30 s — burns through the OUI pool and looks anomalous to DHCP servers.
- **Fix**: Add per-iface "last fired" timestamp; require a minimum cooldown (e.g. `link_flap_min_cooldown_secs`) between successive triggers. Make it configurable; default ~5 min.

### N9. Captive portal probe doesn't validate TLS, doesn't follow redirects
- **Severity**: medium
- **Area**: captive_portal
- **File**: `src/captive_portal/mod.rs:87-113`
- **Description**: Plain HTTP/1.0 probe; `Connection: close`. A 3xx is treated as "portal required" without following — fine for detection, but if the redirect chain loops or terminates at an intercepted endpoint, the detector fires `PortalAuth` repeatedly. Combined with N8 (no debounce) this rotates aggressively.
- **Fix**: Cap follow at 2 hops; treat `>2 hops` as "still captive"; debounce by N8's mechanism.

### N10. Captive portal `to_socket_addrs` is sequential — IPv6-first stalls budget
- **Severity**: low
- **Area**: captive_portal
- **File**: `src/captive_portal/mod.rs:301-340`
- **Description**: When the host resolves to both A and AAAA records, the loop tries each in order. A non-routable IPv6 burns the connect timeout before IPv4 gets its turn.
- **Fix**: Race connects (Happy-Eyeballs-lite) or cap per-address timeout to `total / count`.

### N11. Init-system detection paths hardcoded
- **Severity**: low
- **Area**: init
- **File**: `src/init/*.rs`
- **Description**: Detection probes specific paths like `/run/systemd/system`. A non-standard layout falls through to a default.
- **Fix**: Allow override via env var `PROTEUS_INIT_SYSTEM=systemd|openrc|runit|sysvinit`; documented in `wiki/distro-support.md`.

### N12. Events daemon: missing `DeviceAdded` subscription
- **Severity**: medium
- **Area**: events / nm_connection_up
- **File**: `src/events/source/nm_connection_up.rs:141-142` (TODO)
- **Description**: Devices added after daemon start (USB Wi-Fi dongle plugged in) are never subscribed to. Code comment acknowledges this as future work.
- **Fix**: Subscribe to `org.freedesktop.NetworkManager.DeviceAdded`; on each new device, attach the same per-device proxy. Consider rate-limiting to prevent storms during cold-plug.

### N13. Mock backend mutex poisoning surfaces as test panics
- **Severity**: low (test-only)
- **Area**: tests / mock backend
- **File**: `src/backend/mock.rs:232+`
- **Description**: `Mutex<Inner>` can poison if a test panics inside a method. Subsequent tests using the same backend panic with "mutex poisoned".
- **Fix**: Use `lock().unwrap_or_else(|e| e.into_inner())`; or create a fresh backend per test (which most tests already do).

### N14. Per-SSID policy not debounced against concurrent CLI rotate
- **Severity**: low
- **Area**: events / rotate
- **File**: `src/commands/events.rs:96-110` (handler) + CLI rotate path
- **Description**: Once N1 is fixed, a manual `proteus rotate --yes` running concurrently with a daemon-fired rotate writes the same connection profile twice. The state lock serializes globally but doesn't dedupe per-uuid — both writes succeed in succession and one of the rotations is a wasted OUI burn.
- **Fix**: After acquiring the state lock, check for a recent rotate on the same uuid (within ~1 s); skip if duplicate.

---

## Section 10 — Resource leaks / performance

### R1. nft script stdin not explicitly closed before `wait_with_output()`
- **Severity**: medium-high
- **Area**: nft
- **File**: `src/nft/mod.rs:326-343`
- **Description**: After writing the ruleset to the child's stdin, the code relies on the implicit drop at end-of-scope to close the pipe. Under load (or with a particular kernel scheduling order), `nft` can block on its read waiting for EOF that hasn't arrived yet because the writer-side fd is still alive. The same scope then calls `wait_with_output()` which races with the implicit drop.
- **Fix**: `drop(stdin);` (or take + drop explicitly) immediately after the last `write_all`, before `wait_with_output`.

### R2. `nft list table` reads entire ruleset via `.output()`
- **Severity**: low-medium
- **Area**: nft
- **File**: `src/nft/mod.rs:260-290`
- **Description**: `.output()` allocates an unbounded `Vec<u8>` for stdout. The Proteus table is small, but `nft list ruleset` (or table inheritance) can pull thousands of lines from sibling tables. On embedded / container hosts with tight memory limits this is wasteful.
- **Fix**: Cap the read at e.g. 1 MiB via a streaming reader; bail with a clear error past the cap.

### R3. DHCP status creates a fresh DBus connection per call
- **Severity**: medium (when called from a daemon)
- **Area**: dhcp
- **File**: `src/commands/dhcp.rs:380-387`
- **Description**: `gather_status()` calls `zbus::Connection::system().await` on every invocation and never explicitly closes. For a one-shot CLI this is fine; if the events daemon ever polls DHCP status (Roadmap M4c), one connection per poll accumulates fds.
- **Fix**: Cache the system connection in a struct held by the daemon; explicitly `conn.close().await.ok();` at end of CLI gather paths.

### R4. Events daemon caches captive-portal config at startup; never reloads
- **Severity**: medium
- **Area**: events / captive_portal
- **File**: `src/commands/events.rs:183-187`
- **Description**: `SystemPortalSampler::new(detect_url, expected_response, timeout_secs)` is built once when the daemon spawns sources. Config edits to `[captive_portal]` are ignored until the daemon restarts, even though the trigger handler reloads config per trigger.
- **Fix**: Either teach the sampler to reload config each cycle, or document the restart requirement in `wiki/captive-portals.md`.

### R5. `String::from_utf8_lossy(...).into_owned()` chain on subprocess output
- **Severity**: low (cosmetic / micro-perf)
- **Area**: cross-cutting
- **File**: `src/nft/mod.rs:266`, `src/kill_switch/mod.rs:188`, `src/dns/mod.rs:163`
- **Description**: `lossy().into_owned()` clones once; chaining `.trim().to_string()` clones again. On stdouts known to be ASCII (`ip`, `nft`, `resolvectl`) the lossy path is wasteful.
- **Fix**: Use `String::from_utf8(bytes)` and propagate a typed error if conversion fails. For trim-then-own sites, prefer `String::from(stdout.trim())` after a single utf8 conversion.

### R6. Wiki search re-scans every page every term
- **Severity**: low
- **Area**: wiki
- **File**: `src/wiki.rs:80-156`
- **Description**: Search complexity is `O(terms × pages × lines)`. Today (45 pages × ~150 lines × ≤4 terms ≈ 27,000 iterations) this is fast; if the wiki grows past ~200 pages the per-query cost compounds.
- **Fix**: When the wiki passes ~200 pages, generate a build-time index in `build.rs` (`HashMap<term, Vec<(page, line_no)>>`) and embed it via `include!`.

### R7. DHCP renew allocates 3-4 `String`s per `RenewOutcome`
- **Severity**: low
- **Area**: dhcp
- **File**: `src/commands/dhcp.rs:236-273`
- **Description**: Each outcome's `method` and `note` could be `&'static str` (the method names are constants); the per-iface allocation count is small but easy to halve.
- **Fix**: Switch the constant fields to `&'static str`.

### R8. Implicit subprocess fd close everywhere
- **Severity**: info
- **Area**: cross-cutting
- **File**: many — every `Command::new(...).output()` site
- **Description**: Rust drops `Child` and its piped streams on scope exit, which is correct, but adding a one-line comment at hot sites reduces audit confusion when readers ask "where do these fds get closed?".
- **Fix**: No code change required; consider a single-line `// child + stdin/stdout/stderr fds closed when `output` is dropped` at one canonical helper.

---

## Section 11 — Bonus findings (manual review)

### M1. `DhcpAction::Apply { .. }` and `DhcpAction::Revert { .. }` also drop `--yes`
- **Severity**: critical (extension of CL2)
- **Area**: cli / dispatch
- **File**: `src/cli/dispatch.rs` (DHCP arms)
- **Verified**: same pattern as CL2: `DhcpAction::Apply { .. } => commands::dhcp::apply(state, config)` (no `yes` parameter on the function). Adds `dhcp apply` and `dhcp revert` to the list of mutators that proceed without `--yes`. `dhcp renew` correctly passes `yes`.
- **Fix**: Same as CL2 — pass `yes`, enforce `require_yes(yes)?` in `dhcp::apply` and `dhcp::revert`.

### M2. `mem::forget(guard)` in `write_atomic` is intentional, not a leak
- **Severity**: info (no fix needed; flag for future readers)
- **Area**: commands / atomic write
- **File**: `src/commands/mod.rs:235`
- **Description**: After a successful rename, the temp file IS the destination; the cleanup guard would otherwise delete the just-written file on drop. `mem::forget` here is correct and intentional. Confirmed by the surrounding comment.
- **Fix**: None — this is documented in code. Leaving the note here so future audits don't re-flag it.

### M3. Subprocess interface-name validators allow `b';' b'&' b'|'` (ASCII graphic)
- **Severity**: info
- **Area**: kill_switch / rf
- **File**: `src/kill_switch/mod.rs:174-180` (`is_safe_iface`)
- **Description**: `iface.bytes().all(|b| b != b'/' && b != 0 && b.is_ascii_graphic())` permits `;`, `&`, `|`, backticks. Today this is safe because `Command::args` uses `execve` (no shell), so injection is not exploitable. If anyone ever shell-formats the iface (e.g. into a `bash -c` or a script template), it becomes exploitable.
- **Fix**: Tighten to alphanumeric + `_`, `-`, `.` only — that matches kernel-allowed interface names anyway. Add a regression test.

### M4. `Cargo.toml:7` description text exceeds 256-char crates.io limit (likely)
- **Severity**: low (publication risk)
- **Area**: cargo
- **File**: `Cargo.toml:7`
- **Description**: The `description` field is a single very long sentence (counted: ~480 chars). When `cargo publish` is invoked, crates.io truncates to 256; the truncated text reads awkwardly.
- **Fix**: Trim to ≤ 250 chars; keep the long version in `README.md`.

### M5. Rustc 1.93.0 toolchain pin vs `rust-version = "1.85"`
- **Severity**: info
- **Area**: cargo
- **File**: `rust-toolchain.toml`, `Cargo.toml`
- **Description**: The pinned toolchain (1.93.0) is well above MSRV (1.85). Edition 2024 is supported in both. The note is here only to pre-empt the easy-to-make false positive that "edition 2024 doesn't work on stable" — it does, since 1.85.
- **Fix**: None — both pins are correct; flagging so future audits don't repeat the mistake.

---

## Section 12 — Second-pass findings (2026-05-08)

This section captures additional bugs found in a second focused sweep on top of
the original 85-issue queue. New prefixes (`N*`) reduce collisions with the
existing numbering. Each finding was personally verified against the current
source.

### N12.1. `unpin` subcommand has NO `--yes` field at all
- **Severity**: critical
- **Area**: cli / dispatch / mutator-confirmation
- **File**: `src/cli/command.rs:126-129` (`Unpin { target: String }` — no `yes` field), `src/commands/unpin.rs:10` (no `yes` parameter)
- **Description**: Compare to `Pin` at `src/cli/command.rs:120-124` which carries `yes: bool`. `Unpin` is the asymmetric exception: clap cannot accept `--yes` because the field is absent, and `commands::unpin::run` never calls `require_yes()`. The function does call `acquire_state_lock_or_print` and `state.save(...)` — so the path *is* mutating — but the safety gate is wholly missing. Different from CL2/M1: those drop a present field at dispatch; this one never declared the field at all.
- **Impact**: `proteus unpin <target>` mutates `state.json` (sets `pinned = None` on a managed interface or connection record) without any `--yes` confirmation. Wrappers cannot enforce the "mutators need `--yes`" contract on this path. A misclick / typo'd target with `unpin` paired with shell completion is a vector for a state corruption that downstream apply runs will then "respect" until the user notices.
- **Fix**: Add `yes: bool` to the `Unpin` clap action, thread it through `dispatch.rs`, and call `require_yes(yes, "is mutating (clears MAC pin)", "proteus help unpin")?` before `acquire_state_lock_or_print`. Add an integration test: `proteus unpin wlan0` without `--yes` must exit `CONFIRMATION_REQUIRED` (65) without writing state.

### N12.2. `dhcp apply` and `dhcp revert` drop `--yes` (extension of CL2/M1)
- **Severity**: critical
- **Area**: cli / dispatch
- **File**: `src/cli/dispatch.rs` (DHCP arms), `src/commands/dhcp.rs:85, 125` (function signatures lack `yes`)
- **Description**: Mirror of CL2/M1. `DhcpAction::Apply { yes }` and `Revert { yes }` parse the flag but the dispatch destructures with `{ .. }`, dropping the bool; downstream functions take no `yes` parameter. Note: `dhcp renew` does pass `yes` correctly, so this is genuinely an apply/revert oversight (M1 in the existing list mentioned the pattern but specifically targeted apply only and was tagged "extension of CL2"). Re-recording here so the fix list is complete and clean.
- **Impact**: Same as CL2/M1 — `proteus dhcp apply` and `proteus dhcp revert` proceed without confirmation. Wrapping scripts cannot enforce the contract.
- **Fix**: Pass `yes` through dispatch; call `require_yes(yes, …)?` at the top of both functions before any state-lock acquire.

### N12.3. `portal mark`, `portal unmark`, `portal open` drop `--yes`
- **Severity**: critical
- **Area**: cli / dispatch
- **File**: `src/cli/dispatch.rs` (Portal arms), `src/commands/portal.rs:158, 184, 206`
- **Description**: `PortalAction::Mark { ssid, yes }`, `Unmark { ssid, yes }`, `Open { yes }` declare the field; dispatch destructures with `{ ssid, .. }` / `{ .. }` and drops it. `run_mark`, `run_unmark`, `run_open` take no `yes` parameter. Mutates `state.known_portal_ssids` (mark/unmark) or shells out to `xdg-open` style helpers (open) without the gate.
- **Impact**: Three more mutators bypassing `--yes`. Combined with N12.2 and the existing CL2/M1, every action in the dhcp + portal namespaces silently lacks confirmation enforcement.
- **Fix**: Identical pattern: pass `yes` through, gate with `require_yes(yes, …)?`. Add integration tests for each action (currently in the CL4 gap).

### N12.4. Per-SSID `parse_duration` multiplication overflow
- **Severity**: high
- **Area**: per_ssid / duration parsing
- **File**: `src/per_ssid.rs:218-221`
- **Description**: `match unit { "m" => Some(Duration::from_secs(n * 60)), "h" => ... n * 3600, "d" => ... n * 86_400, ... }`. The `n: u64` value is unchecked-multiplied. For e.g. `n = u64::MAX / 60 + 1` and unit `"m"`, the multiply wraps in release (silent — produces a `Duration::from_secs` of a wildly wrong number) and panics in debug. With `panic = abort` set crate-wide (Cargo.toml:14), a panic here aborts the events daemon. `is_valid_per_ssid_duration` (#N12.5 below) does not bound the magnitude either, so a per-SSID config with `rotate_interval = "999999999999999999d"` survives load-time validation and lands at this multiply.
- **Impact**: A hostile or buggy per-SSID config overflow-wraps the rotation cadence (returns a tiny Duration from a giant input) — turning a configured "do not rotate often" into "rotate continuously". With debug builds or future strict overflow settings, the events daemon aborts.
- **Fix**: Use `n.checked_mul(factor).and_then(|s| ... )` and return `None` on overflow so the resolver transparently falls back to the global timer. Alternatively `n.saturating_mul(factor)` if "very large means very long" is the desired semantic.

### N12.5. `is_valid_per_ssid_duration` panics on multibyte trailing char (regression of #272)
- **Severity**: high
- **Area**: config / per_ssid validator
- **File**: `src/config.rs:940-953`
- **Description**: Issue #272 fixed `per_ssid::parse_duration` (`src/per_ssid.rs:200-224`) to use `char_indices().next_back()` instead of `s.split_at(s.len() - 1)`, because the byte-boundary split panics on multibyte UTF-8 trailing characters. The companion validator `is_valid_per_ssid_duration` in `src/config.rs:945` was NOT updated and still uses `s.split_at(s.len() - 1)`. With `s = "µ"` (length-2 UTF-8 sequence: 0xC2 0xB5), `s.len() - 1 == 1`, and `split_at(1)` lands mid-codepoint → panic. With `panic = abort`, this aborts the binary on any config-load path that calls `validate_ranges()` (which is called from `Config::load`/`default_or_loaded`). Trigger: a per-SSID block with `rotate_interval = "µ"` (or any 2-byte-min UTF-8 string).
- **Impact**: A hostile or accidentally-pasted per-SSID `rotate_interval` aborts every command that loads config, including read-only `proteus status`, `proteus current`, `proteus diff`, etc. Same root cause as #272, only re-introduced because the validator was a copy of the old shape.
- **Fix**: Mirror the #272 fix: switch to `char_indices().next_back()` for the split point, reject non-ASCII unit suffixes as "off-format". Better: have `validate_ranges` call `parse_duration` directly so the two functions share one definition.

### N12.6. `display_string` length-clamp counts input characters, not output
- **Severity**: medium
- **Area**: display / output sanitization
- **File**: `src/display.rs:36-77`
- **Description**: The clamp logic increments `emitted` once per input character regardless of how many output characters that produced. `\\` outputs 2, `\xNN` outputs 4, `\u{NNNN}` outputs 6+. A pathological input of 1024 backslashes hits `emitted == MAX_DISPLAY_LEN` after consuming all input — but the output string is now 2048 characters long. The contract documented at module top ("Output is clamped to `MAX_DISPLAY_LEN` characters; … the operator can spot the clamp") is violated. The existing tests at `src/display.rs:162` only exercise plain `'A'`, never escape-expanding chars, so the bug is not caught.
- **Impact**: A hostile peer (captive-portal `Location:` header, NM SSID, etc.) can plant 4 KiB of `\` characters and produce ~8 KiB of output that wraps the operator's terminal — defeating the wrap-prevention guarantee `display_string` was authored for. Not a security boundary, but the function is the boundary's enforcement, so the regression matters.
- **Fix**: Change the loop to track output length: increment by the size of what was just appended (`out.len()` delta or an explicit count of chars pushed). Add a regression test using `"\\".repeat(MAX_DISPLAY_LEN + 100)` and assert the result is clamped.

### N12.7. Captive portal sends RFC-7230-malformed `Host:` header for IPv6 literal URLs
- **Severity**: medium
- **Area**: captive_portal / HTTP
- **File**: `src/captive_portal/mod.rs:253-262, 314-318`
- **Description**: `parse_http_url` strips IPv6 literal brackets (`[::1]` → `::1`) when storing `host` in `UrlParts`. `http_get` then formats `format!("Host: {}", parts.host)`, producing `Host: ::1` on the wire. RFC 7230 §5.4 requires IPv6 literals in the Host header to remain bracketed: `Host: [::1]`. A spec-compliant server (and many CDNs / portal redirectors that proxy through compliant front-ends) responds to the malformed header with `400 Bad Request`, which the classifier currently treats as `Classification::Unknown`. The detector then misreports the path's portal status.
- **Impact**: Captive portal detection silently fails on IPv6-literal `detect_url` configurations. Defaults to `nmcheck.gnome.org` which is not a literal, so the bug only fires when an operator points at an IPv6 literal — but IPv6-only test rigs and on-LAN portal probes are exactly that case.
- **Fix**: Track whether the URL's host was bracketed (an `Option<bool> bracketed` flag on `UrlParts` or a separate `host_for_header: String` field). Format `Host: [{}]` when bracketed; `Host: {}` otherwise. `(parts.host.as_str(), parts.port).to_socket_addrs()` in line 313 already accepts the unbracketed form for resolution, so only the format string needs to change.

### N12.8. `proteus-events.service` `ExecStart` path conflicts with install.sh default
- **Severity**: high
- **Area**: install.sh / packaging / systemd unit consistency
- **File**: `dist/systemd/proteus-events.service:15` (uses `/usr/bin/proteus`), vs `dist/systemd/proteus-{boot,check,rotate,resume}.service` (use `/usr/local/bin/proteus`), vs `install.sh:19` (`BINARY_DST="/usr/local/bin/proteus"`)
- **Description**: Verified by `grep ExecStart dist/systemd/*.service`:
  - `proteus-events.service:ExecStart=/usr/bin/proteus events run` — distro path
  - All four other unit files use `/usr/local/bin/proteus` — install.sh path
  install.sh's `sed` rewrite (lines 167-189) only patches the polkit policy; nothing rewrites systemd unit `ExecStart` lines. Result on install.sh-deployed systems: 4 of 5 units start, but `proteus-events.service` starts→fails→restarts every 5 s with exit 200 ("binary not found at /usr/bin/proteus"). Systemd's `Restart=on-failure` then thrashes.
  This is a more specific instance of the cross-cutting B10, with the additional twist that `proteus-events.service` is the ONLY unit using the distro path — every other unit follows the install.sh convention. Likely a copy-paste oversight when the events unit landed.
- **Impact**: install.sh-installed events daemon never runs; user sees `[events] enabled = true` honored at config-parse time but no triggers fire. Mirror of N1 in the existing list (handler doesn't actually rotate) — but for an entirely different reason: the daemon never even starts.
- **Fix**: Either (a) change `proteus-events.service` to use `/usr/local/bin/proteus` to match siblings (cheapest); or (b) extend install.sh's polkit-rewrite logic to `sed` `ExecStart=` paths in every unit it copies; or (c) standardize on `/usr/bin/proteus` everywhere and have install.sh symlink it from `/usr/local/bin/proteus`. (c) is the cleanest long-term but most invasive.

### N12.9. `proteus-events.service` lacks `KillMode` / `TimeoutStopSec`
- **Severity**: medium
- **Area**: systemd unit hardening / shutdown
- **File**: `dist/systemd/proteus-events.service`
- **Description**: The unit is `Type=simple` with `Restart=on-failure` but specifies no `KillMode` (default `control-group`), `KillSignal` (default `SIGTERM`), or `TimeoutStopSec` (default 90 s). Pairs with C4 in the existing list (no `tokio::signal` SIGTERM handler in `src/commands/events.rs:240`). On `systemctl stop proteus-events.service`, systemd sends SIGTERM to the entire control group; the daemon process exits without draining `EventSource::stop()` or flushing tracing — covered, but the unit defaults give the daemon a 90 s window to do nothing. After SIGKILL, in-flight DBus subscriptions and netlink sockets drop without graceful close.
- **Impact**: Resource churn on rapid restart (typical when an operator edits config and reloads). Log lines may be lost. systemd journal shows a 90 s delay between `systemctl stop` and the next `systemctl start` succeeding.
- **Fix**: Add `KillMode=mixed` (SIGTERM the main process; SIGKILL stragglers) and `TimeoutStopSec=10s` (a long-running daemon doesn't need 90 s to flush). Pair with C4's runtime fix to actually catch SIGTERM and call `shutdown_tasks(...)` cleanly.

### N12.10. `proteus diff` reads target files unbounded
- **Severity**: medium
- **Area**: diff / OOM
- **File**: `src/diff/mod.rs` (`std::fs::read(&path)` site)
- **Description**: The diff pass enumerates managed paths and `std::fs::read`s each into a `Vec<u8>` for SHA256 hashing and pretty-print. There's no upstream size cap. Managed paths today are small (state.json, config.toml, systemd drop-ins) but the SHA-verification loop in `src/commands/mod.rs:360` is reachable from `proteus apply` and operates on whatever the path enumerator surfaces. A dropped-in unit file at `/etc/systemd/system/proteus-rotate.timer.d/override.conf` could be replaced (root operator typo, or future feature gap) with a multi-GiB file and `proteus diff` / `proteus apply` would OOM.
- **Impact**: Local DoS via misconfiguration. Not security-critical (the path is root-only), but `proteus apply` failing to OOM is unhelpful — better a clear "managed file too large; investigate" error.
- **Fix**: Cap `read` at e.g. 10 MiB via a size check on `metadata().len()` first. For SHA verification, stream via `Read` chunks rather than reading the whole file (the `crypto/sha256` helper supports incremental hashing).

### N12.11. NM connection-up SSID stub always returns `None`
- **Severity**: medium
- **Area**: events / connection-up source
- **File**: `src/events/source/nm_connection_up.rs:312-323` (function `read_active_ssid_via_proc` or similar)
- **Description**: The function probes `/proc/net/wireless` to confirm a wireless interface is present, then unconditionally returns `None`. The comment says "deliberately stop here" (TODO) and the dispatcher path uses `CONNECTION_ID` for SSID instead — but the events-daemon path on networkd / raw backends has no CONNECTION_ID and silently can't resolve the SSID. Per-SSID policy (the `[per_ssid."<ssid>"]` block — the entire roadmap-M3 feature) cannot resolve at trigger time on those backends.
- **Impact**: Per-SSID overrides documented in the wiki silently no-op on non-NM hosts. This is a documentation-vs-code mismatch with security implications (a user thinks they enabled `pin_mac` for a specific SSID; they didn't).
- **Fix**: Either implement the sysfs SSID read (`/sys/class/net/<iface>/wireless/...` is not authoritative; `iw dev <iface> link` or NL80211 `NL80211_CMD_GET_INTERFACE` is the right source), or rename the function to `is_wireless_iface()` and document in the wiki that per-SSID resolution requires the NM backend.

### N12.12. `clap` `u32` flags accept implicit-positive of negative input
- **Severity**: medium
- **Area**: cli / clap parsing
- **File**: `src/cli/actions.rs:47` (`TimerAction::Logs { lines: u32 }` and similar `u32`/`u64` fields)
- **Description**: Plain clap `u32`/`u64` `value_parser` does not reject leading `-`; if the user types `--lines -50`, clap parses the next token as an `OsStr`, which fails u32 parse — but if a downstream caller uses `default_value_t = 0` and a custom converter, the contract is fragile. More directly: `--lines=18446744073709551566` (u64::MAX - 50) is accepted on a u64 field with no upper bound, and any `Vec`/`String` allocation that scales with that count is an OOM vector. Today: counter scans through `journalctl -n` which itself caps but the binary trusts the value.
- **Impact**: An unprivileged user with shell access (no root) can `proteus timer logs rotate --lines 18446744073709551566` and journalctl receives that. journalctl tolerates large `-n` values but the audit pattern (read-only command turning into resource amplifier) is worth tightening.
- **Fix**: For numeric flags, use `value_parser!(u32).range(0..=1_000_000)` or similar at every clap-typed integer site. Audit `src/cli/actions.rs` and `src/cli/command.rs` for unbounded `u32`/`u64` fields.

### N12.13. `state_lock::HELD` mutex held across `acquire_inner` retry sleep loop (still present)
- **Severity**: high (already C1 in existing list — re-confirmed in this pass)
- **Area**: state_lock
- **File**: `src/state_lock.rs:155-170` (current)
- **Description**: This was already filed as C1. Re-verified: `acquire_for_state_path` takes `HELD.lock()` at line 156, then calls `acquire_inner(&path)` at line 163, which executes the up-to-50× retry loop with `std::thread::sleep(RETRY_DELAY)` while still holding the `HELD` mutex. Concurrent acquirers from the same process serialize on this mutex even when the on-disk flock is the contention point. Also worth noting: `acquire_inner` now (after #275) also performs `ensure_state_dir_secure` + `set_permissions` on the `.lock` file — additional work that runs holding the in-process mutex. Nested calls correctly observe `held.is_some()` and return a no-op guard, so the issue only bites when *multiple* fresh outer-acquires race. Today this is most likely on the events daemon where four sources can simultaneously try to invoke a rotate.
- **Fix**: As C1 said — release the mutex before the retry sleep, re-acquire at the top of each iteration. Or rework the type to hold the file outside the mutex with the mutex only guarding the slot.

### N12.14. Captive portal detect: no maximum total request size before TLS would be needed
- **Severity**: low
- **Area**: captive_portal / HTTP
- **File**: `src/captive_portal/mod.rs:316-318`
- **Description**: The request line builder does `format!("GET {path} HTTP/1.0\r\nHost: {host}\r\n…")`. `parts.path` is from `parse_http_url`, which doesn't bound length. A `detect_url` like `http://example.com/aaaaaa…` (1 MiB of `a`'s) builds a 1 MiB request. `is_request_safe` checks character classes but not length. Today the operator owns `detect_url`, so this is benign — but if `detect_url` ever flows from a less-trusted source (e.g. an NM connection profile field), the unbounded `format!` allocates without ceiling.
- **Impact**: None today; future-proofing only.
- **Fix**: Cap `path.len()` at e.g. 4096 in `parse_http_url`; reject longer with `None`. Add a unit test pinning the cap.

### N12.15. `lock_path_for(state_path)`: `parent()` of bare filename returns `Path::new(".")`
- **Severity**: low
- **Area**: state_lock / fallback
- **File**: `src/state_lock.rs:172-175`
- **Description**: `state_path.parent().unwrap_or_else(|| Path::new("."))` — if a caller passes a state path with no parent (e.g. a bare `"state.json"`), the lock lands at `./.lock`. With `--state` flag, this is reachable. Combined with `O_NOFOLLOW` (good — already there) it's safe, but the lock then sits in $CWD which is operator-surprising.
- **Impact**: A `proteus apply --state state.json` invocation creates `.lock` in CWD; subsequent runs from a different CWD don't see it as held. Cross-process serialization fails for this exotic invocation.
- **Fix**: Either canonicalize `state_path` before extracting parent, or `bail!` when the input has no parent.

### N12.16. `state.save` chmod after `write_atomic` is racy under concurrent reader
- **Severity**: low
- **Area**: state.rs / file mode
- **File**: `src/state.rs:390-401`
- **Description**: After `write_atomic` lands `state.json` at 0o600, `state.save` calls `fs::set_permissions(path, 0o600)` belt-and-braces (line 394). Between the rename inside `write_atomic` and the explicit `set_permissions`, the file briefly exists at 0o600 (good — `write_atomic` opens with `.mode(0o600)`), so there's no window where it's wider. But the second `set_permissions` is essentially a no-op. If a future maintainer ever changes `write_atomic`'s mode and forgets to update `STATE_FILE_MODE`, the second call silently corrects only on platforms that honor it. Not exploitable today, but the redundancy is a foot-gun and mis-suggests "this is the authoritative chmod".
- **Fix**: Drop the second `set_permissions` and rely on `write_atomic`'s `.mode()`. Or, if defense-in-depth is intended, add a comment that explicitly documents the assertion semantics ("this fchmod is a defensive belt; `write_atomic` is the actual setter").

### N12.17. `is_uuid_shape` accepts uppercase hex; NM emits lowercase
- **Severity**: low
- **Area**: state migration
- **File**: `src/state.rs:457-473`
- **Description**: The shape check uses `b.is_ascii_hexdigit()` which matches both `[0-9a-f]` and `[0-9A-F]`. Real NM uuids are lowercase RFC-4122. A state file with uppercase uuids (manual edit, restored from a tool that uppercases hex) passes shape check but later string comparisons against fresh NM uuids (which are lowercase) miss → entries silently abandoned during migration / lookup.
- **Impact**: Edge-case data loss when a state file has uppercase uuids — `proteus revert` fails to find originals for those connections.
- **Fix**: Either (a) lowercase the key on load if it passes `is_uuid_shape`, or (b) tighten `is_uuid_shape` to reject uppercase. (a) is more permissive and recovers from the wild edit.

### N12.18. `unix_to_ymdhms` `t as i64` cast of u64 days
- **Severity**: info
- **Area**: commands / time formatting
- **File**: `src/commands/mod.rs:136`
- **Description**: `let mut days = t as i64;` after `t /= 24`. For any `SystemTime` post-2106 (when `u32` epoch overflows), `t > i64::MAX / 86400` becomes possible — but in practice we'd need `t` to exceed ~2.6e8 days (year 1970+700,000), so this is a "year 700,000 problem". Not a bug. Flagging so future audits don't re-flag the cast.
- **Fix**: None.

### N12.19. `factory::permanent_address` returns `Option<String>` — already filed as N2
- **Severity**: high (re-confirmed)
- **Area**: backend / mac factory
- **File**: `src/backend/nm.rs:178-182`, `src/mac/factory.rs:50-58`
- **Description**: Re-confirmed in this pass; same as N2 in the existing queue. Worth re-flagging because two different agents independently surfaced it, indicating it's a high-impact bug on the production path. The Option collapses I/O failure with structural absence.
- **Fix**: As N2 — change to `Result<Option<String>>`.

### N12.20. Subprocess interface-name validators allow ASCII shell metacharacters — duplicate of M3
- **Severity**: info (duplicate of M3)
- **Area**: kill_switch / iface validation
- **File**: `src/kill_switch/mod.rs:174-180`
- **Description**: Already M3. Re-confirmed.
- **Fix**: As M3.

---

### Summary of section-12 additions

20 entries. Severity distribution:
- 4 critical (`N12.1`–`N12.3` + part of `N12.8`'s functional impact)
- 6 high (`N12.4`, `N12.5`, `N12.8`, `N12.13`, `N12.19`)
- 5 medium (`N12.6`, `N12.7`, `N12.9`, `N12.10`, `N12.11`, `N12.12`)
- 4 low (`N12.14`, `N12.15`, `N12.16`, `N12.17`)
- 2 info (`N12.18`, `N12.20`)

Top three to fix first (impact × likelihood):
1. **N12.1** — `unpin` has no `--yes` field at all. Asymmetric vs `pin`. Trivial fix.
2. **N12.5** — `is_valid_per_ssid_duration` panic on multibyte trailing char. Same root cause as already-fixed #272 but in the validator. With `panic = abort`, every load path can be aborted by a malicious / fat-fingered SSID config.
3. **N12.4** — `parse_duration` overflow on multiply. Combined with the missing magnitude bound in `is_valid_per_ssid_duration`, an operator typo of `999999999999d` silently turns into a continuous-rotation footgun.

