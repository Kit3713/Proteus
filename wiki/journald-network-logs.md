Network daemons on a systemd Linux box log to journald by default. Those logs contain a high-resolution record of which networks you joined, which BSSIDs you saw, what your DHCP exchanges looked like, and (with default verbosity) every DNS query you resolved. This page covers what is logged, how to bound it, and what to assume about the journal's forensic surface.

For the broader threat model, read `proteus wiki threat-model` first. For a leak-by-leak inventory of network identifiers, `proteus wiki network-fingerprint-checklist`.

## What network daemons log to journald

A short tour of the daemons that touch the network stack and what each writes by default.

- **NetworkManager.** `journalctl -u NetworkManager`. Connection events (associate, deauth, IP acquisition), DHCP lease details, captive-portal probe results, BSSID transitions, signal strength snapshots, occasional debug detail. Default level is `INFO`.
- **wpa_supplicant.** `journalctl -u wpa_supplicant` (or under NetworkManager when NM owns the supplicant — common). EAP exchanges, association events, scan results. Default `INFO` is verbose; SSIDs and BSSIDs are first-class entries. Cross-ref `proteus wiki wpa-supplicant-hardening`.
- **systemd-networkd.** `journalctl -u systemd-networkd`. On hosts that use it instead of NM, similar event log to the above. Less common on desktops; common on servers.
- **systemd-resolved.** `journalctl -u systemd-resolved`. DNS resolution events, upstream changes. With `LLMNR=yes` or `MulticastDNS=yes`, also logs LAN-side resolution attempts. Cross-ref `proteus wiki dns`.
- **chronyd / systemd-timesyncd / ntpd.** `journalctl -u chronyd` etc. NTP step events, server selection, occasional clock corrections. Cross-ref `proteus wiki discovery` for NTP normalization.
- **Proteus itself.** `journalctl -t proteus`. One line per managed action with a stable identifier prefix; intentionally low-verbosity by default.

`journalctl -t proteus` is the syslog-tag-based filter for Proteus's own output regardless of which unit invoked it. Useful when a rotation happens via the dispatcher hook rather than the timer unit.

## Useful filters

`journalctl` is the right tool, not `cat /var/log/messages`. Filter aggressively or you will drown in noise.

```sh
# Network manager only, follow live
journalctl -u NetworkManager -f

# Last 200 lines from wpa_supplicant
journalctl -u wpa_supplicant -n 200

# Anything Proteus emitted today
journalctl -t proteus --since today

# Network events around a specific time window
journalctl --since "2026-04-12 14:00" --until "2026-04-12 15:30" -u NetworkManager

# Multiple units at once
journalctl -u NetworkManager -u wpa_supplicant -u systemd-resolved -n 500

# JSON for scripting
journalctl -u NetworkManager -o json --since "1 hour ago"

# This boot only
journalctl -u NetworkManager -b
```

`journalctl -p warning` filters to priority warning or higher — useful when you only care about errors. `journalctl --grep <pattern>` runs a regex against the message body. Both compose with the unit and tag filters.

## What gets logged that contains identifying info

The honest inventory. Every item below is something you might prefer not be in a long-term log on disk.

- **SSIDs.** Every network you join (and often every network you scan past) ends up in the NetworkManager and wpa_supplicant logs. Your saved-networks list is implicit in the join history. A casual reader of your journal can reconstruct your travel pattern from SSID transitions alone.
- **BSSIDs.** The MAC of each AP you associated with. Combined with the SSID, this identifies the specific access point — useful for distinguishing "the Marriott in Boston" from "the Marriott in Seattle" if both are named the same.
- **Roaming transitions.** When you walk between APs in the same ESS, both wpa_supplicant and NetworkManager log the BSSID change. A trace of your physical movement on the floor.
- **Signal strength snapshots.** Periodic RSSI readings tied to BSSID. Low-resolution location data over time.
- **DHCP exchanges.** Vendor class identifier, hostname (before suppression), DUID, the offered IP, DNS servers, lease duration. Even after Proteus suppresses the wire-side leaks, the journal records what was sent. Cross-ref `proteus wiki dhcp`.
- **EAP exchanges.** For 802.1X networks: outer identity (the username sent in cleartext), the EAP method, the realm. The inner identity is inside the TLS tunnel and not logged here, but the outer identity goes in the journal verbatim. Cross-ref `proteus wiki enterprise-wifi`.
- **DNS resolutions.** With systemd-resolved at default verbosity, each query is logged. This is the highest-resolution behavioral trace on the system — every domain you visit, every minute of the day. Sites visited, services used, app installations, update checks, the lot.
- **Captive portal probe results.** NetworkManager's `nm-check.gnome.org` (or equivalent) attempts and outcomes. Identifies which networks were captive and when you authed.
- **Bluetooth events.** `journalctl -u bluetooth` records pairing, connect, disconnect, and (with debug enabled) discovery events. Includes BD_ADDRs of paired devices.

The DNS line is the heaviest. A week of journal with default systemd-resolved verbosity is enough to characterize your daily routine, your work, your interests, and your social graph (every contact-management app, every chat app, every social network you use makes API calls that are logged by hostname).

## How to limit logging

The right knob is per-daemon. Lowering global journald verbosity affects everything; usually you want to bound the noisy ones individually.

### NetworkManager

```text
# /etc/NetworkManager/NetworkManager.conf or a drop-in under conf.d/

[logging]
level=WARN
```

NetworkManager's default is `INFO`. Setting `WARN` drops association noise, BSSID transitions, and routine probe results — they only appear if something actually fails. Use `ERR` to suppress further; use `DEBUG` only while actively troubleshooting.

