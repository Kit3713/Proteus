#!/bin/sh
# 09-component-yes-gates.sh — every component mutator rejects without
# `--yes` (Roadmap Stream 1 / CL4).
#
# Pairs with 02-stub-mutators.sh and 06-yes-gates.sh, which cover the
# top-level orchestrator and a handful of dispatch-bug-fix paths. This
# scenario fills in the rest: every `<component> apply` / `<component>
# revert` / `<component> rotate` / `<component> pin` / `<component>
# renew` action that gates on --yes must exit with one of the documented
# pre-flight refusal codes when the flag is missing.
#
# Accept 64 / 65 / 66:
#   - 64 (NOT_IMPLEMENTED) — legacy code, kept for alpha back-compat.
#   - 65 (CONFIG_ERROR / CONFIRMATION_REQUIRED) — the documented code.
#   - 66 (PERMISSION_ERROR) — if the root check fires before the
#     --yes check (some components root-gate at dispatch).

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 09-component-yes-gates\n'

# --- bluetooth (apply / revert) ------------------------------------------
expect_rc_in "proteus bluetooth apply"  64 65 66 -- proteus bluetooth apply
expect_rc_in "proteus bluetooth revert" 64 65 66 -- proteus bluetooth revert

# --- hostname (rotate / pin / revert) -----------------------------------
expect_rc_in "proteus hostname rotate" 64 65 66 -- proteus hostname rotate
expect_rc_in "proteus hostname pin example" 64 65 66 -- \
    proteus hostname pin example
expect_rc_in "proteus hostname revert" 64 65 66 -- proteus hostname revert

# --- ipv6 (apply / revert) ----------------------------------------------
expect_rc_in "proteus ipv6 apply"  64 65 66 -- proteus ipv6 apply
expect_rc_in "proteus ipv6 revert" 64 65 66 -- proteus ipv6 revert

# --- enterprise-wifi (enable / disable; connection arg is required) -----
expect_rc_in "proteus enterprise-wifi enable --connection test" 64 65 66 -- \
    proteus enterprise-wifi enable --connection test
expect_rc_in "proteus enterprise-wifi disable --connection test" 64 65 66 -- \
    proteus enterprise-wifi disable --connection test

# --- stack (apply / revert) ---------------------------------------------
expect_rc_in "proteus stack apply"  64 65 66 -- proteus stack apply
expect_rc_in "proteus stack revert" 64 65 66 -- proteus stack revert

# --- dns (apply / revert) -----------------------------------------------
expect_rc_in "proteus dns apply"  64 65 66 -- proteus dns apply
expect_rc_in "proteus dns revert" 64 65 66 -- proteus dns revert

# --- resolved (apply / revert) ------------------------------------------
expect_rc_in "proteus resolved apply"  64 65 66 -- proteus resolved apply
expect_rc_in "proteus resolved revert" 64 65 66 -- proteus resolved revert

# --- ntp (apply / revert) -----------------------------------------------
expect_rc_in "proteus ntp apply"  64 65 66 -- proteus ntp apply
expect_rc_in "proteus ntp revert" 64 65 66 -- proteus ntp revert

# --- nft (apply / revert) -----------------------------------------------
expect_rc_in "proteus nft apply"  64 65 66 -- proteus nft apply
expect_rc_in "proteus nft revert" 64 65 66 -- proteus nft revert

# --- rf (apply / revert) ------------------------------------------------
expect_rc_in "proteus rf apply"  64 65 66 -- proteus rf apply
expect_rc_in "proteus rf revert" 64 65 66 -- proteus rf revert

# --- dhcp renew (the only DHCP mutator not in 06-yes-gates.sh) ----------
expect_rc_in "proteus dhcp renew" 64 65 66 -- proteus dhcp renew

# --- rotate / rotate-if-needed / pin --------------------------------------
# These three top-level mutators were called out in CL4 as untested in
# the scenarios/ tree. The orchestrator-level rotate smoke in run.sh
# only covers the `--yes` path; this covers the no-yes refusal.
expect_rc_in "proteus rotate"             64 65 66 -- proteus rotate
expect_rc_in "proteus rotate-if-needed"   64 65 66 -- proteus rotate-if-needed
expect_rc_in "proteus pin lo"             64 65 66 -- proteus pin lo

finish 09-component-yes-gates
