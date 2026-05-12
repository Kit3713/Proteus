#!/bin/sh
# 16-config-extra.sh — `proteus config edit / set-profile` (Roadmap
# Stream 1 / CL4).
#
# 03-config-cli.sh covers the read paths (`show`, `keys`, `validate`,
# `get`); 02-stub-mutators.sh covers `set`, `enable`, `disable`,
# `reset`. The two remaining config actions (`edit`, `set-profile`)
# had no coverage.
#
# Asserted:
#   - `config edit` requires root; we run as root in the container
#     but $EDITOR is unset, so the helper falls through to the
#     documented "no editor configured" branch and exits with
#     CONFIG_ERROR (65). We accept either 0 (rare: a usable editor
#     happens to be on PATH) or 65 (the documented refusal).
#   - `config set-profile <name>` without --yes refuses
#     (CONFIRMATION_REQUIRED).
#   - `config set-profile invalid-name` exits CONFIG_ERROR (65) with a
#     friendly "unknown profile" line.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 16-config-extra\n'

# --- config edit (root-gated, opens $EDITOR on config.toml) ------------
# `config edit` requires root + a usable $EDITOR. Inside the
# integration container we're root but no $EDITOR is configured, so
# the helper falls through to CONFIG_ERROR (65). On a non-root dev
# host the root check fires first (66). Either is fine; the contract
# we care about is "does not launch an interactive editor in CI" —
# which both 65 and 66 satisfy.
expect_rc_in "proteus config edit" 0 65 66 -- proteus config edit

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
