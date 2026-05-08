#!/bin/sh
# 06-yes-gates.sh — every mutator without --yes exits with the
# CONFIRMATION_REQUIRED sentinel (Roadmap Stream 1, GH#348/#375/#391/#349).
#
# Pairs with 02-stub-mutators.sh which already covers `apply`, `revert`,
# `uninstall`, `reset`, and the `config` subcommands. This scenario fills
# in the rest of the surface — the four mutators that were silently
# dropping `--yes` (`dhcp apply/revert`, `portal mark/unmark/open`),
# the new gate on `unpin` (N12.1), and the watch-loop CPU-burn fix
# (CL1 / GH#349) which exits CONFIG_ERROR instead of looping forever.
#
# As with 02, we accept 64/65/66 to cover both the require_yes branch
# (65) and the rare path where the root check trips first (66).

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 06-yes-gates\n'

# DHCP apply / revert (GH#348, GH#375, M1, N12.2) — `--yes` was being
# dropped at dispatch via the rest pattern. Now must exit with the gate.
expect_rc_in "proteus dhcp apply"  64 65 66 -- proteus dhcp apply
expect_rc_in "proteus dhcp revert" 64 65 66 -- proteus dhcp revert

# Portal mark/unmark/open (GH#348, N12.3) — same dispatch bug as DHCP.
expect_rc_in "proteus portal mark example-ssid"   64 65 66 -- \
    proteus portal mark example-ssid
expect_rc_in "proteus portal unmark example-ssid" 64 65 66 -- \
    proteus portal unmark example-ssid
expect_rc_in "proteus portal open"                64 65 66 -- proteus portal open

# Unpin (GH#391, N12.1) — newly gated. Without --yes, the destructive
# unpin must refuse before touching state.
expect_rc_in "proteus unpin wlan0" 64 65 66 -- proteus unpin wlan0

# Watch with --interval 0s (CL1 / GH#349) — must reject at parse time
# with CONFIG_ERROR (65) instead of CPU-burning. We use `proteus current`
# because it's quick to dispatch and has --watch wired.
expect_rc_in "proteus current --watch --interval 0s" 65 -- \
    proteus current --watch --interval 0s

# Sub-millisecond intervals (CL7) — same fix family as CL1.
# `0ms` is the canonical case; reject parses below 1ms uniformly.
expect_rc_in "proteus current --watch --interval 0ms" 65 -- \
    proteus current --watch --interval 0ms

# Timer resume (GH#352) — short-name now maps to proteus-resume.service.
# Without --yes, the gate fires before we hit the systemd lookup.
expect_rc_in "proteus timer enable resume"  64 65 66 -- \
    proteus timer enable resume
expect_rc_in "proteus timer disable resume" 64 65 66 -- \
    proteus timer disable resume

finish 06-yes-gates
