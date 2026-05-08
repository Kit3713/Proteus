# Security audit — 2026-05-07

> **Maintainer note (2026-05-08).** This document is preserved verbatim as the
> v0.1.0-alpha audit record. The follow-up at
> [`SECURITY-AUDIT-2026-05-07-followup.md`](./SECURITY-AUDIT-2026-05-07-followup.md)
> re-checked these findings against v0.2.7-alpha; the table below brings the
> status forward to the v0.4.2-beta release window. Original finding text is
> unchanged.

## Status table (added 2026-05-08)

| ID | Severity | Status on `main` (v0.4.2-beta prep) | Reference |
|----|----------|--------------------------------------|-----------|
| H‑1 | High | **Fixed.** `proteus portal open` no longer auto-launches `xdg-open`; `try_xdg_open` removed. | `src/commands/portal.rs:244,288` |
| H‑2 | High | **Fixed.** `write_atomic` uses `O_CREAT\|O_EXCL`, random suffix, mode 0o600, RAII cleanup, parent fsync. | PR #188 |
| M‑1 | Medium | **Fixed.** `iface`/`key` validated via `validate_iface_name`; canonicalized prefix check on writes. | issue #147 |
| M‑2 | Medium | **OPEN.** Tracked for v0.4.2-beta cycle (`PROTEUS_*_DIR` env hardening). See follow-up N‑0. |  in flight  |
| M‑3 | Medium | **Fixed.** `parse_http_url` rejects control bytes via `is_request_safe`. |  landed pre-v0.2.7  |
| L‑1 | Low | **Fixed.** Unbiased `random_index` shipped for MAC + BT alias selection. | PR #307 |
| L‑2 | Low | **Fixed.** `parse_interval` and `annotate_disable_reason` strip CR/LF/`[`/`]`; tightened further this release. | PR #297 |
| L‑3 | Low | **Partial.** `is_safe_iface` covers `iw`/`ip`; remaining call sites tracked in v0.4.2-beta cycle. |  in flight  |
| L‑4 | Low | **Fixed.** `$VISUAL`/`$EDITOR` allowlist + privilege handling landed. | PR #244, PR #309 |
| I‑1 | Info | **In progress.** SHA-256 consolidation underway. | PR #299 |
| I‑2 | Info | **Recommended.** `cargo audit` to be added to release checklist; not yet wired into CI. |  open recommendation  |

Branch: `claude/security-audit-sjnXX`. Audit performed against commit `0c61eac`
(`v0.1.0-alpha`). Build verified (`cargo build --release` clean) and the full
test suite passes (`cargo test --release` → 312/0/0). Read-only commands
(`status`, `current`, `original`, `show-defaults`, `doctor`, `--help`)
exercised on the host.

Scope of this audit: the Rust source tree under `src/` (~22 kLOC), the
installer (`install.sh`), the NetworkManager dispatcher hook
(`dist/networkmanager/dispatcher.d/01-proteus`), and the polkit policy.

This document is a **private** audit log per `SECURITY.md`. Findings should be
filed via the GitHub private security advisory form, not as public issues —
several of these are exploitable by an unauthenticated network attacker.

Findings are ranked by severity. CVSS 3.1 vectors are illustrative.

---

## H‑1 (HIGH) — Captive-portal redirect URL passed to `xdg-open` running as root

**File:** `src/commands/portal.rs:131-201`, `src/captive_portal/mod.rs:138`

**Summary.** `proteus portal open` requires root, makes an HTTP request to the
configured detector URL, and if the response includes a `Location` header,
hands that header value verbatim to `xdg-open` — still running as root.

```rust
// src/captive_portal/mod.rs:138
let target = resp.header("Location").map(|s| s.to_string());
// ...
// src/commands/portal.rs:146
let url = match outcome.redirect_target.as_deref() {
    Some(u) => u.to_string(),
    ...
};
// src/commands/portal.rs:196
ProcCommand::new("xdg-open").arg(url).status()
```

