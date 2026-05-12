# Proteus Recovery Kit (visibility + retractability)

## Visibility/reporting

- Every run writes timestamped log lines to file and syslog (`logger -t proteus-recovery-kit`) when available.
- Default logs:
  - backup: `/var/log/proteus-recovery-kit/backup.log` (fallback `/tmp/proteus-recovery-kit.log`)
  - restore: `/var/log/proteus-recovery-kit/restore.log` (fallback `/tmp/proteus-recovery-kit.log`)
- Override log destination with `--log-file <path>`.
- Restore emits structured audit artifacts: `run-<timestamp>.json` next to the backup bundle.

## Retractability

- Restore stores rollback mappings in the audit JSON.
- Replay rollback with:
  - `restore.sh <bundle> --rollback-from-audit=<run-file.json>`

## Safety/operations features

- Canonical JSON manifest + strict JSON parse.
- Recursive per-file metadata coverage.
- Transaction lock (`flock`) to prevent concurrent runs.
- Service pre-state/post-state reporting and restart retries with timeout checks.
- `--purge-targets` requires `--force` and explicit host+timestamp token (unless `--yes`).
- Optional encryption-at-rest (`--encrypt gpg|age`).
- `backup.sh --verify <bundle>` preflight verification mode.

## Examples

```bash
sudo ./backup.sh /tmp/proteus-backups --json
sudo ./backup.sh /tmp/proteus-backups --log-file /var/log/proteus-recovery-kit/custom-backup.log
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-20260512-120000.tar.gz --force --purge-targets --confirm-host=$(hostname -f)
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-20260512-120000.tar.gz --rollback-from-audit=/tmp/proteus-backups/run-20260512-120500.json
```
