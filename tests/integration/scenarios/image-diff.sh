#!/usr/bin/env bash
# Roadmap Milestone 6: image-diff verification of clean install/uninstall.
# Take a SHA tree of the install-target directories before install, run
# install + uninstall + purge, and assert the result is byte-identical to
# the pre-install state.
#
# Catches: stray files left behind by a future install.sh refactor;
# state-cache files written outside the documented dirs; backup files
# accumulated by `proteus reset`.
#
# Runs in the integration-test container alongside the rest of the
# tests/integration/scenarios/ family. Requires root inside the container.

set -euo pipefail

WORKDIR="${WORKDIR:-/tmp/proteus-image-diff}"
INSTALL_SH="${INSTALL_SH:-/proteus/install.sh}"
UNINSTALL_SH="${UNINSTALL_SH:-/proteus/uninstall.sh}"

mkdir -p "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

# Directories that MUST be byte-identical before install and after uninstall.
# /var/log is excluded because journal/syslog rotates independently and
# would dominate the diff.
TRACK_DIRS=(/etc /usr/bin /usr/sbin /usr/share/man /usr/share/bash-completion /usr/share/zsh /usr/share/fish /usr/lib/systemd/system /var/lib)

snap() {
  local label="$1"
  local out="$WORKDIR/snap-$label.txt"
  : > "$out"
  for d in "${TRACK_DIRS[@]}"; do
    if [ -d "$d" ]; then
      # SHA the file contents + the path. Sort for determinism.
      find "$d" -type f -print0 2>/dev/null \
        | xargs -0 sha256sum 2>/dev/null \
        | sort \
        >> "$out" || true
    fi
  done
  echo "$out"
}

echo "==> snapshot: pre-install"
pre=$(snap pre)

echo "==> install"
bash "$INSTALL_SH"

echo "==> uninstall + purge"
bash "$UNINSTALL_SH" --purge

echo "==> snapshot: post-uninstall"
post=$(snap post)

echo "==> diff"
if diff -u "$pre" "$post" > "$WORKDIR/diff.txt"; then
  echo "PASS: install/uninstall produced byte-identical result"
  exit 0
fi

added=$(diff -u "$pre" "$post" | grep -c '^+[^+]' || true)
removed=$(diff -u "$pre" "$post" | grep -c '^-[^-]' || true)

echo "FAIL: image diff non-empty (+$added / -$removed lines)"
echo
echo "==> diff (first 40 lines)"
head -n 40 "$WORKDIR/diff.txt"
echo
echo "    full diff at $WORKDIR/diff.txt"
exit 1
