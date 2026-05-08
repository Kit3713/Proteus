#!/usr/bin/env bash
# Read-only network-state capture for real-world testing.
#
# See tests/realworld/README.md for the rationale + privacy notes.
# This script is intentionally simple shell so it runs on any host
# that has proteus installed, without any python/perl deps.

set -euo pipefail

TARBALL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --tarball) TARBALL="$2"; shift 2 ;;
    --tarball=*) TARBALL="${1#--tarball=}"; shift ;;
    --help|-h)
      echo "usage: $0 [--tarball PATH]"
      echo "  PATH: write a tar.gz of every probe output (mode 0o600)"
      echo "        otherwise prints a single concatenated text dump on stdout"
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 64 ;;
  esac
done

if [ "$(id -u)" != "0" ]; then
  echo "warn: most probes need root (proteus doctor is read-only but iw/journalctl are gated)" >&2
fi

WORKDIR=$(mktemp -d /tmp/proteus-probe-XXXXXX)
chmod 0700 "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

run() {
  local label="$1"; shift
  local out="$WORKDIR/${label}.txt"
  {
    echo "==> $label"
    echo "    $ $*"
    "$@" 2>&1 || echo "(exit $?)"
    echo
  } > "$out"
  if [ -z "$TARBALL" ]; then
    cat "$out"
  fi
}

# Anonymisation pass — sed-replace in place. Public IPs and SSIDs go to
# RFC docs prefixes / a redaction marker. Run on every text file before
# the tarball is sealed.
anonymise() {
  local f="$1"
  # Conservative: replace strings that look like public IPv4 / IPv6.
  # Skip RFC 1918 / loopback / link-local — those are uninteresting and
  # leaving them helps debugging.
  #
  # IPv4 strategy (issue #263): tag private/loopback/link-local IPs with a
  # sentinel marker before the public-IP catch-all runs, then untag.
  # The previous one-liner replaced every IP and tried to "preserve"
  # privates by re-replacing tagged-but-already-overwritten strings —
  # which is a no-op once the original text is gone.
  #
  # IPv6 strategy (issue #264): require at least one `:` separator in
  # the match so a 64-hex-char SHA256 string can't trip it.
  #
  # SSID/secret strategy (issue #264): use explicit `[Ss][Ss][Ii][Dd]`
  # instead of GNU sed's non-POSIX `I` flag so the script runs under
  # any POSIX `sed -E`.
  sed -i -E '
    s/\b(10|127|169\.254|172\.(1[6-9]|2[0-9]|3[01])|192\.168)\.([0-9]+\.[0-9]+\.[0-9]+|[0-9]+\.[0-9]+|[0-9]+)\b/__PROTEUS_KEEP__&__PROTEUS_KEEP_END__/g;
    s/\b[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+\b/203.0.113.X/g;
    s/__PROTEUS_KEEP__([^_]+)__PROTEUS_KEEP_END__/\1/g;
    s/[0-9a-fA-F]{0,4}(:[0-9a-fA-F]{0,4}){2,}/2001:db8::X/g;
    s/([Ss][Ss][Ii][Dd]="?|"[Ss][Ss][Ii][Dd]": "?)[^",]*/\1<REDACTED>/g;
    s/([Pp][Aa][Ss][Ss][Kk][Ee][Yy]="?|[Pp][Ss][Kk]="?|[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd]="?|"[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd]": "?)[^",]*/\1<REDACTED>/g;
  ' "$f" 2>/dev/null || true
}

# --- Proteus self-reporting ---
run proteus-version proteus --version
run proteus-doctor proteus doctor --json
run proteus-status proteus status --json
run proteus-current proteus current --json
run proteus-session proteus session --json || true
run proteus-portal proteus portal status --json || true
run proteus-config proteus show-config --json || true

# --- Network state ---
run ip-addr ip -j addr
run ip-route ip -j route
run ip-link ip -j link
run resolv-conf cat /etc/resolv.conf || true

if command -v nmcli >/dev/null 2>&1; then
  run nmcli-conn nmcli -t connection show
  run nmcli-dev nmcli -t device status
  run nmcli-radio nmcli -t radio
fi

if command -v iw >/dev/null 2>&1; then
  for iface in $(ls /sys/class/net 2>/dev/null); do
    if [ -d "/sys/class/net/$iface/phy80211" ]; then
      run "iw-link-$iface" iw dev "$iface" link || true
      run "iw-info-$iface" iw dev "$iface" info || true
    fi
  done
fi

run dig dig +short @1.1.1.1 example.com || true
run ping-1111 ping -c 2 -W 1 1.1.1.1 || true

if command -v journalctl >/dev/null 2>&1; then
  run journal-nm journalctl -u NetworkManager -n 50 --no-pager || true
  run journal-resolved journalctl -u systemd-resolved -n 50 --no-pager || true
fi

# Anonymise everything before sealing.
for f in "$WORKDIR"/*.txt; do
  anonymise "$f"
done

if [ -n "$TARBALL" ]; then
  tar czf "$TARBALL" -C "$WORKDIR" .
  chmod 0600 "$TARBALL"
  echo "wrote $TARBALL ($(stat -c%s "$TARBALL") bytes)"
fi
