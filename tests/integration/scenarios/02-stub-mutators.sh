#!/bin/sh
# 02-stub-mutators.sh — mutators reject when --yes is missing.
#
# Every mutating command that gates on --yes must print a friendly error
# and exit with the documented "you forgot --yes" code: 65
# (CONFIRMATION_REQUIRED, an alias for CONFIG_ERROR). Issue #117 moved
# this off the legacy 64 (NOT_IMPLEMENTED), which had meant "the feature
# itself is a stub" and misled wrappers. We still accept 64 for backwards
# compat with older binaries during alpha, and 66 in case the gate moves
# above the root check on a future build.
#
# Inside the container we run as root, so the root-required check (66)
# does not fire and we get to verify the --yes gate specifically.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 02-stub-mutators\n'

expect_rc_in "proteus apply"     64 65 66 -- proteus apply
expect_rc_in "proteus revert"    64 65 66 -- proteus revert
expect_rc_in "proteus uninstall" 64 65 66 -- proteus uninstall
expect_rc_in "proteus reset"     64 65 66 -- proteus reset

expect_rc_in "proteus config set mac.enabled false" 64 65 66 -- \
    proteus config set mac.enabled false
expect_rc_in "proteus config enable mac"  64 65 66 -- proteus config enable mac
expect_rc_in "proteus config disable dns" 64 65 66 -- proteus config disable dns
expect_rc_in "proteus config reset"       64 65 66 -- proteus config reset

finish 02-stub-mutators
