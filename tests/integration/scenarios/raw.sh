#!/bin/sh
# tests/integration/scenarios/raw.sh
#
# Roadmap Milestone 1: per-backend integration scenario for the
# `raw` backend (`ip` + `iw` + `wpa_supplicant`/`iwd` direct).
# Skeleton — the full `backend::raw` implementation lands in a
# follow-up; this file pins the test harness's expectation that one
# will exist.
#
# Driver from the suite's run.sh:
#   ./tests/integration/run.sh --scenario raw
#
# Targets the "any-distro" case from Milestone 5: Alpine + iwd, Void
# + wpa_supplicant, etc. The container will *not* have NM or
# networkd installed — `backend::raw` is the only available backend
# and `proteus apply` must work end-to-end against it.
#
# TODO Milestone 1 follow-up:
#   - Stand up an Alpine + iwd container (no NM, no networkd).
#   - Pre-create a wlan device in the kernel test namespace.
#   - Drive `proteus apply --yes` and assert MAC is set via
#     `ip link set <iface> address <mac>`.
#   - Drive `proteus rotate --yes` and assert the new MAC sticks.
#   - Drive `proteus revert --yes` and assert the factory MAC
#     restores via the same `ip link` path.
#   - Drive `proteus rotate-if-needed --cooldown 60 --yes` and
#     assert the cooldown gate.

set -u

# shellcheck source=lib.sh
. "$(dirname "$0")/lib.sh"
FAILED=0

NAME="raw"

# Bail with a clear "skipped" line until the backend is wired —
# the harness treats this as a deliberate skip, not a failure.
printf 'scenario %s: skipped (Milestone 1 follow-up — backend::raw is a scaffold)\n' "$NAME"
exit 0

# Below this point are the assertions the follow-up PR will turn on.
# Left in tree as a checklist; never executed today.

# expect_zero "ip available"                       command -v ip
# expect_zero "iw available"                       command -v iw
# expect_zero "NetworkManager NOT installed"        ! command -v nmcli
# expect_zero "networkd NOT active"                 ! systemctl is-active systemd-networkd
#
# expect_contains "doctor surfaces raw entry" "raw" \
#     proteus doctor
#
# expect_zero "proteus apply --yes (raw)"   proteus apply --yes
# expect_zero "proteus rotate --yes (raw)"  proteus rotate --yes
# expect_zero "proteus revert --yes (raw)"  proteus revert --yes
#
# finish "$NAME"
