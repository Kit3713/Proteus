#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<USAGE
Usage: $0 [out_dir] [--keep N] [--compression gzip|zstd] [--json]
USAGE
}

out_dir="."
keep=""
compression="gzip"
json=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep) keep="${2:-}"; shift 2 ;;
    --compression) compression="${2:-}"; shift 2 ;;
    --json) json=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) out_dir="$1"; shift ;;
  esac
done

command -v tar >/dev/null || { echo "tar is required" >&2; exit 1; }
command -v sha256sum >/dev/null || { echo "sha256sum is required" >&2; exit 1; }
[[ -d "$out_dir" ]] || mkdir -p "$out_dir"
[[ -w "$out_dir" ]] || { echo "Output path not writable: $out_dir" >&2; exit 1; }

case "$compression" in
  gzip) ext="tar.gz"; tar_flag="-czf" ;;
  zstd) command -v zstd >/dev/null || { echo "zstd is required for --compression zstd" >&2; exit 1; }; ext="tar.zst"; tar_flag="--zstd -cf" ;;
  *) echo "Unsupported compression: $compression" >&2; exit 2 ;;
esac

ts="$(date -u +%Y%m%d-%H%M%S)"
base="proteus-backup-$ts"
bundle="$out_dir/$base.$ext"
manifest="$out_dir/$base.manifest.json"
checksum="$out_dir/$base.sha256"

allowed_rel=(etc/proteus var/lib/proteus usr/share/proteus/personas)
existing_rel=()
for rel in "${allowed_rel[@]}"; do [[ -e "/$rel" ]] && existing_rel+=("$rel"); done
[[ ${#existing_rel[@]} -gt 0 ]] || { echo "No known Proteus paths were found to back up." >&2; exit 1; }

# metadata
proteus_version="unknown"
command -v proteus >/dev/null && proteus_version="$(proteus --version 2>/dev/null | head -n1 || echo unknown)"
schema_version="unknown"
[[ -f /var/lib/proteus/state.json ]] && schema_version="$(sed -n 's/.*"schema_version"[[:space:]]*:[[:space:]]*"\{0,1\}\([^",}]*\).*/\1/p' /var/lib/proteus/state.json | head -n1 || true)"
host="$(hostname -f 2>/dev/null || hostname)"
os="$(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}")"

# archive + manifest
tar -C / --numeric-owner --owner=0 --group=0 $tar_flag "$bundle" "${existing_rel[@]}"

file_entries="[]"
for rel in "${existing_rel[@]}"; do
  if [[ -f "/$rel" ]]; then
    size=$(stat -c '%s' "/$rel" 2>/dev/null || echo 0)
    sum=$(sha256sum "/$rel" | awk '{print $1}')
    file_entries=$(python3 - <<PY
import json
arr=json.loads('''$file_entries''')
arr.append({"path":"/$rel","bytes":$size,"sha256":"$sum"})
print(json.dumps(arr))
PY
)
  fi
done

cat > "$manifest" <<JSON
{"timestamp_utc":"$ts","host":"$host","os":"$os","proteus_version":"$proteus_version","schema_version":"$schema_version","compression":"$compression","bundle":"$(basename "$bundle")","paths":$(printf '%s\n' "${existing_rel[@]}" | python3 -c 'import json,sys; print(json.dumps(["/"+l.strip() for l in sys.stdin if l.strip()]))'),"files":$file_entries}
JSON

sha256sum "$(basename "$bundle")" "$(basename "$manifest")" > "$checksum" 2>/dev/null || (cd "$out_dir" && sha256sum "$(basename "$bundle")" "$(basename "$manifest")" > "$(basename "$checksum")")

if [[ -n "$keep" ]]; then
  [[ "$keep" =~ ^[0-9]+$ ]] || { echo "--keep must be numeric" >&2; exit 2; }
  ls -1t "$out_dir"/proteus-backup-*.tar.* 2>/dev/null | tail -n +$((keep+1)) | xargs -r rm -f
  ls -1t "$out_dir"/proteus-backup-*.manifest.json 2>/dev/null | tail -n +$((keep+1)) | xargs -r rm -f
  ls -1t "$out_dir"/proteus-backup-*.sha256 2>/dev/null | tail -n +$((keep+1)) | xargs -r rm -f
fi

if [[ $json -eq 1 ]]; then
  printf '{"bundle":"%s","manifest":"%s","checksum":"%s"}\n' "$bundle" "$manifest" "$checksum"
else
  echo "Created backup: $bundle"
  echo "Manifest: $manifest"
  echo "Checksum: $checksum"
fi