`Location` is attacker‑controlled (the captive portal is by definition
untrusted) and the URL scheme is **never validated**. `xdg-open` dispatches by
scheme to whatever desktop handler is registered, which means an attacker on
the local network can:

* Force `file:///…` opens (most browsers refuse, but some image viewers /
  PDF readers do not — and they will be invoked as **root**, with root's
  access to anything on disk).
* Force opens of any registered URI scheme: `ssh://`, `vnc://`, `tel://`,
  `mailto:`, `slack://`, `zoom://`, `discord://`, `steam://`, `ms-…://`,
  custom in-house handlers, etc. Several of those handler families have a
  history of remote-argument-injection RCEs (Zoom CVE‑2022‑28762, Steam,
  etc.). Any RCE in a registered handler becomes **root** RCE here.
* Force `data:` / `javascript:` URLs into a browser running as root.
* Pass a value beginning with `--` and exploit any `xdg-open` shim that
  forwards arguments to a downstream tool (the bundled `xdg-open` is
  generally argument-safe, but desktop replacements such as `mimeopen`,
  `kde-open`, and various distro forks have varied behavior).

There is also a softer downstream issue: `xdg-open` is widely understood to
be unsafe to invoke as root because it inherits the calling user's
`DBUS_SESSION_BUS_ADDRESS`/`XDG_RUNTIME_DIR` only when those happen to be
preserved by `sudo`/`pkexec`, leading to inconsistent and surprising
behavior.

**Reproduction.**
```sh
# attacker-controlled HTTP server on the LAN
$ python3 -m http.server 8080 &
$ printf 'HTTP/1.0 302 Found\r\nLocation: file:///etc/shadow\r\n\r\n' \
    | nc -lp 80   # whatever the configured detect_url resolves to

# victim
$ sudo proteus portal open
opened file:///etc/shadow in default browser   # <- xdg-open invoked as root
```

**CVSS 3.1:** AV:A/AC:L/PR:N/UI:R/S:C/C:H/I:H/A:L → **8.0 High** (adjacent
network, requires the victim to run `proteus portal open` while on a hostile
LAN; impact crosses scope because xdg-open hands a root-privileged process
to a desktop handler).

**Recommendations.**
1. Reject any redirect target whose scheme is not `http` or `https` before
   passing it to `xdg-open`.
2. Drop privileges before invoking `xdg-open` — find the invoking user via
   `SUDO_UID`/`SUDO_GID` (or `PKEXEC_UID`) and `setresuid`/`setresgid` to
   them, with a hard fail if the target uid is 0.
3. Consider not auto-opening at all: print the URL and let the user paste
   it into their browser (the existing fallback path on line 165 already
   does this when `xdg-open` is missing — make it the default).
4. Sanitize the redirect target so it cannot begin with `-` (defense in
   depth against argument injection in any downstream tool).

---

## H‑2 (HIGH) — `write_atomic` follows symlinks; predictable temp filename

**File:** `src/commands/mod.rs:121-134` (also `src/dns/apply.rs:68-72`,
`src/timer/mod.rs:285-294`, `src/commands/stack.rs:195-200`).

**Summary.** The shared atomic-writer creates a temp file at
`path.with_extension("tmp")` with `std::fs::File::create`, then renames it
into place. None of the call sites:

* set restrictive permissions on the temp file (mode is umask-dependent),
* pass `O_NOFOLLOW` (so a pre-existing symlink at the temp path is
  followed),
* fsync the parent directory after the rename (durability, not security),
* pass `O_EXCL` to fail if the temp already exists.

```rust
let tmp = path.with_extension("tmp");
let mut f = std::fs::File::create(&tmp)?;   // O_CREAT|O_WRONLY|O_TRUNC, follows symlinks
f.write_all(contents)?;
f.sync_all()?;
std::fs::rename(&tmp, path)?;
```

