#!/bin/sh
# 10-persona-cli.sh — read paths for `proteus persona` (Roadmap Stream 1
# / CL4, Milestone 2).
#
# `proteus persona ...` ships eleven subactions; only the mutating five
# (`use`, `clear`, `new`, `import`, `export`) require root, and only
# `validate` takes a path-argument that must point to a real file. The
# read paths (`list`, `show`, `current`, `random`) are public and must:
#
#   - exit 0 with valid JSON when `--json` is passed.
#   - print a known builtin persona (we assert `iphone-15` is in the
#     builtin catalogue — its absence would be a real regression).
#   - exit `CONFIG_ERROR` (65) on:
#       * `show <unknown>` (no such persona in the catalogue).
#       * `validate <missing-path>` (reading the file fails).
#
# The mutator-yes-gate-without-root path was already covered by
# 09-component-yes-gates.sh; here we verify the read surface.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 10-persona-cli\n'

# --- list / current / random read paths --------------------------------
expect_zero "proteus persona list"           proteus persona list
expect_zero "proteus persona list --json"    proteus persona list --json
expect_zero "proteus persona current"        proteus persona current
expect_zero "proteus persona current --json" proteus persona current --json
expect_zero "proteus persona random --json"  proteus persona random --json

# `iphone-15` is one of the documented built-in personas in
# data/personas/. The full list is exercised in Rust unit tests; this
# is just the "the catalogue is loaded at all" floor check.
expect_contains "persona list mentions iphone-15" "iphone-15" \
    proteus persona list

# --- show (read, but takes an id) --------------------------------------
expect_zero "proteus persona show iphone-15"         proteus persona show iphone-15
expect_zero "proteus persona show iphone-15 --json"  proteus persona show iphone-15 --json
expect_rc_in "proteus persona show unknown-zzz" 65 -- \
    proteus persona show unknown-zzz

# --- validate (takes a path) -------------------------------------------
# The path must exist; otherwise we get a CONFIG_ERROR with a friendly
# "reading <path>" message. We use a path under /tmp because that's
# writable in the container.
expect_rc_in "proteus persona validate /tmp/nope-xyz" 65 -- \
    proteus persona validate /tmp/nope-xyz

# --- JSON parse checks -------------------------------------------------
for cmd in "persona list" "persona current" "persona random" "persona show iphone-15"; do
    # shellcheck disable=SC2086
    OUT=$(proteus $cmd --json 2>/dev/null) || OUT=""
    if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
        ok "proteus $cmd --json parses"
    else
        fail "proteus $cmd --json did not parse"
    fi
done

finish 10-persona-cli
