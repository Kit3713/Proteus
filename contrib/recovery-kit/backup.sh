#!/usr/bin/env bash
set -euo pipefail

usage(){ cat >&2 <<USAGE
Usage: $0 [out_dir] [--keep N] [--compression gzip|zstd] [--encrypt gpg|age] [--output-name NAME] [--exclude GLOB] [--json] [--verify <bundle>]
USAGE
}

out_dir="."; keep=""; compression="gzip"; json=0; encrypt=""; output_name=""; verify_bundle=""; excludes=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep) keep="${2:-}"; shift 2 ;;
    --compression) compression="${2:-}"; shift 2 ;;
    --encrypt) encrypt="${2:-}"; shift 2 ;;
    --output-name) output_name="${2:-}"; shift 2 ;;
    --exclude) excludes+=("${2:-}"); shift 2 ;;
    --json) json=1; shift ;;
    --verify) verify_bundle="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) out_dir="$1"; shift ;;
  esac
done

lock="/tmp/proteus-recovery.lock"
exec 9>"$lock"; command -v flock >/dev/null && flock -n 9 || { echo "Recovery lock is busy" >&2; exit 40; }

if [[ -n "$verify_bundle" ]]; then
  manf="${verify_bundle%.tar.gz}.manifest.json"; manf="${manf%.tar.zst}.manifest.json"; sum="${verify_bundle%.tar.gz}.sha256"; sum="${sum%.tar.zst}.sha256"
  (cd "$(dirname "$verify_bundle")" && sha256sum -c "$(basename "$sum")")
  python3 -c 'import json,sys; json.load(open(sys.argv[1])); print("manifest-ok")' "$manf"
  exit 0
fi

for c in tar sha256sum python3 stat find; do command -v "$c" >/dev/null || { echo "$c required" >&2; exit 1; }; done
[[ -d "$out_dir" ]] || mkdir -p "$out_dir"

case "$compression" in gzip) ext="tar.gz"; tar_args=(-czf) ;; zstd) command -v zstd >/dev/null || exit 1; ext="tar.zst"; tar_args=(--zstd -cf) ;; *) exit 2;; esac

name_base="${output_name:-proteus-backup-$(date -u +%Y%m%d-%H%M%S)}"
bundle="$out_dir/$name_base.$ext"; manifest="$out_dir/$name_base.manifest.json"; checksum="$out_dir/$name_base.sha256"

paths=(etc/proteus var/lib/proteus usr/share/proteus/personas)
existing=(); for p in "${paths[@]}"; do [[ -e "/$p" ]] && existing+=("$p"); done
[[ ${#existing[@]} -gt 0 ]] || { echo "no paths" >&2; exit 3; }

exclude_args=(); for ex in "${excludes[@]}"; do exclude_args+=("--exclude=$ex"); done
tar -C / --numeric-owner --owner=0 --group=0 "${exclude_args[@]}" "${tar_args[@]}" "$bundle" "${existing[@]}"

python3 - "$manifest" "$bundle" "$compression" <<'PY'
import json,os,sys,hashlib,platform,time,subprocess
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
with open(manifest,'w') as f:
    json.dump(meta,f,sort_keys=True,separators=(',',':'))
PY

(cd "$out_dir" && sha256sum "$(basename "$bundle")" "$(basename "$manifest")" > "$(basename "$checksum")")

if [[ -n "$encrypt" ]]; then
  case "$encrypt" in
    gpg) command -v gpg >/dev/null || exit 1; gpg --batch --yes -c "$bundle"; gpg --batch --yes -c "$manifest" ;;
    age) command -v age >/dev/null || exit 1; [[ -n "${AGE_RECIPIENT:-}" ]] || { echo "set AGE_RECIPIENT" >&2; exit 1; }; age -r "$AGE_RECIPIENT" -o "$bundle.age" "$bundle"; age -r "$AGE_RECIPIENT" -o "$manifest.age" "$manifest" ;;
    *) echo bad encrypt >&2; exit 2;;
  esac
fi

if [[ -n "$keep" ]]; then ls -1t "$out_dir"/proteus-backup-*.* 2>/dev/null | tail -n +$((keep+1)) | xargs -r rm -f; fi
[[ $json -eq 1 ]] && printf '{"bundle":"%s","manifest":"%s","checksum":"%s"}\n' "$bundle" "$manifest" "$checksum" || echo "Created backup: $bundle"