In the production layout `/etc/proteus` is `0755 root:root` and
`/var/lib/proteus` is `0700 root:root`, so a non-root attacker cannot
pre-stage symlinks there. **However:**

* Several paths Proteus writes to are **not** in those private dirs and
  are world-traversable. Examples that go through the same writer or
  similar `fs::write`-based helpers:
  - `/etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf`
    (`src/dns/apply.rs:68-72`)
  - `/etc/systemd/system/<unit>.timer.d/override.conf`
    (`src/timer/mod.rs:285-294`)
  - `/etc/sysctl.d/95-proteus.conf`,
    `/etc/sysctl.d/96-proteus-ipv6.conf`
    (`src/commands/stack.rs:195-200`)
  - `/etc/systemd/timesyncd.conf.d/10-proteus.conf`
* The `/etc/systemd/*` dropin parents are normally `0755 root:root` so a
  non-root local attacker cannot pre-create the `.tmp`. The risk surface
  is therefore modest — but a misconfigured umask in a unit/manifest, or
  a future call site under a less-protected dir, will turn this into a
  symlink-overwrite primitive.
* The `--state /tmp/foo` and `--config /tmp/foo` overrides exist on every
  command (see `src/commands/mod.rs:42-52`). If a privileged operator runs
  Proteus with a state path under a world-writable directory (e.g. a CI
  job, a shared admin host), an unprivileged user can pre-create
  `/tmp/foo.tmp` as a symlink to `/etc/sudoers` and the writer will
  follow it. The temp file path is fully predictable from the destination
  path.

Additionally, `state.json` contains the **cached original MAC addresses**
— exactly the identifier the rest of the tool exists to hide. The state
file is written with default umask (typically `0644`) by way of this same
helper. Inside `0700 /var/lib/proteus` it's invisible; outside (test
overrides, alternate layouts, docker bind mounts) it leaks.

**Recommendations.**
1. Open the temp with `OpenOptions::new().write(true).create_new(true)`
   plus a `custom_flags(libc::O_NOFOLLOW)` (or use the `nix` crate's
   `OFlag::O_NOFOLLOW | O_EXCL`).
2. `chmod` the temp to `0600` for `state.json` and `0644` for system
   drop-ins **before** the rename.
3. Randomize the temp suffix (`.tmp.<pid>.<rand>`) so concurrent runs
   don't trample and pre-staged symlinks are harder to time.
4. fsync the parent dir after rename for crash-safety (orthogonal but
   cheap).

**CVSS 3.1 (worst case, custom state path on shared host):**
AV:L/AC:H/PR:L/UI:N/S:C/C:H/I:H/A:H → **7.4 High**. In the shipped layout
the impact is far smaller (defense-in-depth issue only) but the bug is
the same.

---

## M‑1 (MEDIUM) — Path-traversal write into `/proc/sys/net/ipv6/conf/<iface>/<key>`

**File:** `src/ipv6/mod.rs:150-179`

```rust
pub fn write_sysctl(root_prefix: Option<&Path>, iface: &str, key: &str, value: &str) -> Result<()> {
    let path = sysctl_path(root_prefix, iface, key);     // base.join(iface).join(key)
    std::fs::write(&path, format!("{value}\n"))
}
```

`Path::join` does **not** normalize `..`. The `iface` and `key` arguments
are not validated against `[A-Za-z0-9_-]`. An `iface` value of
`"../../../../etc"` plus a `key` of `"shadow"` would resolve through
`/proc/sys/net/ipv6/conf/../../../../etc/shadow` and overwrite
`/etc/shadow` (running as root).

In the current call graph `iface` always comes from a NetworkManager
device list (so kernel‑validated, max 15 bytes, no `/`), and `key` is
hardcoded. So this is **not** exploitable from outside today. But:

* The function is `pub` and any future caller — the `dry_run` preview
  iterator, a planned per-iface CLI, a config-driven extension — could
  pass attacker-shaped strings.
