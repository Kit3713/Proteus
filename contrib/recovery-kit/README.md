# Proteus Recovery Kit (sidecar)

Recovery sidecar scripts for fast backup/restore iteration while roadmap work continues.

## Included scripts

- `backup.sh`: create timestamped `.tar.gz` bundles.
- `restore.sh`: aggressive restore with optional service bounce and optional pre-existing data preservation.

## Default restore behavior

`restore.sh` is intentionally disruptive by default to reduce leftover state conflicts:

- validates tar members against Proteus path allowlist
- stops common Proteus services when `systemctl` is available
- removes existing target directories before extraction
- extracts backup to `/`
- attempts to restart services

Use `--keep-existing` if you do not want pre-removal behavior.
Use `--no-service-bounce` to skip stop/start attempts.

## Paths covered

- `/etc/proteus`
- `/var/lib/proteus`
- `/usr/share/proteus/personas` (if present)

## Usage

```bash
sudo ./backup.sh /tmp/proteus-backups
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-YYYYmmdd-HHMMSS.tar.gz --yes
sudo ./restore.sh /tmp/proteus-backups/proteus-backup-YYYYmmdd-HHMMSS.tar.gz --dry-run --yes
```
