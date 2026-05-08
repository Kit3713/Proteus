# Security audit follow-up — 2026-05-07 (v0.2.7-alpha) (ARCHIVED)

> **Archived 2026-05-08.** This document is preserved verbatim as the
> v0.2.7-alpha re-audit record. **Live status tracking has moved to
> [`docs/ROADMAP.md`](../ROADMAP.md).** Do not update status against this
> file; resolve work against the roadmap streams listed in the
> "Source-of-truth migration" section there. The original audit at
> [`SECURITY-AUDIT-2026-05-07.md`](./SECURITY-AUDIT-2026-05-07.md) is also
> archived.

## Where the open findings now live

This follow-up filed five new IDs (N-0 through N-3, plus T-1). N-2 and T-1
landed in `main`. The remaining three are absorbed into the roadmap:

| ID | Severity | Roadmap home |
|----|----------|--------------|
| N‑0 (regression of M‑2) | High | [Roadmap Stream 9](../ROADMAP.md) — `PROTEUS_*_DIR` env hardening |
| N‑1 | Low | [Roadmap Stream 4](../ROADMAP.md) — `ethtool -P` iface validation |
| N‑3 (residual) | Low | [Roadmap Stream 5](../ROADMAP.md) — `O_NOFOLLOW` on `state_lock` open |

The original-audit findings re-checked here (H-1 through I-2) are also covered
in the parent file's archival header. Use the finding text below for
implementation context; do not file a new issue against this file.

---

## New findings

## N‑0 (HIGH, regression of M‑2) — `PROTEUS_*_DIR` env vars still steer `remove_dir_all`

**File:** `src/commands/uninstall.rs:55, 99-121`

The previous audit flagged this as M‑2 (Medium). It is **unchanged** in
v0.2.7-alpha. Re-stating because every other Medium-and-above finding was
addressed:

```rust
// src/commands/uninstall.rs:55  (production entry point)
let layout = Layout::from_env();
// ...
// src/commands/uninstall.rs:107-121
impl Layout {
    fn from_env() -> Self {
        Self {
            config_dir:  env_path("PROTEUS_CONFIG_DIR",  DEFAULT_CONFIG_DIR),
            state_dir:   env_path("PROTEUS_STATE_DIR",   DEFAULT_STATE_DIR),
            systemd_dir: env_path("PROTEUS_SYSTEMD_DIR", DEFAULT_SYSTEMD_DIR),
        }
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    match std::env::var_os(key) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(default),
    }
}
```

Reproduction (still works on the audited tree):

```sh
sudo PROTEUS_STATE_DIR=/etc proteus uninstall --purge --yes
# → recursively removes /etc
```

`sudo` does strip these env vars by default, but:
* Operators with `Defaults env_keep+="PROTEUS_*"` (likely if they ran the
  test suite) inherit the override into production.
* Wrappers / Make targets that `sudo -E` preserve all of them.
* `pkexec` invocations under the bundled polkit policy may pass these
  through depending on how `org.freedesktop.policykit.exec.allow_gui` is
  resolved.

**Severity bumped to High** in this follow-up because the simpler workarounds
(symlink TOCTOU, captive-portal RCE) have been closed; this is now the
highest-impact remaining issue.

**Recommendation.** Either:

1. Gate `Layout::from_env()` on `#[cfg(test)]` and hardcode the production
   paths in the `run()` function. The test suite at lines 304-329 then
   needs to call a `Layout::for_test()` helper.
2. Or refuse to operate on a path outside an allowlist
   (`/etc/proteus`, `/var/lib/proteus`, `/etc/systemd/system`, plus their
   tempdir-prefixed test variants when `cfg(test)`).

Both are about ten lines of code.

**CVSS 3.1:** AV:L/AC:H/PR:H/UI:R/S:C/C:N/I:H/A:H → **6.0 Medium-High**.

---

## N‑1 (LOW) — `factory::permanent_address` invokes `ethtool -P <iface>` without iface validation

**File:** `src/mac/factory.rs:97-104`