* The defense lives entirely outside the function. A linter/grep won't
  flag it.

**Recommendations.**
1. Reject `iface` and `key` containing `/`, `..`, NUL, or non-ASCII
   characters at the entry point.
2. After joining, verify the canonicalized result starts with the
   intended `/proc/sys/net/ipv6/conf/` prefix. The fact that `/proc/sys`
   does not contain symlinks today does not make this safe forever.

**CVSS 3.1 (latent):** AV:L/AC:H/PR:H/UI:N/S:U/C:N/I:H/A:H → **5.4 Medium**.
Latent — promotes to High if a future call site forwards user input.

---

## M‑2 (MEDIUM) — `PROTEUS_*_DIR` env vars steer destructive `remove_dir_all`

**File:** `src/commands/uninstall.rs:55-112`

```rust
struct Layout { config_dir, state_dir, systemd_dir }
impl Layout { fn from_env() -> Self {
    Self {
        config_dir: env_path("PROTEUS_CONFIG_DIR", DEFAULT_CONFIG_DIR),
        state_dir:  env_path("PROTEUS_STATE_DIR",  DEFAULT_STATE_DIR),
        systemd_dir:env_path("PROTEUS_SYSTEMD_DIR",DEFAULT_SYSTEMD_DIR),
    }
}}
// ...
for dir in [&layout.config_dir, &layout.state_dir] {
    note(dir, remove_dir_opt(dir), &mut warns);   // remove_dir_all
}
```

`Layout::from_env()` runs in production, not just tests, and there is no
hint in the docs that production callers can override these. A user who
runs

```sh
sudo PROTEUS_STATE_DIR=/etc proteus uninstall --purge --yes
```

will recursively delete `/etc`. The `--purge` flag is the only consent
gate. `sudo` strips most env vars by default, but:

* `pkexec` with the bundled polkit policy may pass them depending on
  `org.freedesktop.policykit.exec.allow_gui` / sudoers configuration.
* Custom sudoers rules (`Defaults env_keep+="PROTEUS_*"`) are
  predictable for any operator who set up tests; they will bleed into
  production runs.
* Helper wrappers / Make targets routinely set env vars and shell out
  to `sudo` (`sudo -E`), which preserves them.

The tests at lines 266-291 use these env vars deliberately, but the
production code path (`run` at line 45) reads the same vars without
distinguishing test-mode from real use.

**Recommendations.**
1. Only honor these env vars in `cfg(test)` — gate `Layout::from_env`
   behind a `#[cfg(any(test, feature = "test-overrides"))]` switch and
   hardcode the production paths.
2. Or: refuse to operate on a path outside an allowlist
   (`/etc/proteus`, `/var/lib/proteus`, `/etc/systemd/system`). The
   value of these env vars for tests is replaceable by passing the
   path through the function signature.

**CVSS 3.1:** AV:L/AC:H/PR:H/UI:R/S:C/C:N/I:H/A:H → **6.0 Medium**.
Requires the operator to have an `env_keep` sudoers rule or a wrapper
that preserves the variable, but the consequence (rm -rf /etc) is total.

---

## M‑3 (MEDIUM) — CRLF injection into captive-portal HTTP request

**File:** `src/captive_portal/mod.rs:182-216`

```rust
fn parse_http_url(url: &str) -> Option<UrlParts> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') { ... };
    // host/path returned verbatim; no \r\n / NUL / control-char check
}
// ...
let req = format!(
    "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: ...\r\n\r\n",
    parts.path, parts.host        // <- no escaping
);
```

A `detect_url` value of
`"http://example.com/foo\r\nX-Smuggle: yes\r\n\r\n"` would inject
arbitrary headers (and on long-enough payloads, an entire smuggled
request) into the GET. The `Host` header is similarly injectable via
the host portion.

