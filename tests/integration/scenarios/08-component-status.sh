#!/bin/sh
# 08-component-status.sh — every component `status` reader works without
# root + emits valid JSON (Roadmap Stream 1 / CL4).
#
# The bluetooth / hostname / ipv6 / enterprise-wifi / stack / dns /
# resolved / ntp / nft / rf / portal / dhcp components all expose a
# `<component> status` reader with a `--json` flag. None of them
# require root for the read path (they introspect sysfs / DBus /
# config) and none of them required coverage in the previous scenarios.
#
# This file is the central "every component status is healthy on a
# cold machine" smoke. If a new component lands without a status
# reader (or its status reader regresses), this trips.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 08-component-status\n'

# Each readable status command exits 0 + emits valid JSON. We don't
# assert any specific field shape here — that's the job of per-component
# scenarios (e.g. 05-doctor.sh) — only the floor contract.
for cmd in \
    "bluetooth status" \
    "hostname status" \
    "ipv6 status" \
    "enterprise-wifi status" \
    "stack status" \
    "dns status" \
    "resolved status" \
    "ntp status" \
    "nft status" \
    "rf status" \
    "rf scan" \
    "rf chipset" \
    "portal status" \
    "portal list" \
    "dhcp status"; do
    # shellcheck disable=SC2086
    expect_zero "proteus $cmd"        proteus $cmd
    # shellcheck disable=SC2086
    expect_zero "proteus $cmd --json" proteus $cmd --json

    # JSON parse check.
    # shellcheck disable=SC2086
    OUT=$(proteus $cmd --json 2>/dev/null) || OUT=""
    if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
        ok "proteus $cmd --json parses"
    else
        fail "proteus $cmd --json did not parse"
    fi
done

finish 08-component-status
