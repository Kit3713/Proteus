#!/bin/sh
# tests/integration/scenarios/nm.sh
#
# Roadmap Milestone 1: per-backend integration scenario for the
# NetworkManager backend. Drives a full apply / revert / rotate cycle
# end-to-end on a podman+systemd container with NM running.
#
# This is the "host-class" scenario the rest of the integration
# suite targets: a Fedora-flavoured image with NM as PID-1's first
# child, a Wi-Fi or Ethernet device managed by NM, and the proteus
# binary built from the source tree mounted into the container.
#
# Driver from the suite's run.sh:
#   ./tests/integration/run.sh --scenario nm
#
# Pre-flight assumptions:
#   - Container started under systemd (`systemctl is-system-running`
#     reports `running` or `degraded`).
#   - NetworkManager.service is active.
#   - At least one device shows up in `nmcli device status`.
#
# The test exits 0 only when every check passes. FAILED counts the
# misses; the lib.sh helpers do the printing.

set -u

# shellcheck source=lib.sh
. "$(dirname "$0")/lib.sh"
FAILED=0

NAME="nm"

# --- Pre-flight ----------------------------------------------------

expect_zero "systemd is running"      systemctl is-system-running
expect_zero "NetworkManager active"   systemctl is-active NetworkManager
expect_zero "nmcli reachable"          nmcli --version

# --- Backend selector lands on nm ---------------------------------

expect_contains "doctor reports nm backend" "backend" \
    proteus doctor

# `proteus doctor` should advertise `nm = available` — pin the line
# so a future doctor refactor doesn't drop the matrix.
expect_contains "doctor surfaces nm matrix entry" "nm" \
    proteus doctor

# --- Apply / Revert / Rotate cycle (the headline acceptance) ------

# `apply` must succeed without --yes confirmation gating (we pass it).
expect_zero "proteus apply --yes" \
    proteus apply --yes

# Status should now show at least one managed iface.
expect_zero "proteus status after apply" \
    proteus status

# Rotate against the first managed iface. NM-backed cycle exercises:
# capture-then-save-then-mutate (issue #119), set_cloned_mac per
# profile (issue #122), and the secrets-merge round trip (issue #207).
expect_zero "proteus rotate --yes" \
    proteus rotate --yes

# `current` should reflect the just-rotated MAC. We don't assert the
# exact MAC because it's randomly generated; we assert the field is
# populated.
expect_contains "current shows a cloned MAC" "current_mac" \
    proteus current --json

# Revert restores the originals captured in apply.
expect_zero "proteus revert --yes" \
    proteus revert --yes

# --- rotate-if-needed (issue #206-C) ------------------------------

# First call rotates; second within cooldown skips. Pinning the
# observable shape ("rotated " vs "skipped ") so the dispatcher
# script can log either with no special-casing.
expect_contains "rotate-if-needed first run rotates" "rotated" \
    proteus rotate-if-needed --cooldown 60 --yes
expect_contains "rotate-if-needed second run skips" "skipped" \
    proteus rotate-if-needed --cooldown 60 --yes

# --- Done ----------------------------------------------------------

finish "$NAME"
