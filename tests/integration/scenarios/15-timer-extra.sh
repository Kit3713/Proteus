#!/bin/sh
# 15-timer-extra.sh — `proteus timer set / reset / logs` (Roadmap
# Stream 1 / CL4).
#
# 04-timer-cli.sh covers `timer status` / `timer list` and the
# name-to-unit mapping; 06-yes-gates.sh covers `timer enable resume` /
# `timer disable resume`. The three remaining timer actions
# (`set` / `reset` / `logs`) had no coverage. This fills that gap.
#
# Asserted:
#   - `timer set <name> --interval <dur>` without --yes refuses
#     (CONFIRMATION_REQUIRED).
#   - `timer set <name>` without --interval trips clap (rc=2,
#     required-arg).
#   - `timer reset <name>` without --yes refuses.
#   - `timer logs <name>` runs against journald and exits 0 (in the
#     container journald is up).
#   - `timer logs <name> --lines 99999999` rejects with clap-range
#     (rc=2) because lines is bounded 1..=100_000.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 15-timer-extra\n'

# --- timer set (mutating; needs --interval + --yes) -------------------
# Missing --interval = clap rejects at parse (rc=2).
expect_rc_in "proteus timer set rotate (no --interval)" 2 -- \
    proteus timer set rotate

# With --interval but without --yes: confirmation gate.
expect_rc_in "proteus timer set rotate --interval 5m (no --yes)" 64 65 66 -- \
    proteus timer set rotate --interval 5m

# --- timer reset (mutating; --yes gated) -----------------------------
expect_rc_in "proteus timer reset rotate (no --yes)" 64 65 66 -- \
    proteus timer reset rotate

# --- timer logs (read; surfaces journald output) ----------------------
# In the privileged container with systemd as PID 1, journalctl is
# available; the timer unit may not have any history yet, but
# journalctl exits 0 either way (empty output is not an error).
expect_zero "proteus timer logs rotate" proteus timer logs rotate

# --lines is bounded 1..=100_000 — out-of-range trips clap (rc=2).
expect_rc_in "proteus timer logs rotate --lines 0" 2 -- \
    proteus timer logs rotate --lines 0
expect_rc_in "proteus timer logs rotate --lines 999999999" 2 -- \
    proteus timer logs rotate --lines 999999999

finish 15-timer-extra