The L‑3 fix landed `is_safe_iface` in `kill_switch/mod.rs` and `rf/mod.rs`,
but the new `mac/factory.rs` module (added in commit `9540582` as part of
the issue #123 fix) calls `ethtool` with an unchecked iface:

```rust
impl EthtoolRunner for EthtoolBin {
    fn permanent(&self, iface: &str) -> Option<String> {
        let out = Command::new("ethtool").args(["-P", iface]).output().ok()?;
        ...
    }
}
```

Every existing call site reaches this with kernel-trusted strings (sysfs
walk, NM device list), so the bug is latent. The other L‑3 sites got the
`is_safe_iface` guard exactly because audit policy is "validate at every
exec boundary, not just at currently-known callers." Same defense should
apply here — it is one if-statement.

**Recommendation.** Reuse `crate::ipv6::validate_iface_name` (or pull the
common bytes-validator into `crate::mac::iface`) and gate
`EthtoolBin::permanent` on it.

---

## N‑2 (LOW) — Terminal-escape injection via SSID and NM connection IDs printed to stdout

**Files:** `src/commands/session.rs:265-266, 718-722, 700-705`,
`src/commands/portal.rs:150-151,173,179,201`

Wi-Fi SSIDs are arbitrary 0..32 byte sequences. Proteus reads the SSID
bytes from NetworkManager via DBus and feeds them to
`String::from_utf8_lossy`, then formats and prints to the terminal:

```rust
// src/commands/session.rs:265-266
if let Some(ssid_bytes) = extract_byte_array(&settings, "802-11-wireless", "ssid") {
    session.ssid = Some(String::from_utf8_lossy(&ssid_bytes).into_owned());
}
// src/commands/session.rs:718-722
match &net.ssid {
    Some(ssid) => format!("{} \u{2192} {ssid} ({kind_label}{chip})", net.iface),
    ...
}
// src/commands/session.rs:703-705
fn print_row(label: &str, value: &str) {
    println!("{label:<ROW_LABEL_WIDTH$} {value}");
}
```

A hostile AP can name itself with terminal escape sequences:

* `\x1b[2J\x1b[H` (clear screen, home cursor) — overwrite the user's
  terminal with a fake prompt before running `proteus session`.
* `\x1b]0;<title>\x07` — change the terminal window title.
* `\x1b[<n>D` / `\x1b[<n>A` — back up cursor over previously printed text
  and overwrite. Combined with line-wrap math, an attacker can rewrite
  the output of the *next* command.
* `\x07` (BEL), `\x08` (BS), and OSC sequences are all valid 7-bit ASCII
  and pass `from_utf8_lossy` unchanged.

The same SSID flows into `state.json` if the user runs
`proteus portal mark "$ssid"`, then back out via `proteus portal list`
(`println!("{s}")` at line 151) and `proteus session` after a portal
match. JSON output is safe (serde escapes control bytes); the human
TUI is not.

This is the same class of bug as the `iwlist scan` / `nmcli dev wifi
list` terminal-injection issues that have surfaced periodically in the
Wi-Fi tooling space (cf. `wpa_supplicant` SSID handling). Severity is
low — no privilege escalation, but a credible spoofing primitive on a
hostile LAN.

**Recommendation.** Sanitize SSID strings (and NM connection IDs, which
are also user-supplied) before they hit any stdout/stderr `println`/
`print_row` call. The standard fix is to replace control bytes with a
visible escape (e.g. `\x1b` → `\\e`, control bytes → `\\xNN`) and clamp
display width. A small helper `display_safe(&str) -> Cow<str>` next to
`print_row` would cover every site.

**CVSS 3.1:** AV:A/AC:H/PR:N/UI:R/S:U/C:N/I:L/A:N → **2.9 Low**.

---

## N‑3 (LOW) — `state_lock` open path follows symlinks and uses umask permissions

**File:** `src/state_lock.rs:128-140`

```rust
let file = OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(path)
    .with_context(|| format!("opening lock file {}", path.display()))?;
```

* No `O_NOFOLLOW`: a symlink at `<state-dir>/.lock` is followed.
* No `mode(0o600)`: the lock file inherits umask. In production
  `/var/lib/proteus` is `0700 root:root` so this is invisible. With
  `--state /tmp/foo` (or via the `PROTEUS_STATE_DIR` env vector — N‑0)
  the lock file ends up at world-readable mode.

Risk is small (lock file holds no secrets, just an empty regular file),
but defense-in-depth should match `write_atomic`'s posture: same flags,
same mode, same audit story.

**Recommendation.** Apply
`.custom_flags(libc::O_NOFOLLOW).mode(0o600)` to the `OpenOptions`.

---

## T‑1 (TEST FAILURE, not a security issue) — `captured_factory_mac_persists_to_disk` is non-hermetic

**File:** `src/commands/rotate.rs:321-345`

```
test commands::rotate::tests::captured_factory_mac_persists_to_disk ... FAILED
  left:  Some("02:fc:00:00:00:01")  // host's real eth0
  right: Some("11:22:33:44:55:66")  // test fixture
```

`capture_original_mac` calls `factory::permanent_address(iface)` first and
falls back to the `hw_hint` only when that returns `None`. The test passes
`"eth0"` and `"wlan0"` as iface names; on a host that *has* an
`eth0` (this audit's container did), `factory::permanent_address` reads
the live `/sys/class/net/eth0/address` (whose `addr_assign_type` happens
to be 0/`NET_ADDR_PERM` in podman), and the live MAC wins over the test
fixture.

This is a test-hermeticity bug, not exploitable. But it surfaces a real
production concern worth recording: **`capture_original_mac` reads
host-state without sandboxing**, so any environment where `/sys/class/net`
is unusual (containers with `--cap-add NET_ADMIN`, network-namespaced
unit tests, kernel namespaces with the namespaces feature enabled) will
silently cache whatever the kernel says was the "original" MAC of the
host iface that happens to have the same name. Issue #123 was supposed
to fix exactly this case.

**Recommendation.** The test should use the existing
`crate::mac::factory::permanent_address_under(sysfs_root, iface,
ethtool)` test hook with an empty `TempRoot` so neither `phy80211` nor
`addr_assign_type==0` is found, forcing the `hw_hint` fallback. Two-line
fix: thread `permanent_address_under` through `capture_original_mac`
behind a `#[cfg(test)]` constructor, or rename the test fixture iface
to something that cannot exist on a real host (e.g. `proteus-test0`).

Production callers should also probably bail out if
`addr_assign_type==0` but the MAC came from a clearly virtual iface
(no `device/` link in sysfs). Today that case slips through.

---

## Summary

* All five Medium/High v0.1 findings except **M‑2** are now closed.
* M‑2 is unchanged and is the single biggest remaining issue
  (re-classified as N‑0 in this follow-up). The fix is straightforward.
* Three new Low findings (N‑1, N‑2, N‑3) all map to defense-in-depth
  gaps that the Medium fixes already established the pattern for.
* The test suite has one non-security failure (T‑1) tied to issue #123's
  factory-MAC capture not being sandboxed for tests.

Per `SECURITY.md`, file N‑0 privately at
<https://github.com/Kit3713/Proteus/security/advisories/new>; N‑1, N‑2,
N‑3, T‑1 can be public issues at the maintainer's discretion.
