#!/bin/sh
# 01-read-only.sh — every read-only command exits 0.
#
# These commands must never crash, even on a freshly-booted system with
# no config, no state, no managed interfaces, no Wi-Fi, etc. They're the
# Proteus equivalent of `--help` smoke: if these break, everything broke.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 01-read-only\n'

expect_zero "proteus doctor"          proteus doctor
expect_zero "proteus doctor --json"   proteus doctor --json
expect_zero "proteus doctor --quick"  proteus doctor --quick
expect_zero "proteus status"          proteus status
expect_zero "proteus status --json"   proteus status --json
expect_zero "proteus current"         proteus current
expect_zero "proteus original"        proteus original
expect_zero "proteus show-config"     proteus show-config
expect_zero "proteus show-defaults"   proteus show-defaults
expect_zero "proteus wiki"            proteus wiki
expect_zero "proteus wiki intro"      proteus wiki intro

finish 01-read-only