`detect_url` lives in `/etc/proteus/config.toml`, which is root-owned
0644, so the attacker has to be root already — making this **low
severity for the primary threat model**. It is still listed as Medium
because:

* The detector is a security-relevant feature (it tells Proteus whether
  to rotate or stay put). Any path Proteus can be tricked into making
  arbitrary HTTP requests on is a request-smuggling primitive against
  intermediate proxies.
* Future config-distribution mechanisms (Ansible, MDM, profile import)
  may reduce the trust on `config.toml`.

**Recommendations.**
1. After `parse_http_url`, reject any host or path containing
   `\r`, `\n`, NUL, or non-ASCII control bytes.
2. Percent-encode the path before formatting it into the request line.
3. Switch to a real HTTP library if the dep budget eases; rolling
   HTTP/1.0 by hand has been wrong far more often than it's been right.

---

## L‑1 (LOW) — Modulo bias in MAC and Bluetooth alias selection

**File:** `src/mac/generator.rs:25,31`, `src/bluetooth/alias.rs:43-46`

```rust
let token_idx = (rand_u8()? as usize) % opts.pool.len();
let prefix_idx = (rand_u8()? as usize) % prefixes.len();
// ...
let idx = (buf[0] as usize) % GENERIC_ALIASES.len();   // 19 entries → biased
```

Pulling a `u8` and reducing modulo a non-power-of-two set introduces a
small but measurable bias. With 19 aliases the probability mass on the
first 9 entries is ~14% higher than on the remaining 10. The
fingerprinting threat model assumes uniform selection; biased selection
is a (very weak) distinguisher.

This is privacy hygiene, not an exploitable bug. Easy fix:

```rust
fn random_index(n: usize) -> Result<usize> {
    let mut buf = [0u8; 4];
    let limit = (u32::MAX / n as u32) * n as u32;
    loop {
        getrandom::getrandom(&mut buf)?;
        let v = u32::from_le_bytes(buf);
        if v < limit { return Ok((v % n as u32) as usize); }
    }
}
```

---

## L‑2 (LOW) — Drop-in / OnCalendar injection via root-only inputs

**File:** `src/timer/mod.rs:189-203`

```rust
Interval::Calendar { expr } => {
    format!("[Timer]\nOnCalendar=\nOnCalendar={expr}\n")
}
```

`expr` is taken verbatim from `parse_interval` (`looks_like_calendar_expr`
just checks for ` `, `*`, or `:`). A value containing newlines and a
fake `[Unit]` section header would inject `OnFailure=evil.service` or a
new timer trigger. Mitigated by the `require_root` gate on
`proteus timer set` — root is already trusted to write systemd units.

**Recommendation.** Reject `\n`, `\r`, `[`, `]` in `expr`. Cheap and
makes the attack surface honest about its inputs.

A similar pattern exists in `annotate_disable_reason` in
`src/commands/config_cmd.rs:322-336`, where the `reason` string is
formatted into a comment without escaping `\n`. Same root-required
mitigation; same recommendation (strip newlines).

---

## L‑3 (LOW) — `iw` / `ip` invocations don't pass `--` before user-influenced positional args

**Files:** `src/rf/mod.rs:104,118,136,159`, `src/kill_switch/mod.rs:131-138`

```rust
Command::new("iw").args(["dev", iface, "info"]) ...
Command::new("ip").args(&["link", "set", iface, "down"]) ...
```

If `iface` ever begins with `-`, `iw` or `ip` will parse it as a flag
(`iw dev -h info` shows help; `ip link set -all down` would walk past
the iface position). Today `iface` always comes from kernel sources
(`/sys/class/net`, NM device list) which kernel-validates. But `iw` is
actually invoked from `set_tx_power_mbm` against a name that traces back
to NM's `Interface` property of the wireless device — kernel-trusted but
with no defensive validation in our code.

**Recommendation.** Insert a `--` separator before the iface position on
every `iw` / `ip` invocation, and reject iface names beginning with `-`
at the call site as defense-in-depth.

