#!/bin/sh
# 18-events.sh — `proteus events run` (Roadmap Stream 1 / CL4,
# Milestone 4c).
#
# `events run` is the long-lived event daemon. The systemd unit
# (`proteus-events.service`) is the production entry point; the shell
# path is for development and smoke tests.
#
# The contract:
#   - `--max-triggers` and `--once-after-secs` are clap-bounded
#     (0..=10_000_000 and 0..=86_400 respectively). Values outside
#     the range fail at parse time with clap's default rc=2.
#   - Without `--force`, the command refuses when `[events] enabled =
#     false` in config — the container ships defaults, so this is
#     the documented refusal path. Exit code is CONFIG_ERROR (65).
#
# We don't actually start the loop in this smoke. The legitimate
# `--max-triggers N --once-after-secs N` shape is exercised by the
# Rust integration tests (tests/events_*.rs).

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 18-events\n'

# --- --help (floor smoke) ---------------------------------------------
expect_zero "proteus events --help"     proteus events --help
expect_zero "proteus events run --help" proteus events run --help

# --- refusal when [events] enabled = false (default) ------------------
# The default config ships `[events] enabled = false`. `events run`
# without `--force` must surface the documented "pass --force" hint
# and exit CONFIG_ERROR (65).
expect_rc_in "proteus events run (disabled, no --force)" 65 -- \
    proteus events run --once-after-secs 1

# --- clap-range guards (N12.12) ---------------------------------------
# `--max-triggers` is bounded 0..=10_000_000; anything larger trips
# clap at parse time with rc=2.
expect_rc_in "proteus events run --max-triggers 99999999999" 2 -- \
    proteus events run --max-triggers 99999999999

# `--once-after-secs` is bounded 0..=86_400.
expect_rc_in "proteus events run --once-after-secs 99999999999" 2 -- \
    proteus events run --once-after-secs 99999999999

finish 18-events
