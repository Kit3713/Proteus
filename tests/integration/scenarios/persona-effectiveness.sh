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
trap 'rm -f "$baseline_out" "$persona_out"' EXIT
nmap -O -Pn "$TARGET_IP" 2>&1 | tee "$baseline_out" || true
echo

# 2. Apply persona, capture again.
echo "==> persona: applying iphone-15 + capturing nmap -O"
"$PROTEUS" persona use iphone-15 --yes
"$PROTEUS" apply --yes

# Give NM/networkd a moment to push the new MAC + DHCP fingerprint.
sleep 5

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
