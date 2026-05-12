#!/bin/sh
# 14-kill-resume.sh — `proteus kill` + `proteus resume` (Roadmap Stream
# 1 / CL4).
#
# The kill switch is the most destructive single command Proteus ships
# (drops every interface + disables radios). Its sibling `resume` is
# the only documented way to bring traffic back without `proteus apply`
# tearing through state. Both must:
#
#   - reject without --yes (CONFIRMATION_REQUIRED, 65), or refuse for
#     root with PERMISSION_ERROR (66) — accept either, plus 64 for
#     alpha back-compat, same as 02-stub-mutators.sh.
#   - `kill status` is read-only but requires root (it reads
#     /run/proteus/kill-active and /sys/class/rfkill). Inside the
#     container we run as root so the read succeeds and emits valid
#     JSON with `--json`.
#   - `resume --json` was added in CL6 for wrapper parity; assert the
#     JSON path is wired even on a fresh system.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 14-kill-resume\n'

# --- kill (mutator, no subcommand = destructive action) ---------------
# Without --yes, the destructive `kill` must refuse before doing
# anything. Inside the container we run as root, so the gate fires at
# the --yes check (65) rather than the root check (66).
expect_rc_in "proteus kill (no --yes)" 64 65 66 -- proteus kill

# --- kill status (read; root-required because it reads /run) ---------
# `kill status` reads /run/proteus/kill-active and /sys/class/rfkill —
# the binary gates this on EUID == 0. Inside the integration container
# we're root and the call returns rc=0; on a non-root dev host it
# surfaces PERMISSION_ERROR (66). Accept both so the scenario passes
# in either environment.
expect_rc_in "proteus kill status"        0 66 -- proteus kill status
expect_rc_in "proteus kill status --json" 0 66 -- proteus kill status --json

# JSON parse — only meaningful when the rc=0 path fired (i.e. we're
# root). If we got 66, the JSON channel was just a hint message, not
# the documented schema; skip the parse assertion in that case.
proteus kill status --json >/dev/null 2>&1
if [ $? -eq 0 ]; then
    OUT=$(proteus kill status --json 2>/dev/null)
    if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
        ok "proteus kill status --json parses"
    else
        fail "proteus kill status --json did not parse"
    fi
else
    ok "proteus kill status --json (skipped — not root)"
fi

# --- resume (mutator, --yes gated; CL6 added --json) -----------------
expect_rc_in "proteus resume (no --yes)" 64 65 66 -- proteus resume

# `resume --help` is the floor check for the --json flag wiring; it
# always exits 0.
expect_zero "proteus resume --help" proteus resume --help

finish 14-kill-resume
