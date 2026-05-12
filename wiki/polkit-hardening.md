Operator notes for hardening the PolicyKit (`polkit`) action policy that Proteus ships at `dist/polkit/com.kit3713.proteus.policy`. The defaults are a deliberate UX-only baseline; if your threat model wants strict group-membership enforcement at the polkit layer, this page documents the supported recipe and the runtime check you can use to verify it landed.

For the broader story of why Proteus does not consult polkit from inside the binary, read `proteus wiki internals` (the "Wrapping for a GUI" section) and `proteus wiki threat-model`. For the bundled policy file itself, read `dist/polkit/README.md` in the source tree.

## What ships in the box

The shipped XML policy at `dist/polkit/com.kit3713.proteus.policy` declares two action ids and pins their authorisation defaults:

- `com.kit3713.proteus.rotate` — used by `proteus rotate`, `proteus pin`, `proteus unpin`.
- `com.kit3713.proteus.apply` — used by `proteus apply`, `proteus revert`, `proteus reset`, `proteus uninstall`.

Both actions carry the same `<defaults>` block:

```xml
<defaults>
  <allow_inactive>no</allow_inactive>
  <allow_active>auth_admin</allow_active>
</defaults>
```

The shape, in plain terms:

- **`auth_admin`** — every privileged invocation requires a fresh admin password prompt. Issue #133 deliberately chose the one-shot variant (not `auth_admin_keep`) so a user who runs `proteus apply` and walks away does not leave a ~5 minute window during which any local process can re-invoke a mutator without re-authenticating. The `polkit_mutating_actions_do_not_cache_auth` test in `src/lib.rs` pins this choice.
- **`allow_inactive>no`** — inactive sessions (an orphaned ssh session, a detached tmux on a logged-out console) cannot answer the prompt at all.
- **`<allow_any>` omitted** — non-{active,inactive} contexts (cron, system services) must use their own root-only execution path rather than routing through polkit.

What the shipped policy does *not* enforce: group membership. Anyone who can answer an `auth_admin` prompt (any admin on the system) can complete the dialog. The XML policy format does not accept a `<unix-group>` selector under `<defaults>`. Group restriction has to be layered on with a JavaScript rule under `/etc/polkit-1/rules.d/`, which this page documents below.

The "honest caveat" from `dist/polkit/README.md` still applies: the Proteus binary itself does not consult polkit. It only refuses mutating commands without root (exit code `66`, `PERMISSION_ERROR`). Anyone with `sudo` can run `sudo proteus rotate` directly, bypassing polkit entirely. The policy file is a usability layer for GUI wrappers, not a defence-in-depth boundary. See `proteus wiki internals` for the full discussion.

## Optional: restrict to `wheel` / `sudo` via a polkit JS rule

If your operator policy is "only members of `wheel` (or `sudo` on Debian-family distros) may answer a Proteus polkit prompt", drop a JavaScript rule into `/etc/polkit-1/rules.d/`. polkit evaluates `.rules` files in lexical order; the `49-` prefix puts this file ahead of the default `50-default.rules` so the rule wins.

Save the snippet below as `/etc/polkit-1/rules.d/49-proteus.rules` (root-owned, mode `0644`):

```js
// /etc/polkit-1/rules.d/49-proteus.rules
//
// Restrict Proteus mutating polkit actions to members of `wheel` and `sudo`.
// Non-members get NO (the dialog never appears); members fall through to the
// XML policy's auth_admin default, so they still get a one-shot admin prompt.
//
// Coordinate with src/lib.rs::polkit_mutating_actions_do_not_cache_auth
// (issue #133): we keep the one-shot auth_admin and do not switch to
// auth_admin_keep. Cache-then-walk-away is still the larger risk.
polkit.addRule(function(action, subject) {
    if (action.id.indexOf("com.kit3713.proteus.") !== 0) {
        return;
    }
    if (subject.isInGroup("wheel") || subject.isInGroup("sudo")) {
        return;  // fall through to the XML policy default (auth_admin)
    }
    return polkit.Result.NO;
});
```

