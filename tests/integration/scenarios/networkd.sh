#!/bin/sh
# tests/integration/scenarios/networkd.sh
#
# Roadmap Milestone 1: per-backend integration scenario for the
# `systemd-networkd` backend. Skeleton — the full `backend::networkd`
# implementation lands in a follow-up; this file pins the test
# harness's expectation that one will exist.
#
# Driver from the suite's run.sh:
#   ./tests/integration/run.sh --scenario networkd
#
# TODO Milestone 1 follow-up:
#   - Stand up a minimal Fedora-server / NixOS container that has
#     networkd active and NM removed.
#   - Pre-seed `/etc/systemd/network/10-test.network` so a managed
#     ethernet device exists.
#   - Drive `proteus apply --yes` and assert NM is NOT consulted
#     (backend::nm::available() returns false).
#   - Drive `proteus rotate --yes` and assert the cloned MAC lands
#     via a drop-in under `/etc/systemd/network/proteus.d/`.
#   - Drive `proteus revert --yes` and assert the drop-in is removed
#     plus `networkctl reload` runs.
#   - Drive `proteus rotate-if-needed --cooldown 60 --yes` and
#     assert the cooldown is honoured.

set -u

# shellcheck source=lib.sh
. "$(dirname "$0")/lib.sh"
FAILED=0

NAME="networkd"

# Bail with a clear "skipped" line until the backend is wired —
# the harness treats this as a deliberate skip, not a failure.
printf 'scenario %s: skipped (Milestone 1 follow-up — backend::networkd is a scaffold)\n' "$NAME"
exit 0

# Below this point are the assertions the follow-up PR will turn on.
# Left in tree as a checklist; never executed today.

# expect_zero "systemd is running"           systemctl is-system-running
# expect_zero "systemd-networkd active"      systemctl is-active systemd-networkd
# expect_zero "networkctl reachable"          networkctl --version
# expect_zero "NetworkManager NOT active"     ! systemctl is-active NetworkManager
#
# expect_contains "doctor surfaces networkd entry" "networkd" \
#     proteus doctor
# expect_contains "doctor reports nm unavailable" "nm" \
#     proteus doctor
#
# expect_zero "proteus apply --yes (networkd)"   proteus apply --yes
# expect_zero "proteus rotate --yes (networkd)"  proteus rotate --yes
# expect_zero "proteus revert --yes (networkd)"  proteus revert --yes
#
# finish "$NAME"
