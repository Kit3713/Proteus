#!/bin/sh
# 11-ssid-cli.sh — read paths + --yes gates for `proteus ssid` (Roadmap
# Stream 1 / CL4, Milestone 3).
#
# `proteus ssid` has four subactions:
#   - `list` / `show` are read-only and work for any user.
#   - `set` / `clear` mutate config under /etc/proteus/ and require
#     root + --yes.
#
# `show` accepts any SSID string — there's no enumeration step that
# could reject "unknown SSIDs", because per-SSID config is documented
# as "absent = fall through to global", so the command always renders a
# resolved policy. The "set/clear without --yes" tests overlap with the
# component-yes-gates scenario; we replicate them here so this scenario
# is self-contained for the SSID surface.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 11-ssid-cli\n'

# --- list (read-only) ----------------------------------------------------
expect_zero "proteus ssid list"        proteus ssid list
expect_zero "proteus ssid list --json" proteus ssid list --json

# --- show (read-only, takes an SSID) ------------------------------------
# Any SSID works — absent entries fall through to global config.
expect_zero "proteus ssid show home-wifi"        proteus ssid show home-wifi
expect_zero "proteus ssid show home-wifi --json" proteus ssid show home-wifi --json

# --- set / clear without --yes ------------------------------------------
# Mutators must refuse without --yes. Inside the container we run as
# root, so the root check (66) doesn't usually fire; we accept 64/65/66
# to cover all dispatch orderings.
expect_rc_in "proteus ssid set home persona iphone-15" 64 65 66 -- \
    proteus ssid set home persona iphone-15
expect_rc_in "proteus ssid clear home" 64 65 66 -- proteus ssid clear home

# --- JSON parse checks --------------------------------------------------
OUT=$(proteus ssid list --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "proteus ssid list --json parses"
else
    fail "proteus ssid list --json did not parse"
fi

OUT=$(proteus ssid show home-wifi --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "proteus ssid show --json parses"
else
    fail "proteus ssid show --json did not parse"
fi

finish 11-ssid-cli
