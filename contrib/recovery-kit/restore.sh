#!/usr/bin/env bash
set -euo pipefail
usage(){ cat >&2 <<USAGE
Usage: $0 <bundle> [--yes] [--force] [--purge-targets] [--confirm-host=HOST] [--only=config|state|personas] [--plan] [--json] [--log-file PATH] [--rollback-from-audit FILE]
USAGE
}
[[ $# -ge 1 ]] || { usage; exit 2; }
bundle="$1"; shift

auto_yes=0; force=0; purge=0; only=""; plan=0; json=0; confirm_host=""; log_file=""; rollback_audit=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes) auto_yes=1; shift ;;
    --force) force=1; shift ;;
    --purge-targets) purge=1; shift ;;
    --only=*) only="${1#*=}"; shift ;;
    --plan) plan=1; shift ;;
    --json) json=1; shift ;;
    --confirm-host=*) confirm_host="${1#*=}"; shift ;;
    --log-file) log_file="${2:-}"; shift 2 ;;
    --rollback-from-audit=*) rollback_audit="${1#*=}"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
init_log(){ if [[ -z "$log_file" ]]; then mkdir -p /var/log/proteus-recovery-kit 2>/dev/null || true; log_file="/var/log/proteus-recovery-kit/restore.log"; [[ -w /var/log/proteus-recovery-kit ]] || log_file="/tmp/proteus-recovery-kit.log"; fi; }
log(){ msg="$(ts) restore $*"; echo "$msg" | tee -a "$log_file" >&2; command -v logger >/dev/null && logger -t proteus-recovery-kit "$msg" || true; }
init_log

lock="/tmp/proteus-recovery.lock"; exec 9>"$lock"; command -v flock >/dev/null && flock -n 9 || { log "lock busy"; exit 40; }

if [[ -n "$rollback_audit" ]]; then
  python3 - "$rollback_audit" <<'PY'
import json,sys,os,shutil
j=json.load(open(sys.argv[1]))
for item in j.get("rollback",[]):
  src=item["from"]; dst=item["to"]
  if os.path.exists(dst): shutil.rmtree(dst,ignore_errors=True)
  if os.path.exists(src): shutil.move(src,dst)
print("rollback replay complete")
PY
  log "rollback replayed from audit=$rollback_audit"
  exit 0
fi

manifest="${bundle%.tar.gz}.manifest.json"; manifest="${manifest%.tar.zst}.manifest.json"; checksum="${bundle%.tar.gz}.sha256"; checksum="${checksum%.tar.zst}.sha256"
[[ -f "$manifest" && -f "$checksum" ]] || { log "manifest/checksum missing"; exit 1; }
(cd "$(dirname "$bundle")" && sha256sum -c "$(basename "$checksum")")
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$manifest"

host="$(hostname -f 2>/dev/null || hostname)"; [[ -z "$confirm_host" || "$confirm_host" == "$host" ]] || { log "host mismatch"; exit 1; }
meta_schema="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("schema_version","unknown"))' "$manifest")"
cur_schema="unknown"; [[ -f /var/lib/proteus/state.json ]] && cur_schema="$(sed -n 's/.*"schema_version"[[:space:]]*:[[:space:]]*"\{0,1\}\([^",}]*\).*/\1/p' /var/lib/proteus/state.json | head -n1)"
[[ "$meta_schema" == "$cur_schema" || "$meta_schema" == "unknown" || "$cur_schema" == "unknown" || $force -eq 1 ]] || { log "schema mismatch"; exit 1; }

[[ $purge -eq 0 || $force -eq 1 ]] || { log "purge requires force"; exit 1; }
if [[ $purge -eq 1 && $auto_yes -ne 1 ]]; then token="PURGE-${host}-$(date -u +%Y%m%d%H%M)"; echo "Type $token"; read -r t; [[ "$t" == "$token" ]] || exit 1; fi

allowed=(etc/proteus var/lib/proteus usr/share/proteus/personas)
case "$only" in "") ;; config) allowed=(etc/proteus) ;; state) allowed=(var/lib/proteus) ;; personas) allowed=(usr/share/proteus/personas) ;; *) exit 2;; esac
validate(){ m="${1#./}"; [[ "$m" != /* && "$m" != *..* ]] || return 1; for p in "${allowed[@]}"; do [[ "$m" == "$p" || "$m" == "$p/"* ]] && return 0; done; return 1; }
while IFS= read -r m; do validate "$m" || { log "unexpected path $m"; exit 1; }; done < <(tar -tf "$bundle")

services=(proteus.service proteus-dispatcher.service proteus-events.service)
report=(); running=()
for s in "${services[@]}"; do st=$(systemctl is-active "$s" 2>/dev/null || echo unknown); report+=("$s:$st"); [[ "$st" == active ]] && running+=("$s"); done
[[ $plan -eq 1 ]] && { log "plan requested pre=${report[*]}"; tar -tf "$bundle"; exit 0; }

[[ $EUID -eq 0 ]] || { log "must run as root"; exit 1; }
((${#running[@]})) && systemctl stop "${running[@]}" || true

staging="$(mktemp -d)"; trap 'rm -rf "$staging"' EXIT
[[ "$bundle" == *.zst ]] && tar --zstd -xf "$bundle" -C "$staging" || tar -xzf "$bundle" -C "$staging"

ts2="$(date -u +%Y%m%d-%H%M%S)"; audit="$(dirname "$bundle")/run-$ts2.json"; rollback='[]'; actions='[]'
fail(){ python3 - "$audit" "$rollback" "$actions" "$manifest" "$bundle" "$host" <<'PY'
import json,sys
p,r,a,m,b,h=sys.argv[1:7]
meta=json.load(open(m))
json.dump({"status":"failed","host":h,"bundle":b,"manifest":meta,"rollback":json.loads(r),"actions":json.loads(a)},open(p,'w'))
PY
log "failed; audit=$audit"; exit 1; }

for p in "${allowed[@]}"; do src="/$p"; st="$staging/$p"; [[ -e "$st" ]] || continue; if [[ -e "$src" ]]; then b="${src}.pre-recovery.$ts2"; mv "$src" "$b" || fail; rollback=$(python3 - <<PY
import json; arr=json.loads('''$rollback'''); arr.append({"from":"$b","to":"$src"}); print(json.dumps(arr))
PY
); fi; [[ $purge -eq 1 ]] && rm -rf "$src"; mkdir -p "$(dirname "$src")"; cp -a "$st" "$src" || fail; done

for s in "${running[@]}"; do ok=0; for _ in 1 2 3; do systemctl start "$s" || true; timeout 10 bash -c "until systemctl is-active --quiet '$s'; do sleep 1; done" && { ok=1; break; }; done; [[ $ok -eq 1 ]] || fail; done
post=(); for s in "${services[@]}"; do post+=("$s:$(systemctl is-active "$s" 2>/dev/null || echo unknown)"); done

python3 - "$audit" "$manifest" "$bundle" "$host" "${report[*]}" "${post[*]}" "$rollback" <<'PY'
import json,sys
p,m,b,h,pre,post,r=sys.argv[1:8]
meta=json.load(open(m))
json.dump({"status":"ok","host":h,"bundle":b,"manifest":meta,"pre_state":pre.split(),'post_state':post.split(),"rollback":json.loads(r)},open(p,'w'))
PY
log "success audit=$audit"
[[ $json -eq 1 ]] && cat "$audit" || echo "Restore complete"