What the rule does:

- Returns early for any action id that is not in the `com.kit3713.proteus.*` namespace, so unrelated actions are not affected.
- For Proteus actions, members of `wheel` *or* `sudo` get an implicit "fall through" return (the XML policy's `auth_admin` default still applies — they still get the prompt).
- Everyone else gets `polkit.Result.NO`: the dialog does not appear; the action is denied silently.

What the rule does *not* change:

- It does not weaken the one-shot `auth_admin` choice. Returning `polkit.Result.NO` is strictly more restrictive than the default; returning `undefined` (the implicit fall-through above) leaves `auth_admin` intact. Do **not** replace the fall-through with `polkit.Result.AUTH_ADMIN_KEEP` — that would conflict with the `polkit_mutating_actions_do_not_cache_auth` test in `src/lib.rs` (issue #133) which exists specifically to prevent the cached variant from being shipped.
- It does not change what happens when someone runs `sudo proteus rotate` from a terminal. polkit is not consulted on the sudo path, so group restriction at the polkit layer cannot bound sudo callers. If your goal is to limit who can run mutating Proteus commands at *all*, you also need sudoers rules (or to restrict membership in `wheel` / `sudo` itself).
- It does not change the `allow_inactive>no` invariant; inactive sessions remain denied regardless of group.

### Variants

If your distro uses neither `wheel` nor `sudo` (Arch's default install is `wheel`; Debian's is `sudo`; some site policies use a dedicated `admin` group), edit the `isInGroup(...)` calls accordingly. The rule supports any number of group checks chained with `||`.

If you want a *log* of every denied invocation (for audit), add `polkit.log(...)` before the `return polkit.Result.NO`:

```js
    polkit.log("denied Proteus action " + action.id +
               " for user " + subject.user +
               " (not in wheel/sudo)");
    return polkit.Result.NO;
```

`polkit.log()` writes to the polkit daemon's stderr, which lands in journald under `unit=polkit.service`. Filter with `journalctl -u polkit -t polkitd`.

### Verify the rule is loaded

polkit reloads `.rules` files automatically on edit; no daemon restart needed. To confirm a freshly-written file parsed without errors:

```sh
sudo journalctl -u polkit -n 50 --no-pager
```

A parse error logs as `Failed to load rules file ...`. A clean load is silent. If you don't see your file mentioned, polkit found no problems with it.

## Runtime check operators can use today

`proteus doctor` does not yet ship a check for the polkit policy shape — Stream 9 / S7 / B15 deferred it pending the maintainer decision on group enforcement (see `docs/ROADMAP.md` Stream 9). Until that lands, operators can run the check by hand using the standard polkit client, `pkcheck`:

```sh
# Does the current shell (PID $$) currently have authorisation to
# perform the Proteus rotate action? Returns 0 on yes, 1 on no,
# 2 on "would need authentication interactively", 3 on error.
pkcheck --action-id com.kit3713.proteus.rotate --process $$ --allow-user-interaction

# Same for the apply action.
pkcheck --action-id com.kit3713.proteus.apply --process $$ --allow-user-interaction
```

Exit codes are documented in `pkcheck(1)`; the short version:

- `0` — already authorised. Action would proceed without a prompt.
- `1` — not authorised. With `--allow-user-interaction`, this means the user declined the prompt or the rule above returned `polkit.Result.NO`.
- `2` — authorisation would require user interaction. Without `--allow-user-interaction`, this is the "you'd be prompted" answer.
- `3` — error (action id unknown, polkit unreachable, etc.).

To confirm the JS rule from the previous section is in effect, run `pkcheck` from a shell session owned by a user who is *not* in `wheel` or `sudo`. It should return exit code `1` immediately, with no dialog. Run again from a session owned by a `wheel` member; you should get a normal admin prompt and, on success, exit code `0`.

Two operator-side gotchas:

- `--process $$` ties the check to the shell's PID. Some polkit versions also require `--process-uid` or `--process-start-time`; `man pkcheck` for your installed version. If the check rejects with "no caller", try `pkcheck --action-id com.kit3713.proteus.rotate --process "$$,$(awk '{print $22}' /proc/$$/stat),$(id -u)"`.
- `--allow-user-interaction` triggers the dialog if the answer is "would prompt". Omit it for a strictly non-interactive probe — exit code `2` then tells you the action would prompt, without actually prompting.

A doctor-level check (planned for B15) will wrap this same `pkcheck` call and surface the result as a `polkit::*` line in `proteus doctor` output, including a `warn` when the shipped policy is detected but the optional group restriction is absent. Until then, the manual recipe above is the supported runtime check.

## Composition with the rest of the auth stack

The polkit rule above only governs the `pkexec` / desktop-elevation path. Three other paths bypass it; address them at the right layer:

- **`sudo proteus apply`**. polkit is not consulted. Restrict at the sudoers layer. A minimal `/etc/sudoers.d/proteus` allowing only `wheel` to run proteus mutators:

  ```text
  %wheel ALL=(root) /usr/bin/proteus
  ```

  Or, if you want sudoers to *deny* non-`wheel` users running proteus while leaving the general sudo policy alone, see `man sudoers` on the `!` operator.

- **Direct execution as root**. A process already running as root needs no authorisation from polkit or sudo. Audit who can become root.

- **The systemd units that ship with Proteus (`proteus-rotate.service`, `proteus-check.service`)**. They run as root via systemd's standard service mechanism. polkit does not gate them. If you want to restrict timer firing, edit the systemd timer (`systemctl edit proteus-rotate.timer`) or disable the unit (`systemctl disable --now proteus-rotate.timer`).

The polkit layer hardens one path — desktop-elevation via `pkexec` and any GUI wrapper that uses the same flow. It does not, and cannot, harden the others. Cross-ref `proteus wiki security-checklist` for the operational view of what to verify.

## Why not just ship the JS rule

Two reasons the rule lives here as documentation rather than in `dist/polkit/`:

- **Group naming varies across distros.** `wheel` is Fedora / RHEL / Arch; `sudo` is Debian / Ubuntu; some site policies use `admin` or a custom group. A one-size rule baked into the package would be wrong for at least half the install base. Documenting the recipe lets each operator choose.
- **The shipped XML policy is intentionally a UX hint, not an enforcement gate** (see the "honest caveat" in `dist/polkit/README.md` and `proteus wiki internals`). Bundling a JS rule that *looks* like a defence-in-depth boundary would create the wrong mental model. The right framing: the policy file is for `pkexec` dialogue text; the optional JS rule is for sites that want to enforce group membership; the binary itself only checks for `EUID == 0`.

Stream 9 / S7 / B15 leaves the maintainer decision open on whether to package this rule by default once the conflict with the `polkit_mutating_actions_do_not_cache_auth` pin (issue #133) is fully understood. The decision and any policy-file changes will land via the ROADMAP, not via the wiki.

## Cross-refs

- `proteus wiki internals` — "Wrapping for a GUI" section: why the polkit policy is a UX hint, not an authorisation gate.
- `proteus wiki threat-model` — the broader stance on hardening invariants Proteus refuses to weaken.
- `proteus wiki security-checklist` — operational hygiene routines (daily, weekly, monthly, pre-trip, post-trip).
- `proteus wiki distro-support` — what each `dist/` directory ships; the polkit policy entry.
- `proteus wiki doctor` — current per-check meaning of every `doctor` line (does not yet cover polkit; B15 deferred).
- `proteus wiki cli` — full command reference, exit codes including `66` `PERMISSION_ERROR` for mutator-without-root.

External:

- `polkit(8)`, `pkcheck(1)`, `pkexec(1)` — canonical reference for the polkit client tools.
- `polkit-rules(5)` — `.rules` file format and the `polkit.Result` constants.
- `dist/polkit/README.md` and `dist/polkit/com.kit3713.proteus.policy` in the source tree — the shipped policy file and its inline rationale.