---

## L‑4 (LOW) — `proteus config edit` runs `$VISUAL`/`$EDITOR` as root

**File:** `src/commands/config_cmd.rs:114-147`

```rust
let editor = std::env::var_os("VISUAL")
    .or_else(|| std::env::var_os("EDITOR"))
    .unwrap_or_else(|| OsString::from(DEFAULT_EDITOR));
let status = Command::new(&editor).arg(&path).status() ... ;
```

Standard behavior for tools in this category, but worth flagging:
running `$EDITOR` as root means any plugin / autoload / `.vimrc` /
`.emacs` of the invoking user can run as root if `sudo` preserves
`HOME` (it does not by default; `sudo -H` is the safe form, and
`pkexec` strips it). With user-installed Vim plugins this becomes an
arbitrary code execution as root from a malicious dotfile.

**Recommendation.** Either drop privileges before exec'ing the editor
(write the file via a small setuid-style helper, then unprivileged
edit, then reload), or restrict `$EDITOR` to a hardcoded allowlist
(`vi`, `vim`, `nano`, `emacs`, `ed`). At minimum, document that
`sudo proteus config edit` should only be run with a `HOME` you trust.

---

## I‑1 (INFO) — Hand-rolled SHA-256

**File:** `src/dns/apply.rs:81-175`, `src/stack/sha256.rs`, `src/diff/sha256.rs`,
`src/ipv6/mod.rs:181-…`

The codebase ships **three** copies of a hand-rolled SHA-256 to avoid
adding `sha2` as a dependency. The implementation looks faithful to FIPS
180-4 and has known-vector tests for the empty string and `"abc"`. No
defect found, but:

* Three copies risk drift; consolidate into one `crate::hash` module.
* The hash is used as a tamper-evidence marker on managed drop-ins.
  Anyone with write access to the drop-in can recompute the marker, so
  the marker is not a security property — please document this in
  `wiki/dns.md` so operators don't over-trust it.

No CVSS — informational.

---

## I‑2 (INFO) — `cargo audit` not run

The audit machine did not have `cargo-audit` available; running it
against `Cargo.lock` is recommended before each release. The dep set is
small (`zbus`, `clap`, `tokio`, `toml`, `toml_edit`, `serde`,
`tracing`, `getrandom`) and pinned via `Cargo.lock`, but `zbus`
historically has CVE traffic and should be checked.

---

## Items deliberately **not** flagged

* Wiki path traversal — `src/wiki.rs:34-37`. Pages live in an
  `include_dir!` blob that has no real filesystem; `WIKI.get_file()`
  cannot escape it.
* MAC validation in `src/hostname/mod.rs:78-123` is RFC-strict; no
  injection possible.
* DBus calls go through `zbus`'s typed proxies (`#[proxy(...)]`),
  no string-formatted DBus messages.
* The NM dispatcher hook (`dist/networkmanager/dispatcher.d/01-proteus`)
  uses `set -u`, quotes every variable expansion, and only calls
  `proteus rotate --iface "$iface"` (a fixed argv shape — clap parses
  `--iface`, no shell interpolation).
* `validate_hostname` (`src/hostname/mod.rs:78`) correctly rejects
  non-ASCII, leading/trailing hyphens, and length overflow.
* Unsafe usage is limited to test-only env-mutation in
  `src/commands/uninstall.rs:267-286`.

---

## Reproduction artifacts

* `cargo build --release` — clean.
* `cargo test --release` — `312 passed; 0 failed; 0 ignored`.
* `proteus --version` → `proteus 0.1.0`.
* `proteus doctor` exercised; all read-only commands return cleanly.

## Disclosure

Per `SECURITY.md`, do **not** open a public issue for any finding
above H‑1, H‑2, M‑1, M‑2, or M‑3. File a private GitHub Security
Advisory at
<https://github.com/Kit3713/Proteus/security/advisories/new> and
attach this document.
