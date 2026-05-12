#!/usr/bin/env bash
set -euo pipefail

usage(){ cat >&2 <<USAGE
Usage: $0 [out_dir] [--keep N] [--compression gzip|zstd] [--encrypt gpg|age] [--output-name NAME] [--exclude GLOB] [--json] [--verify <bundle>] [--log-file PATH]
USAGE
}

out_dir="."; keep=""; compression="gzip"; json=0; encrypt=""; output_name=""; verify_bundle=""; log_file=""; excludes=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep) keep="${2:-}"; shift 2 ;;
    --compression) compression="${2:-}"; shift 2 ;;
    --encrypt) encrypt="${2:-}"; shift 2 ;;
    --output-name) output_name="${2:-}"; shift 2 ;;
    --exclude) excludes+=("${2:-}"); shift 2 ;;
    --json) json=1; shift ;;
    --verify) verify_bundle="${2:-}"; shift 2 ;;
    --log-file) log_file="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) out_dir="$1"; shift ;;
  esac
done

ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
init_log(){
  if [[ -z "$log_file" ]]; then
    if [[ -d /var/log ]]; then mkdir -p /var/log/proteus-recovery-kit 2>/dev/null || true; fi
    log_file="${log_file:-/tmp/proteus-recovery-kit.log}"
    [[ -w /var/log/proteus-recovery-kit || ! -d /var/log/proteus-recovery-kit ]] || log_file="/var/log/proteus-recovery-kit/backup.log"
  fi
}
log(){ msg="$(ts) backup $*"; echo "$msg" | tee -a "$log_file" >&2; command -v logger >/dev/null && logger -t proteus-recovery-kit "$msg" || true; }
init_log

lock="/tmp/proteus-recovery.lock"
exec 9>"$lock"; command -v flock >/dev/null && flock -n 9 || { log "lock busy"; exit 40; }
log "start out_dir=$out_dir compression=$compression keep=${keep:-none}"

if [[ -n "$verify_bundle" ]]; then
  manf="${verify_bundle%.tar.gz}.manifest.json"; manf="${manf%.tar.zst}.manifest.json"; sum="${verify_bundle%.tar.gz}.sha256"; sum="${sum%.tar.zst}.sha256"
  (cd "$(dirname "$verify_bundle")" && sha256sum -c "$(basename "$sum")")
  python3 -c 'import json,sys; json.load(open(sys.argv[1])); print("manifest-ok")' "$manf"
  log "verify success bundle=$verify_bundle"
  exit 0
fi

for c in tar sha256sum python3 stat find; do command -v "$c" >/dev/null || { log "$c required"; exit 1; }; done
[[ -d "$out_dir" ]] || mkdir -p "$out_dir"

case "$compression" in gzip) ext="tar.gz"; tar_args=(-czf) ;; zstd) command -v zstd >/dev/null || exit 1; ext="tar.zst"; tar_args=(--zstd -cf) ;; *) log "bad compression"; exit 2;; esac

name_base="${output_name:-proteus-backup-$(date -u +%Y%m%d-%H%M%S)}"
bundle="$out_dir/$name_base.$ext"; manifest="$out_dir/$name_base.manifest.json"; checksum="$out_dir/$name_base.sha256"

paths=(etc/proteus var/lib/proteus usr/share/proteus/personas)
existing=(); for p in "${paths[@]}"; do [[ -e "/$p" ]] && existing+=("$p"); done
[[ ${#existing[@]} -gt 0 ]] || { log "no source paths"; exit 3; }

exclude_args=(); for ex in "${excludes[@]}"; do exclude_args+=("--exclude=$ex"); done
tar -C / --numeric-owner --owner=0 --group=0 "${exclude_args[@]}" "${tar_args[@]}" "$bundle" "${existing[@]}"
log "archive created bundle=$bundle"

python3 - "$manifest" "$bundle" "$compression" <<'PY'
import json,os,sys,hashlib,platform,time
manifest,bundle,compression=sys.argv[1:4]
roots=["/etc/proteus","/var/lib/proteus","/usr/share/proteus/personas"]
files=[]
for r in roots:
    if not os.path.exists(r):
        continue
    for dp,_,fns in os.walk(r):
        for fn in sorted(fns):
            p=os.path.join(dp,fn)
            st=os.lstat(p)
            h=hashlib.sha256()
            with open(p,'rb') as f:
                for c in iter(lambda:f.read(1024*1024),b''): h.update(c)
            files.append({"path":p,"bytes":st.st_size,"sha256":h.hexdigest(),"mode":oct(st.st_mode & 0o777),"uid":st.st_uid,"gid":st.st_gid,"mtime":int(st.st_mtime)})
meta={"timestamp_utc":time.strftime('%Y%m%d-%H%M%S',time.gmtime()),"host":platform.node(),"os":platform.platform(),"compression":compression,"bundle":os.path.basename(bundle),"paths":roots,"files":files}
with open(manifest,'w') as f: json.dump(meta,f,sort_keys=True,separators=(',',':'))
PY

(cd "$out_dir" && sha256sum "$(basename "$bundle")" "$(basename "$manifest")" > "$(basename "$checksum")")
log "manifest+checksum written manifest=$manifest checksum=$checksum"

if [[ -n "$encrypt" ]]; then
  case "$encrypt" in
    gpg) command -v gpg >/dev/null || exit 1; gpg --batch --yes -c "$bundle"; gpg --batch --yes -c "$manifest"; log "gpg encryption complete" ;;
    age) command -v age >/dev/null || exit 1; [[ -n "${AGE_RECIPIENT:-}" ]] || { log "AGE_RECIPIENT missing"; exit 1; }; age -r "$AGE_RECIPIENT" -o "$bundle.age" "$bundle"; age -r "$AGE_RECIPIENT" -o "$manifest.age" "$manifest"; log "age encryption complete" ;;
    *) log "bad encrypt value"; exit 2;;
  esac
fi

if [[ -n "$keep" ]]; then ls -1t "$out_dir"/proteus-backup-*.* 2>/dev/null | tail -n +$((keep+1)) | xargs -r rm -f; log "prune complete keep=$keep"; fi

[[ $json -eq 1 ]] && printf '{"bundle":"%s","manifest":"%s","checksum":"%s","log":"%s"}\n' "$bundle" "$manifest" "$checksum" "$log_file" || echo "Created backup: $bundle"
log "success"
