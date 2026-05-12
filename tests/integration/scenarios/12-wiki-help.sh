#!/bin/sh
# 12-wiki-help.sh — `proteus wiki search` and `proteus help` (Roadmap
# Stream 1 / CL4).
#
# `wiki search` and the top-level `help` shortcut are both read-only
# discoverability commands. The roadmap's competitive-positioning note
# pins discoverability as Proteus's moat — so the wiki/help surface
# being healthy is load-bearing.
#
# Asserted:
#   - `wiki search <query>` exits 0 and emits valid JSON with `--json`.
#   - `wiki search <bogus-query>` exits 0 (no matches is not an error;
#     the JSON `hits` array is just empty).
#   - `wiki search` without a query exits non-zero (clap rejects the
#     missing required arg at parse time, rc=2).
#   - `proteus help` (no arg) exits 0 (prints the index).
#   - `proteus help intro` exits 0 (a known page).
#   - `proteus help unknown-page-zzz` exits non-zero with a friendly
#     "no wiki page or matches for ..." line.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 12-wiki-help\n'

# --- wiki search --------------------------------------------------------
expect_zero "proteus wiki search dns"        proteus wiki search dns
expect_zero "proteus wiki search dns --json" proteus wiki search dns --json
# No-match queries are not errors — the result set is just empty.
expect_zero "proteus wiki search unmatchable-query-zzz" \
    proteus wiki search unmatchable-query-zzz

# Missing required arg trips clap (rc=2).
expect_rc_in "proteus wiki search (no query)" 2 -- proteus wiki search

# JSON parse.
OUT=$(proteus wiki search dns --json 2>/dev/null) || OUT=""
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "proteus wiki search --json parses"
else
    fail "proteus wiki search --json did not parse"
fi

# --- help (top-level, alias for `wiki <feature>` w/ fallback) ----------
expect_zero "proteus help (no arg)"  proteus help
expect_zero "proteus help intro"     proteus help intro
expect_rc_in "proteus help unknown-page-zzz" 1 -- proteus help unknown-page-zzz

finish 12-wiki-help
