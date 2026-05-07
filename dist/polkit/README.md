# Proteus PolicyKit policy

A PolicyKit (`polkit`) action policy that lets a future GUI wrapper elevate
mutating `proteus` commands through `pkexec` instead of `sudo`. The user gets
the desktop password prompt their session uses for everything else, not a
terminal `sudo` dialog.

## What it provides

`com.kit3713.proteus.policy` declares two actions:

- `com.kit3713.proteus.rotate` — for `proteus rotate`, `proteus pin`,
  `proteus unpin`. The "I want a fresh MAC right now" path.
- `com.kit3713.proteus.apply` — for `proteus apply`, `proteus revert`,
  `proteus reset`, `proteus uninstall`. The "modify managed system
  configuration" path.

Both actions default to `auth_admin_keep` for active sessions: prompt for an
admin password, remember it for ~5 minutes. Inactive sessions are denied.
That matches the convention for desktop tooling that mutates system state.

## Install

    sudo cp dist/polkit/com.kit3713.proteus.policy \
        /usr/share/polkit-1/actions/com.kit3713.proteus.policy

`install.sh` does this automatically when `/usr/share/polkit-1/actions/`
exists. polkit picks the file up on the next action lookup; no daemon
restart is required.

## Why a GUI would use this

A GUI wrapper that needs to run `proteus rotate` (or any other mutating
command) has two reasonable options for elevation:

1. Spawn a terminal and run `sudo proteus rotate`. The user sees a TTY
   password prompt — jarring inside a desktop app.
2. Run `pkexec proteus rotate`. PolicyKit checks the action policy,
   raises the same desktop password dialog the rest of the session uses,
   and runs the command with elevated privileges if the user authenticates.

This file makes option 2 work cleanly: the dialog shows the
`<message>` text from the action ("Authentication is required to rotate
network identifiers."), and the `auth_admin_keep` default avoids
re-prompting for back-to-back operations.

## Honest caveat

The Proteus binary itself does **not** enforce per-subcommand policy. It
just refuses to run mutating commands without root (exit code `66`,
`PERMISSION_ERROR`). The policy file above is a hint to `pkexec` and
desktop tooling — it tells PolicyKit which dialog text and defaults to
use when something tries to elevate `proteus` via PolicyKit. Anyone with
sudo can still run `sudo proteus rotate` directly, bypassing polkit
entirely. The policy is a usability layer for GUI wrappers, not a
defence-in-depth boundary.

## Uninstall

    sudo rm -f /usr/share/polkit-1/actions/com.kit3713.proteus.policy
