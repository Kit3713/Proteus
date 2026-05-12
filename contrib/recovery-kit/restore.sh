#!/usr/bin/env bash
set -euo pipefail
usage(){ cat >&2 <<USAGE
Usage: $0 <bundle> [--yes] [--force] [--purge-targets] [--confirm-host=HOST] [--only=config|state|personas] [--plan] [--json]
USAGE
}
[[ $# -ge 1 ]] || { usage; exit 2; }
bundle="$1"; shift

auto_yes=0; force=0; purge=0; only=""; plan=0; json=0; confirm_host=""
for a in "$@"; do case "$a" in --yes)auto_yes=1;; --force)force=1;; --purge-targets)purge=1;; --only=*)only="${a#*=}";; --plan)plan=1;; --json)json=1;; --confirm-host=*)confirm_host="${a#*=}";; *) exit 2;; esac; done

lock="/tmp/proteus-recovery.lock"; exec 9>"$lock"; command -v flock >/dev/null && flock -n 9 || { echo "Recovery lock busy" >&2; exit 40; }

manifest="${bundle%.tar.gz}.manifest.json"; manifest="${manifest%.tar.zst}.manifest.json"; checksum="${bundle%.tar.gz}.sha256"; checksum="${checksum%.tar.zst}.sha256"
[[ -f "$manifest" && -f "$checksum" ]] || exit 1
(cd "$(dirname "$bundle")" && sha256sum -c "$(basename "$checksum")")
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$manifest"

host="$(hostname -f 2>/dev/null || hostname)"; [[ -z "$confirm_host" || "$confirm_host" == "$host" ]] || { echo "host mismatch" >&2; exit 1; }
meta_schema="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("schema_version","unknown"))' "$manifest")"
cur_schema="unknown"; [[ -f /var/lib/proteus/state.json ]] && cur_schema="$(sed -n 's/.*"schema_version"[[:space:]]*:[[:space:]]*"\{0,1\}\([^",}]*\).*/\1/p' /var/lib/proteus/state.json | head -n1)"
[[ "$meta_schema" == "$cur_schema" || "$meta_schema" == "unknown" || "$cur_schema" == "unknown" || $force -eq 1 ]] || { echo "schema mismatch" >&2; exit 1; }

[[ $purge -eq 0 || $force -eq 1 ]] || { echo "--purge-targets requires --force" >&2; exit 1; }
if [[ $purge -eq 1 && $auto_yes -ne 1 ]]; then token="PURGE-${host}-$(date -u +%Y%m%d%H%M)"; echo "Type $token"; read -r t; [[ "$t" == "$token" ]] || exit 1; fi

allowed=(etc/proteus var/lib/proteus usr/share/proteus/personas)
case "$only" in "") ;; config) allowed=(etc/proteus) ;; state) allowed=(var/lib/proteus) ;; personas) allowed=(usr/share/proteus/personas) ;; *) exit 2;; esac
validate(){ m="${1#./}"; [[ "$m" != /* && "$m" != *..* ]] || return 1; for p in "${allowed[@]}"; do [[ "$m" == "$p" || "$m" == "$p/"* ]] && return 0; done; return 1; }
while IFS= read -r m; do validate "$m" || exit 1; done < <(tar -tf "$bundle")

if [[ $plan -eq 1 ]]; then echo "pre-state"; for s in proteus.service proteus-dispatcher.service proteus-events.service; do systemctl is-active "$s" 2>/dev/null || true; done; tar -tf "$bundle"; exit 0; fi

[[ $EUID -eq 0 ]] || exit 1

services=(proteus.service proteus-dispatcher.service proteus-events.service)
report=(); running=()
for s in "${services[@]}"; do st=$(systemctl is-active "$s" 2>/dev/null || echo unknown); report+=("$s:$st"); [[ "$st" == active ]] && running+=("$s"); done
((${#running[@]})) && systemctl stop "${running[@]}" || true

staging="$(mktemp -d)"; trap 'rm -rf "$staging"' EXIT
[[ "$bundle" == *.zst ]] && tar --zstd -xf "$bundle" -C "$staging" || tar -xzf "$bundle" -C "$staging"

rollback="[]"; actions="[]"
restore_fail(){ python3 - "$audit" "$actions" "$rollback" <<'PY'
import json,sys
p,a,r=sys.argv[1:4]
json.dump({"status":"failed","actions":json.loads(a),"rollback":json.loads(r)},open(p,'w'))
PY
exit 1; }

ts="$(date -u +%Y%m%d-%H%M%S)"; audit="$(dirname "$bundle")/run-$ts.json"
for p in "${allowed[@]}"; do src="/$p"; st="$staging/$p"; [[ -e "$st" ]] || continue; if [[ -e "$src" ]]; then b="${src}.pre-recovery.$ts"; mv "$src" "$b" || restore_fail; rollback=$(python3 - <<PY
import json; arr=json.loads('''$rollback'''); arr.append({"from":"$b","to":"$src"}); print(json.dumps(arr))
PY
); fi; [[ $purge -eq 1 ]] && rm -rf "$src"; mkdir -p "$(dirname "$src")"; cp -a "$st" "$src" || restore_fail; done

for s in "${running[@]}"; do
  ok=0
  for _ in 1 2 3; do systemctl start "$s" || true; timeout 10 bash -c "until systemctl is-active --quiet '$s'; do sleep 1; done" && { ok=1; break; }; done
  [[ $ok -eq 1 ]] || restore_fail
done
post=(); for s in "${services[@]}"; do post+=("$s:$(systemctl is-active "$s" 2>/dev/null || echo unknown)"); done

python3 - "$audit" "$manifest" "$bundle" "$host" "${report[*]}" "${post[*]}" <<'PY'
import json,sys
audit,manifest,bundle,host,pre,post=sys.argv[1:7]
meta=json.load(open(manifest))
json.dump({"status":"ok","bundle":bundle,"host":host,"manifest":meta,"pre_state":pre.split(),'post_state':post.split()},open(audit,'w'))
PY

[[ $json -eq 1 ]] && cat "$audit" || echo "Restore complete"
