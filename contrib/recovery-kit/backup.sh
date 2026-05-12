#!/usr/bin/env bash
set -euo pipefail

out_dir="${1:-.}"
mkdir -p "$out_dir"

if [[ ! -d "$out_dir" ]]; then
  echo "Output path is not a directory: $out_dir" >&2
  exit 1
fi

ts="$(date -u +%Y%m%d-%H%M%S)"
bundle="$out_dir/proteus-backup-$ts.tar.gz"

# Paths are stored as archive-relative paths (no leading slash) so restore can
# validate entries before extracting.
allowed_rel=(etc/proteus var/lib/proteus usr/share/proteus/personas)
existing_rel=()
for rel in "${allowed_rel[@]}"; do
  [[ -e "/$rel" ]] && existing_rel+=("$rel")
done

if [[ ${#existing_rel[@]} -eq 0 ]]; then
  echo "No known Proteus paths were found to back up." >&2
  exit 1
fi

# Deterministic owner/group values make backups portable across hosts.
tar -C / --numeric-owner --owner=0 --group=0 -czf "$bundle" "${existing_rel[@]}"
echo "Created backup: $bundle"
echo "Included paths: ${existing_rel[*]}"