NetworkManager also supports per-domain levels: `level=INFO,DOMAINS=DEVICE,WIFI:DEBUG` keeps default INFO globally and bumps Wi-Fi-specific logging to DEBUG. Inverse useful for muting one domain while leaving others.

### wpa_supplicant

```text
# /etc/wpa_supplicant/wpa_supplicant.conf

level_str=WARN
```

Same pattern as NetworkManager. Default is `INFO`; `WARN` suppresses the per-association EAP exchange, scan-result enumeration, and roaming transitions.

### systemd-resolved

```text
# /etc/systemd/resolved.conf or a drop-in under resolved.conf.d/

[Resolve]
LogLevel=warning
```

The big one. systemd-resolved at `info` logs every DNS resolution; at `warning` it only logs failures and configuration changes. The behavioral trace from DNS is the highest-volume identifying data on most Linux desktops; muting it has the largest privacy benefit.

Caveat: dropping the log level here makes "why did that domain not resolve" debugging require flipping it back on. Keep a comment in the file noting the original setting.

### journald itself

```text
# /etc/systemd/journald.conf or a drop-in

[Journal]
Storage=persistent
SystemMaxUse=500M
MaxRetentionSec=2week
```

Bound the disk footprint and the time horizon. `SystemMaxUse=500M` caps total journal size; `MaxRetentionSec=2week` discards entries older than two weeks. These are blunt — they affect every unit, not just the network daemons — but they cap the worst-case forensic surface.

`Storage=volatile` keeps journal in `/run` only (cleared on reboot). Useful on high-threat-tier setups; loses every record across reboots, including non-network logs you might actually want.

### The tradeoff

Lower log level means harder troubleshooting. The standard pattern: live with bounded verbosity, bump to debug temporarily when actively diagnosing, drop back when done.

```sh
# Bump for debugging
sudo nmcli general logging level DEBUG domains ALL

# Reset to defaults
sudo nmcli general logging level KEEP domains DEFAULT
```

Same idea for wpa_supplicant via `wpa_cli -i <iface> log_level DEBUG` (runtime, doesn't survive restart). For systemd-resolved, edit the conf file and `systemctl restart systemd-resolved`.

## Forensic awareness

What survives a poweroff, what an adversary with disk access sees.

- **Persistent journals (`Storage=persistent`)** — the default on most distros. Journal lives at `/var/log/journal/<machine-id>/`. Survives reboots. An attacker with physical access (or root after compromise) reads everything.
- **Volatile journals (`Storage=volatile`)** — journal in `/run/log/journal/`, tmpfs-backed. Cleared on reboot. Reduces the persistent footprint to "current uptime" only.
- **Encrypted disks.** The journal is on the disk, so it inherits whatever encryption the disk has. LUKS-on-root means the journal is unreadable without the passphrase. LUKS does nothing while the system is running and unlocked — a live malicious process or a live-imaging adversary still sees clear journal entries.
- **Journal sealing (FSS).** Forward Secure Sealing (`journalctl --setup-keys`) detects post-hoc tampering of the journal. Useful for integrity, not confidentiality. An adversary still reads everything; they just cannot silently edit it.
- **Suspend-resume.** The journal stays on disk across suspend. The keys are in RAM during suspend; an evil-maid or cold-boot attack against an unattended laptop sees both the LUKS keys and the journal.

A practical rule: assume the journal is readable by anyone with root or with physical access plus your LUKS passphrase. If the journal contains data you do not want correlated with you, either prune it (`journalctl --vacuum-time=1d`), bound retention (`MaxRetentionSec`), or limit verbosity at the daemon level so the data was never written.

## Pruning the journal

Manual cleanup hatches, when the bounded retention is not aggressive enough.

```sh
# Drop everything older than 1 day
sudo journalctl --vacuum-time=1d

# Drop everything past 100 MB total
sudo journalctl --vacuum-size=100M

# Drop entries older than the most recent 5 boots
sudo journalctl --vacuum-files=5

# Hard-rotate the active journal (forces a new file, old becomes archive)
sudo journalctl --rotate
```

`--vacuum-*` operations only act on archived journal files (the inactive ones). To force the current journal into archive first, run `--rotate` then `--vacuum-time`.

## Verifying

A pre-commit check that your reductions actually took effect.

```sh
# Inspect current effective NM log level
nmcli general logging

# Inspect resolved log level (root)
sudo systemd-analyze cat-config systemd/resolved.conf | grep -i log

# Watch for noise in the next minute
journalctl -u NetworkManager -u wpa_supplicant -u systemd-resolved -f -n 0
```

Reconnect to a network and watch what scrolls. With `WARN` levels set everywhere, you should see one or two lines for the connect — not the dozen-plus that `INFO` produces.

## Cross-refs

- `proteus wiki troubleshooting` — when a reduced log level makes diagnosis harder, the recovery path
- `proteus wiki hostile-environments` — what to do with the journal before traveling and after returning
- `proteus wiki dhcp` — what Proteus suppresses on the wire (the journal still records the suppressed-then-not-sent state)
- `proteus wiki dns` — the systemd-resolved configuration and the detect-and-defer rule
- `proteus wiki wpa-supplicant-hardening` — the supplicant's own log knobs
- `proteus wiki network-fingerprint-checklist` — leak surfaces, journald is the on-disk shadow of every wire-side rotation
- `proteus wiki concepts` — managed-file headers and idempotency context

External:

- journald.conf(5), systemd-journald.service(8), journalctl(1) — the canonical references
- NetworkManager.conf(5) — `[logging]` section details
- wpa_supplicant.conf(5) — `level_str` and related controls
