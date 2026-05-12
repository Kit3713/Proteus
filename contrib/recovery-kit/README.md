# Proteus Recovery Kit (hardened sidecar)

Backup/restore helper scripts for fast operational recovery iteration.

## Implemented hardening features

- Integrity: generates `.sha256` for bundle + manifest and verifies before restore.
- Optional signature verification: if `<checksum>.sig` exists, restore verifies with `gpg` or `minisign`.
- Metadata/versioning: manifest stores host, OS, Proteus version, schema version, timestamp, compression mode.
- Compatibility gate: schema mismatch requires `--force`.
- Atomic-ish restore: extract to staging directory first, then copy into place with rollback on failure.
- Service-state aware: only restarts services that were active before restore and checks health after start.
- Explicit manifest: `manifest.json` includes paths and per-file bytes/checksums where available.
- Retention policy: `backup.sh --keep N` prunes old backups/manifests/checksums.
- Safer destructive controls: `--purge-targets` and timestamped confirmation token.
- Preflight checks: command availability, writable destination, root requirement for restore, free-space check.
- Nice-to-have: `--only=<config|state|personas>`, `--plan`, gzip/zstd compression, `--json` output.

## Backup

```bash
sudo ./backup.sh /tmp/proteus-backups --keep 10 --compression zstd --json
```

## Restore

```bash
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-YYYYmmdd-HHMMSS.tar.zst --yes --force
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-YYYYmmdd-HHMMSS.tar.gz --plan --only=config
```

## Paths covered

- `/etc/proteus`
- `/var/lib/proteus`
- `/usr/share/proteus/personas`
