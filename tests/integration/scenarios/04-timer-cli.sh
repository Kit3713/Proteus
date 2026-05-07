#!/bin/sh
# 04-timer-cli.sh — timer read commands work + mapping is correct.
#
# `proteus timer list` lists four entries (rotate, check, resume, boot).
# `proteus timer status` queries systemd; in the privileged container
# with systemd as PID 1, this should not crash even when the units
# aren't installed.
#
# We also assert the timer name-to-unit mapping — this catches the
# regression where a CLI rename breaks the dispatcher hook or install
# script (both of which key on the systemd unit names).

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 04-timer-cli\n'

expect_zero "proteus timer status"         proteus timer status
expect_zero "proteus timer status --json"  proteus timer status --json
expect_zero "proteus timer list"           proteus timer list
expect_zero "proteus timer list --json"    proteus timer list --json

# Mapping checks against the stable contract from src/timer/mod.rs::TIMERS.
expect_contains "rotate -> proteus-rotate.timer" \
    "proteus-rotate.timer" proteus timer list --json
expect_contains "check -> proteus-check.timer" \
    "proteus-check.timer"  proteus timer list --json
expect_contains "resume -> proteus-resume.timer" \
    "proteus-resume.timer" proteus timer list --json
expect_contains "boot -> proteus-boot.service" \
    "proteus-boot.service" proteus timer list --json

finish 04-timer-cli
