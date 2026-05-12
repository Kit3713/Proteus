#!/bin/sh
# 07-session-diff-dryrun.sh — smoke tests for `session`, `diff`,
# `dry-run` (Roadmap Stream 1 / CL4).
#
# These three read-only siblings were previously absent from the
# scenarios/ tree even though they're on every `--help` listing and
# `proteus help` page. CL4 calls them out as part of the "24 untested
# subcommands" gap.
#
# All three commands must:
#   - exit 0 on `--help` (the floor smoke).
#   - exit 0 on a no-arg invocation against a fresh system (no state
#     file, no managed interfaces — that's the in-container default).
#   - emit valid JSON when `--json` is passed (where supported).
#
# `dry-run` takes a trailing positional argv vector; we test it with a
# real mutator name (`apply`, `rotate`) and an obvious typo. The typo
# path is the documented "unrecognized subcommand" branch — the command
# itself succeeds (it's the dry-run *preview* that's a no-op), so the
# overall exit stays 0.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 07-session-diff-dryrun\n'

# --- session (read-only, ships --json) -----------------------------------
expect_zero "proteus session --help"   proteus session --help
expect_zero "proteus session"          proteus session
expect_zero "proteus session --json"   proteus session --json

# JSON parse — same pattern as 05-doctor.sh.
OUT=$(proteus session --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "session --json parses"
else
    fail "session --json did not parse"
fi

# --- diff (read-only, ships --json) --------------------------------------
expect_zero "proteus diff --help"      proteus diff --help
expect_zero "proteus diff"             proteus diff
expect_zero "proteus diff --json"      proteus diff --json

OUT=$(proteus diff --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "diff --json parses"
else
    fail "diff --json did not parse"
fi

# --- dry-run (positional trailing argv) ----------------------------------
expect_zero "proteus dry-run --help"           proteus dry-run --help
expect_zero "proteus dry-run apply"            proteus dry-run apply
expect_zero "proteus dry-run rotate"           proteus dry-run rotate
# Unknown subcommand under dry-run prints the documented
# "not implemented for ...: unrecognized subcommand" diagnostic and
# exits NOT_IMPLEMENTED (64). Keep this gentle — only the exit code,
# not the message wording, is the contract.
expect_rc_in "proteus dry-run unknown-cmd-xyz" 64 -- \
    proteus dry-run unknown-cmd-xyz

finish 07-session-diff-dryrun
