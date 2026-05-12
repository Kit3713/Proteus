#!/usr/bin/env bash
# Roadmap Milestone 2 acceptance: nmap -O before/after a persona apply
# should produce materially different OS-detection output.
#
# Runs in two halves:
#   1. baseline — capture nmap -O of the target host with no persona applied
#   2. persona  — apply iphone-15, capture nmap -O again, assert the
#                 detection row changed
#
# Requires CAP_NET_RAW for nmap -O (raw-socket TCP/IP probes); intended to
# run in the integration-test container alongside the rest of the
# tests/integration/scenarios/ family. NOT a unit test — the
# interpretation of "materially different" is intentionally fuzzy. CI
# wires this up as a smoke-only scenario; failures are reported but don't
# gate the build.

set -euo pipefail

PROTEUS="${PROTEUS:-/usr/bin/proteus}"
TARGET_IFACE="${TARGET_IFACE:-eth0}"
TARGET_IP="${TARGET_IP:-127.0.0.1}"

echo "==> persona-effectiveness scenario starting"
echo "    proteus = $PROTEUS"
echo "    iface   = $TARGET_IFACE"
echo "    target  = $TARGET_IP"

if ! command -v nmap >/dev/null 2>&1; then
  echo "skip: nmap not installed (apk add nmap / dnf install nmap)"
  exit 0
fi

# 1. Baseline — clear any active persona, capture nmap -O.
echo "==> baseline: clearing persona + capturing nmap -O"
"$PROTEUS" persona clear --yes >/dev/null 2>&1 || true
"$PROTEUS" apply --yes >/dev/null 2>&1 || true

baseline_out=$(mktemp)
# `persona_out` is created later (after the poll loop); initialize empty so
# the EXIT trap doesn't trip `set -u` on early exits (timeout, SIGINT).
persona_out=""
trap 'rm -f "$baseline_out" "$persona_out"' EXIT
nmap -O -Pn "$TARGET_IP" 2>&1 | tee "$baseline_out" || true
echo

# Extract a single field from `proteus current --json --iface $TARGET_IFACE`.
# Used to gate the persona apply on an observable MAC / last_rotated change
# rather than a fixed sleep. Pretty-printed JSON, one entry per iface, so a
# grep + sed pair is enough — avoids pulling in jq, which isn't guaranteed on
# every CI runner.
proteus_field() {
  # $1 = field name; reads JSON from stdin. Never fails — `grep`'s no-match
  # rc=1 would otherwise trip `set -e`/`pipefail` on an absent field.
  { grep -E "^[[:space:]]*\"$1\":" || true; } | head -n 1 \
    | sed -E "s/.*\"$1\"[[:space:]]*:[[:space:]]*//;s/,[[:space:]]*\$//;s/^\"(.*)\"\$/\1/"
}

# Capture the pre-apply MAC + last_rotated so the poll loop below has
# something concrete to compare against.
baseline_json=$("$PROTEUS" current --json --iface "$TARGET_IFACE" 2>/dev/null || true)
baseline_mac=$(printf '%s\n' "$baseline_json" | proteus_field mac)
baseline_rotated=$(printf '%s\n' "$baseline_json" | proteus_field last_rotated)
echo "    pre-apply MAC          = ${baseline_mac:-<unknown>}"
echo "    pre-apply last_rotated = ${baseline_rotated:-<unknown>}"

# 2. Apply persona, capture again.
echo "==> persona: applying iphone-15 + capturing nmap -O"
"$PROTEUS" persona use iphone-15 --yes
"$PROTEUS" apply --yes

# NTEST.2: a fixed `sleep 5` here was flaky on slow CI runners — the apply
# could still be propagating through NM/networkd by the time the second
# nmap -O fired, so baseline and persona output looked identical and the
# scenario reported a false negative. Poll `proteus current --json` until
# either the MAC differs from the pre-apply baseline AND last_rotated has
# advanced past its pre-apply value, or the (configurable) timeout fires.
timeout_secs="${PROTEUS_PERSONA_EFFECT_TIMEOUT_SECS:-60}"
deadline=$(( $(date +%s) + timeout_secs ))
poll_ok=0
while [ "$(date +%s)" -lt "$deadline" ]; do
  current_json=$("$PROTEUS" current --json --iface "$TARGET_IFACE" 2>/dev/null || true)
  current_mac=$(printf '%s\n' "$current_json" | proteus_field mac)
  current_rotated=$(printf '%s\n' "$current_json" | proteus_field last_rotated)

  mac_changed=0
  if [ -n "$current_mac" ] && [ "$current_mac" != "null" ] \
     && [ "$current_mac" != "$baseline_mac" ]; then
    mac_changed=1
  fi

  # ISO-8601 strings sort lexicographically the same way they sort
  # chronologically, so a plain string compare is correct here. An empty
  # baseline_rotated (never rotated before) counts as advanced the moment
  # we see any non-null stamp.
  rotated_advanced=0
  if [ -n "$current_rotated" ] && [ "$current_rotated" != "null" ]; then
    if [ -z "$baseline_rotated" ] || [ "$baseline_rotated" = "null" ] \
       || [ "$current_rotated" \> "$baseline_rotated" ]; then
      rotated_advanced=1
    fi
  fi

  if [ "$mac_changed" -eq 1 ] && [ "$rotated_advanced" -eq 1 ]; then
    poll_ok=1
    echo "    post-apply MAC          = $current_mac"
    echo "    post-apply last_rotated = $current_rotated"
    break
  fi
  sleep 1
done

if [ "$poll_ok" -ne 1 ]; then
  echo "TIMEOUT waiting for persona-driven MAC/DHCP change after ${timeout_secs}s"
  echo "      baseline MAC          = ${baseline_mac:-<unknown>}"
  echo "      baseline last_rotated = ${baseline_rotated:-<unknown>}"
  echo "      latest   MAC          = ${current_mac:-<unknown>}"
  echo "      latest   last_rotated = ${current_rotated:-<unknown>}"
  echo "      override the wait via PROTEUS_PERSONA_EFFECT_TIMEOUT_SECS=N"
  exit 1
fi

persona_out=$(mktemp)
nmap -O -Pn "$TARGET_IP" 2>&1 | tee "$persona_out" || true
echo

# 3. Compare. We're checking that the OS detection line changed — exact
#    semantic match against "Apple iOS" is flaky (nmap's database varies),
#    so the assertion is "the row differs at all".
baseline_os=$(grep -E "^(Aggressive |OS details:)" "$baseline_out" | head -n 1 || true)
persona_os=$(grep -E "^(Aggressive |OS details:)" "$persona_out" | head -n 1 || true)

echo "==> comparison"
echo "    baseline OS line: $baseline_os"
echo "    persona  OS line: $persona_os"

if [ -z "$baseline_os" ] && [ -z "$persona_os" ]; then
  echo "skip: nmap produced no OS row in either run (target may not be probable from this segment)"
  exit 0
fi

if [ "$baseline_os" = "$persona_os" ]; then
  echo "FAIL: nmap -O produced identical OS detection before/after persona apply"
  echo "      this means the persona's OUI / DHCP / TCP-stack shaping isn't"
  echo "      flowing onto the wire. Check proteus -vv apply --yes for details."
  exit 1
fi

echo "PASS: persona apply materially changed nmap -O detection"
exit 0
