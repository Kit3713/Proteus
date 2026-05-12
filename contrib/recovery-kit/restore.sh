#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<USAGE
Usage: $0 <bundle.tar.gz> [--yes] [--dry-run] [--keep-existing] [--no-service-bounce]
USAGE
}

if [[ $# -lt 1 ]]; then
  usage
  exit 2
fi

bundle="$1"
shift || true

auto_yes=0
dry_run=0
keep_existing=0
service_bounce=1
for arg in "$@"; do
  case "$arg" in
    --yes) auto_yes=1 ;;
    --dry-run) dry_run=1 ;;
    --keep-existing) keep_existing=1 ;;
    --no-service-bounce) service_bounce=0 ;;
    *) echo "Unknown argument: $arg" >&2; usage; exit 2 ;;
  esac
done

[[ -f "$bundle" ]] || { echo "Bundle not found: $bundle" >&2; exit 1; }

allowed_rel=(etc/proteus var/lib/proteus usr/share/proteus/personas)

validate_member() {
  local member="$1"
  member="${member#./}"
  [[ "$member" != /* ]] || return 1
  [[ "$member" != *".."* ]] || return 1
  local prefix
  for prefix in "${allowed_rel[@]}"; do
    [[ "$member" == "$prefix" || "$member" == "$prefix/"* ]] && return 0
  done
  return 1
}

while IFS= read -r member; do
  validate_member "$member" || { echo "Refusing restore: unexpected path: $member" >&2; exit 1; }
done < <(tar -tzf "$bundle")

if [[ $auto_yes -ne 1 ]]; then
  token="RESTORE"
  echo "Aggressive restore mode will replace existing Proteus config/state data."
  read -r -p "Type $token to continue: " typed
  [[ "$typed" == "$token" ]] || { echo "Aborted."; exit 1; }
fi

if [[ $dry_run -eq 1 ]]; then
  echo "Dry run OK: archive validated."
  exit 0
fi

if [[ $service_bounce -eq 1 ]] && command -v systemctl >/dev/null 2>&1; then
  systemctl stop proteus.service proteus-dispatcher.service proteus-events.service 2>/dev/null || true
fi

if [[ $keep_existing -eq 0 ]]; then
  for rel in "${allowed_rel[@]}"; do
    rm -rf "/$rel"
  done
fi

tar -xzf "$bundle" -C /

if [[ $service_bounce -eq 1 ]] && command -v systemctl >/dev/null 2>&1; then
  systemctl start proteus.service proteus-dispatcher.service proteus-events.service 2>/dev/null || true
fi

echo "Restore complete from $bundle"
