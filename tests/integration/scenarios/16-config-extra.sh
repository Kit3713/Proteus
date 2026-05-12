#!/bin/sh
# 16-config-extra.sh — `proteus config set-profile` (Roadmap
# Stream 1 / CL4).
#
# 03-config-cli.sh covers the read paths (`show`, `keys`, `validate`,
# `get`); 02-stub-mutators.sh covers `set`, `enable`, `disable`,
# `reset`. The remaining config mutator (`set-profile`) had no
# dedicated coverage.
#
# Asserted:
#   - `config set-profile <name>` without --yes refuses
#     (CONFIRMATION_REQUIRED).
#   - `config set-profile invalid-name` exits CONFIG_ERROR (65) with a
#     friendly "unknown profile" line.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 16-config-extra\n'

# --- config set-profile (mutating, --yes gated) ----------------------
# A valid profile name without --yes -> refuse.
expect_rc_in "proteus config set-profile high (no --yes)" 64 65 66 -- \
    proteus config set-profile high

# An invalid profile name -> CONFIG_ERROR, before the --yes check.
expect_rc_in "proteus config set-profile bogus-name" 65 -- \
    proteus config set-profile bogus-name

# Confirm the unknown-profile error mentions the valid list, so a
# wrapper can surface it to the user without spelunking. The message
# lands on stderr; we redirect to stdout for the grep.
if proteus config set-profile bogus-name 2>&1 | grep -qF "valid:"; then
    ok "set-profile lists valid profiles"
else
    fail "set-profile error did not mention 'valid:' on stderr"
fi

finish 16-config-extra
