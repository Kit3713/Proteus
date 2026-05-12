#!/usr/bin/env bash
set -euo pipefail

usage(){ cat >&2 <<USAGE
Usage: $0 <bundle.tar.gz|tar.zst> [--yes] [--force] [--purge-targets] [--only config|state|personas] [--plan] [--confirm-host HOST] [--json]
USAGE
}

[[ $# -ge 1 ]] || { usage; exit 2; }
bundle="$1"; shift

auto_yes=0; force=0; purge=0; only=""; plan=0; confirm_host=""; json=0; no_bounce=0
for a in "$@"; do
  case "$a" in
    --yes) auto_yes=1 ;;
    --force) force=1 ;;
    --purge-targets) purge=1 ;;
    --plan) plan=1 ;;
    --json) json=1 ;;
    --no-service-bounce) no_bounce=1 ;;
    --only=*) only="${a#*=}" ;;
    --confirm-host=*) confirm_host="${a#*=}" ;;
    *) echo "Unknown arg: $a" >&2; usage; exit 2 ;;
  esac
done

for c in tar sha256sum mktemp; do command -v "$c" >/dev/null || { echo "$c required" >&2; exit 1; }; done
[[ -f "$bundle" ]] || { echo "Bundle not found: $bundle" >&2; exit 1; }

manifest="${bundle%.tar.gz}.manifest.json"; manifest="${manifest%.tar.zst}.manifest.json"
checksum="${bundle%.tar.gz}.sha256"; checksum="${checksum%.tar.zst}.sha256"
[[ -f "$manifest" ]] || { echo "Missing manifest: $manifest" >&2; exit 1; }
[[ -f "$checksum" ]] || { echo "Missing checksum: $checksum" >&2; exit 1; }

( cd "$(dirname "$bundle")" && sha256sum -c "$(basename "$checksum")" )

if [[ -f "${checksum}.sig" ]]; then
  if command -v gpg >/dev/null; then gpg --verify "${checksum}.sig" "$checksum"; elif command -v minisign >/dev/null; then minisign -Vm "$checksum" -x "${checksum}.sig"; else echo "Signature present but neither gpg nor minisign installed" >&2; exit 1; fi
fi

current_host="$(hostname -f 2>/dev/null || hostname)"
meta_host="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("host",""))' "$manifest")"
meta_schema="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("schema_version","unknown"))' "$manifest")"
cur_schema="unknown"; [[ -f /var/lib/proteus/state.json ]] && cur_schema="$(sed -n 's/.*"schema_version"[[:space:]]*:[[:space:]]*"\{0,1\}\([^",}]*\).*/\1/p' /var/lib/proteus/state.json | head -n1 || true)"

if [[ -n "$confirm_host" && "$confirm_host" != "$current_host" ]]; then echo "Host confirmation failed" >&2; exit 1; fi
if [[ "$meta_schema" != "unknown" && "$cur_schema" != "unknown" && "$meta_schema" != "$cur_schema" && $force -ne 1 ]]; then
  echo "Schema mismatch backup=$meta_schema current=$cur_schema (use --force)" >&2; exit 1
fi

allowed=(etc/proteus var/lib/proteus usr/share/proteus/personas)
case "$only" in
  "" ) ;;
  config) allowed=(etc/proteus) ;;
  state) allowed=(var/lib/proteus) ;;
  personas) allowed=(usr/share/proteus/personas) ;;
  *) echo "Invalid --only" >&2; exit 2 ;;
esac

validate_member(){ local m="${1#./}"; [[ "$m" != /* && "$m" != *".."* ]] || return 1; local p; for p in "${allowed[@]}"; do [[ "$m" == "$p" || "$m" == "$p/"* ]] && return 0; done; return 1; }
while IFS= read -r m; do validate_member "$m" || { echo "Unexpected path in archive: $m" >&2; exit 1; }; done < <(tar -tf "$bundle")

if [[ $plan -eq 1 ]]; then
  echo "Plan: restore $(printf '%s ' "${allowed[@]}") from $bundle"
  tar -tf "$bundle"
  exit 0
fi

if [[ $auto_yes -ne 1 ]]; then
  token="RESTORE-$(date -u +%Y%m%d%H%M)"
  echo "Type $token to continue restore on $current_host"
  read -r typed
  [[ "$typed" == "$token" ]] || { echo "Aborted"; exit 1; }
fi

[[ $EUID -eq 0 ]] || { echo "Run as root for restore" >&2; exit 1; }
[[ $(df --output=avail / | tail -n1) -gt 102400 ]] || { echo "Insufficient free space on /" >&2; exit 1; }

services=(proteus.service proteus-dispatcher.service proteus-events.service)
running=()
if [[ $no_bounce -ne 1 ]] && command -v systemctl >/dev/null; then
  for s in "${services[@]}"; do systemctl is-active --quiet "$s" && running+=("$s"); done
  ((${#running[@]})) && systemctl stop "${running[@]}" || true
fi

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
if [[ "$bundle" == *.zst ]]; then tar --zstd -xf "$bundle" -C "$tmp"; else tar -xzf "$bundle" -C "$tmp"; fi

rollback_log=()
restore_fail(){ echo "Restore failed, rolling back" >&2; for pair in "${rollback_log[@]}"; do src="${pair%%:*}"; dst="${pair##*:}"; rm -rf "$dst"; [[ -e "$src" ]] && mv "$src" "$dst"; done; exit 1; }

for p in "${allowed[@]}"; do
  src="/$p"; staged="$tmp/$p"
  [[ -e "$staged" ]] || continue
  if [[ -e "$src" ]]; then bak="${src}.pre-recovery.$(date -u +%Y%m%d-%H%M%S)"; mv "$src" "$bak" || restore_fail; rollback_log+=("$bak:$src"); fi
  [[ $purge -eq 1 ]] && rm -rf "$src"
  mkdir -p "$(dirname "$src")"
  cp -a "$staged" "$src" || restore_fail
done

if [[ $no_bounce -ne 1 ]] && command -v systemctl >/dev/null && ((${#running[@]})); then
  systemctl start "${running[@]}" || restore_fail
  for s in "${running[@]}"; do systemctl is-active --quiet "$s" || restore_fail; done
fi

if [[ $json -eq 1 ]]; then printf '{"restored":true,"host":"%s","bundle":"%s"}\n' "$current_host" "$bundle"; else echo "Restore complete from $bundle"; fi
