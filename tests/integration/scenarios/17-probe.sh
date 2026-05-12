#!/bin/sh
# 17-probe.sh — `proteus probe` (Roadmap Stream 1 / CL4).
#
# `probe` runs a manual probe round against the configured endpoints
# (the same quorum-N-of-total endpoint vector that powers the captive
# portal classifier). It's read-only, doesn't require root, and ships
# a `--json` flag.
#
# The container has no upstream internet by default, so the probe
# round will likely time out / fail on most endpoints. That's fine —
# the contract is:
#   - the command itself exits 0 even when probes fail (it's a
#     diagnostic, not a "is the internet up" gate).
#   - `--json` emits a valid JSON document with the documented
#     top-level fields.
#
# We accept both 0 (probes succeeded) and 1 (probes failed) for the
# non-help paths so the scenario works in air-gapped CI.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 17-probe\n'

# --help always succeeds.
expect_zero "proteus probe --help" proteus probe --help

# Plain probe — may classify as clear/portal/unknown depending on
# whether the container has internet, but should never crash.
expect_rc_in "proteus probe"        0 1 -- proteus probe
expect_rc_in "proteus probe --json" 0 1 -- proteus probe --json
expect_rc_in "proteus probe --quick" 0 1 -- proteus probe --quick

# JSON parse — even on failure paths, --json must emit something
# parseable so wrappers can surface the error.
OUT=$(proteus probe --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "proteus probe --json parses"
else
    fail "proteus probe --json did not parse"
fi

# The documented JSON schema includes `schema_version` and
# `classification` at the top level (see commands::probe::run).
for field in '"schema_version"' '"classification"'; do
    if printf '%s' "$OUT" | grep -qF -- "$field"; then
        ok "probe --json contains $field"
    else
        fail "probe --json missing top-level field $field"
    fi
done

finish 17-probe
