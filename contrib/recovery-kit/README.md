# Proteus Recovery Kit (canonical + transactional)

## Features implemented

- Canonical JSON manifest via Python `json.dump(..., sort_keys=True, separators=(',',':'))`.
- Strict JSON parsing on restore verify path.
- Recursive per-file manifest coverage (sha256, bytes, mode, uid, gid, mtime).
- Transactional lock using `flock` on `/tmp/proteus-recovery.lock`.
- Service orchestration hardening with retries/timeouts and pre/post state reporting.
- Safer purge behavior: `--purge-targets` requires `--force`; manual token includes hostname + timestamp.
- Structured restore audit artifact: `run-<timestamp>.json`.
- Optional encryption-at-rest: `--encrypt gpg|age` for bundle + manifest.
- `backup.sh --verify <bundle>` helper mode.
- `--exclude` patterns and `--output-name` template support.
- `--json` output for CI.

## Exit codes (machine-readable)

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | runtime/precondition failure |
| 2 | invalid CLI arguments |
| 3 | no source paths found for backup |
| 40 | transactional lock busy |

## Examples

```bash
sudo ./backup.sh /tmp/proteus-backups --keep 8 --compression zstd --output-name nightly --exclude '*.cache' --json
sudo ./backup.sh --verify /tmp/proteus-backups/nightly.tar.zst
sudo ./restore.sh /tmp/proteus-backups/nightly.tar.zst --force --purge-targets --confirm-host=$(hostname -f)
sudo ./restore.sh /tmp/proteus-backups/nightly.tar.zst --plan --only=config
```
