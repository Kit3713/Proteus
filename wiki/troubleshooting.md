Common failure modes and concrete recovery commands. Read top-to-bottom or jump to your symptom. Cross-refs at the bottom.

## First step for any "something broke"

Run `proteus doctor`. It runs a battery of read-only checks (kernel, systemd, daemons, files, detected DNS/NTP competitors, Proteus state) and prints `ok / warn / fail / skip` per check with remediation pointers. It works without root; some checks degrade to `skip` instead of `fail` when not root. JSON output is available for wrappers:

```sh
proteus doctor                # human-readable
proteus doctor --json         # machine-readable
proteus doctor --quick        # fast subset (skip filesystem walks)
proteus -v doctor             # extra detail per check
```

`proteus doctor` exits `0` when no checks fail, `1` if any check fails. See `proteus wiki doctor` for the full list of checks and how to interpret each result.

## "I can't connect to my network anymore"

You just rotated and the link is dead. Start gentle, escalate.

- Try a revert: `sudo proteus revert --yes`. Failing that, restart NetworkManager: `sudo systemctl restart NetworkManager`.
- Check whether the current MAC is wedged: `proteus status --json | jq .interfaces` and `proteus current --json | jq .interfaces[].mac`.
- Captive portal in path? See `proteus wiki captive-portals` for the loop fix. The portal detector should classify it; if it didn't, check `proteus status --json | jq .portal`.
- Verify the rotated MAC didn't collide with the gateway. This should never happen — Proteus checks the ARP table before assigning — but it's the first thing to rule out: `arp -a` then compare with `proteus current --json`.
- Recovery hatch: `sudo proteus reset --yes` clears your config back to defaults and re-applies. It does not touch the original-MAC cache, so this is safe.

## "My printer stopped being discovered"

Almost always the discovery blocks. Both are opt-in for a reason.

- Did you set `discovery.wsd_block = true` or `discovery.ssdp_block = true`? Set them back to `false` in `/etc/proteus/config.toml`.
- Re-apply: `sudo proteus apply`.
- WS-Discovery printers (most modern network printers) need WSD. SSDP carries UPnP discovery for some printers and scanners.
- Cross-ref `proteus wiki discovery` for the WSD/SSDP details and the workaround patterns.

## "KDE Connect doesn't see my phone"

Same family of problem.

- `discovery.ssdp_block = true` blocks KDE Connect's SSDP-based discovery. Default is `false` — if you set it true, set it back.
- `sudo proteus apply` after editing.
- KDE Connect also uses mDNS in newer releases; if you've enabled mDNS responder blocking for some reason, that'll bite too. Check `discovery.mdns_responder` in your config.

## "My corporate Wi-Fi rejects me"

Enterprise Wi-Fi is fussy. The opt-in 802.1X knob is the usual cause.

- Did you enable `enterprise_wifi.anonymous_outer_identity`? Some Microsoft NPS configs reject mismatched outer/inner identity and silently drop you.
- Disable per-connection: `sudo proteus enterprise-wifi disable --connection "MyOrgWiFi" --yes`.
- Look for the EAP failure: `journalctl -u NetworkManager -n 100` and search for `EAP-Failure` or `auth failed`.
- If your org also pins MAC for 802.1X cert binding, pin Proteus to a stable MAC for that connection: `sudo proteus pin --connection "MyOrgWiFi" --yes`.

## "My DHCP lease keeps renewing too often"

Aggressive rotation triggers a renewal each time.

- Check your interval: `proteus show-config | grep rotation_interval`. Anything under `30m` means a fresh lease every half hour.
- Some networks throttle this and may temporarily blacklist a MAC that asks too often.
- Increase to the default `"2h"` or longer. Edit `/etc/proteus/config.toml`:
  ```toml
  [mac]
  rotation_interval = "2h"
  ```
- Or disable rotation for that environment: `[mac] enabled = false`, then `sudo proteus apply`.

## "My BlueZ paired device disconnected"

Bluetooth pairing is stickier than it looks.

- Did you change `bluetooth.ble_rpa`? Paired devices share an Identity Resolving Key (IRK). Proteus does not rotate IRKs — but switching RPA modes can hiccup an in-flight pairing.
- Re-pair if needed. The remote device's bond should still recognize you once it sees the IRK.
- Cross-ref `proteus wiki bluetooth` for the RPA/IRK discussion.

## "My machine boots but no network for ~30s"

`proteus-boot.service` runs once on boot to re-apply state. It can stall initial network setup if NetworkManager is slow to come up.

- Check it: `systemctl status proteus-boot`.
- If it's hanging on a DBus call: `journalctl -u NetworkManager -b` and `journalctl -u proteus-boot -b`.
- Disable boot integration entirely: `sudo systemctl disable --now proteus-boot.service`. You'll lose first-boot re-application but periodic rotation still works via the timer.

## "`proteus status` says feature X is `failed`"

Look at the structured reason first.

- `proteus status --json | jq '.features.X'` shows the per-feature state and the failure reason.
- DBus failures (NetworkManager, BlueZ): the daemon may be down, or you may lack permission. Mutating commands need root — `sudo` is mandatory.
- Sysctl failures: `journalctl -t proteus -n 50` shows the underlying syscall error.
- nft rule failures: ensure the `nftables` package is installed; check for partial state with `sudo nft list ruleset` (look for the `proteus` table).
- If it's a transient DBus glitch, `sudo proteus apply` again often fixes it. Idempotency is an invariant; running apply twice is safe.

## "I want to know what Proteus changed"

- `proteus diff` — config vs defaults vs live state, with drift flagged on managed files via the SHA in their headers.
- `cat /var/lib/proteus/state.json | jq .` — see what's been captured (original MACs, original hostname, history).
- `find /etc -name '*proteus*'` — see every drop-in Proteus has written.
- `sudo nft list ruleset` — see the firewall rules in the `proteus` table.
- `journalctl -t proteus -n 200 --no-pager` — recent log output.

## "I want to fully remove Proteus"

- One-shot: `sudo proteus uninstall --purge --yes`. Runs revert, removes the binary, clears `/etc/proteus/` and `/var/lib/proteus/`.
- Manually:
  1. `sudo proteus revert --yes` to undo every applied change.
  2. `sudo rm /usr/local/bin/proteus` to remove the binary.
  3. `sudo rm -rf /etc/proteus/ /var/lib/proteus/` to clear config and state.
  4. `sudo systemctl disable --now proteus-rotate.timer proteus-check.timer proteus-boot.service` if any units are still around.
- Cross-ref `proteus wiki uninstall` for the full procedure and what each step touches.

## "My DNS broke after enabling Proteus"

Probably not the ECS-strip knob — it has a hard guard that defers to dnscrypt-proxy, Pi-hole, AdGuard Home, or any custom resolver.

- Check the guard fired: `proteus status --json | jq '.features.dns'` should show `skipped (detected <tool>)` if you have one of those installed.
- If the guard didn't fire, look for the Proteus drop-in: `ls /etc/systemd/resolved.conf.d/`. Remove the suspect file:
  ```sh
  sudo rm /etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf
  sudo systemctl restart systemd-resolved
  ```
- Cross-ref `proteus wiki dns` for the decision tree.

## "I see `not yet implemented` for command X"

The CLI surface is fully wired today. If you see `not yet implemented`, the most common cause is the `[backend] driver = "networkd"` or `"raw"` selection: the NM-driven write paths have full coverage; networkd / raw are graceful-degrade stubs for some mutating paths. Pin `[backend] driver = "nm"` if NM is present, or check `proteus doctor` for the backend matrix.

## "Permission denied"

- All mutating commands need root: `apply`, `rotate`, `revert`, `pin`, `unpin`, `reset`, `uninstall`. Prefix with `sudo`.
- Read commands work without root for any file the user can read. They degrade quietly when files aren't readable rather than failing loudly.

## "How do I run Proteus without systemd?"

Limited support, via the init-system abstraction. Proteus knows about `Systemd`, `Openrc`, `Runit`, and `Sysvinit`.

- Read commands work without systemd. `proteus status`, `proteus current`, `proteus original` will run.
- For OpenRC / runit / sysvinit hosts, `proteus doctor` reports the detected init and which features depend on systemd-specific paths (drop-ins under `/etc/systemd/...`, journald-only logging, etc.).
- See `proteus wiki distro-support` for the matrix.

## "Proteus says my config has drift"

Manual edits to a managed file get flagged loudly. This is a feature, not a bug.

- Run `proteus diff` to see the path, expected SHA, and current SHA. The SHA is an edit-detection signal, not an integrity guarantee against an attacker with write access.
- Decide: re-apply Proteus's version with `sudo proteus apply`, or accept the local edit and update the header SHA, or back the whole thing out with `sudo proteus revert`.
- The header on every managed file looks like `# managed by proteus — do not edit` followed by `# sha256: <hex>`. If you removed those headers, Proteus will treat the file as foreign and refuse to overwrite it.

## "Probes keep triggering rotations on a flaky link"

The probe quorum exists for exactly this — but if your network is bad enough, you can still trip it.

- Check the probe state: `proteus status --json | jq '.probes'`.
- Recent rotations: `journalctl -u proteus-check -n 100`.
- Loosen the quorum or extend the cooldown in `[probes]`. See `proteus wiki probes`.
- Or pin the interface for that environment: `sudo proteus pin --iface <iface> --yes`. Pinned interfaces are skipped by both schedule and probe-driven rotation.

## Logs and diagnostics

When in doubt, look at the journal.

- Recent Proteus output: `journalctl -t proteus -n 200 --no-pager`.
- Rotate timer service: `journalctl -u proteus-rotate -n 100`.
- Probe-check service: `journalctl -u proteus-check -n 100`.
- Boot oneshot: `journalctl -u proteus-boot -n 100`.
- Run any command with `-vv` for debug: `proteus -vv status`. Verbose output goes to stderr and (under systemd) to journald.
- Status in JSON for programmatic inspection: `proteus status --json | jq .`. Same data the CLI uses.

## Backend × init-system × persona symptom matrix

Quick-reference for "X broke; what's the likely culprit?" Read across the row, then `proteus wiki <linked-page>` for depth.

### By backend

| Backend | Symptom | Most likely cause | First action |
|---|---|---|---|
| `nm` | `proteus rotate` skipped: `pinned to ...` | Profile pin from prior `proteus pin` | `proteus unpin <iface-or-conn>` |
| `nm` | `set_cloned_mac failed: ... NoSecrets` | NM secrets-merge contention (rare; see #207) | Re-run; `proteus -vv rotate` to surface the section |
| `nm` | `proteus dhcp renew` exits "Reapply rejected" | NM ≤1.0 doesn't support `Reapply`; Proteus falls back to Disconnect+Activate | Expected; the lease still rotates |
| `networkd` | some mutating commands bail `not yet implemented` | Selected backend write paths are stubs while NM is the lead | Pin `[backend] driver = "nm"` if NM is present, else file an issue with the missing path |
| `raw` | same | same | same |
| any | doctor `Backend: no backend available` | No NM, no networkd, no `ip` on `$PATH` | `apt install iproute2` / `dnf install iproute` / equivalent |
| any | rotate exits 75 | State-lock contention (issue #211) | Wait (timer/dispatcher overlap); raise `PROTEUS_LOCK_TIMEOUT_MS` if persistent |

### By init system

| Init | Symptom | Cause | Action |
|---|---|---|---|
| systemd | `proteus-rotate.timer` not firing | Unit not enabled | `systemctl enable --now proteus-rotate.timer`; verify with `proteus timer status` |
| systemd | rotate fires but reports "no factory MAC captured for ..." | #208 — driver lacks phy80211 + `ETHTOOL_GPERMADDR` | Expected; revert is a no-op for that iface — operator should record the original MAC manually before any cloning if revert matters |
| systemd | hostname doesn't change | `hostnamed` not active or polkit blocked | `systemctl status systemd-hostnamed`; `proteus -vv hostname rotate` to surface DBus error |
| OpenRC | `rc-service proteus start` fails with "binary not found" | Alpine package paths differ from `/usr/bin/proteus` | Verify the APKBUILD installs to `/usr/bin/proteus`; symlink if a packaging variant differs |
| OpenRC | periodic rotate not firing | `crond` provider not running | `rc-service crond start && rc-update add crond default` |
| Runit | service flapping | `run` script's `sleep` argument too short | Edit `/etc/sv/proteus/run` to widen the supervised loop |
| sysvinit | LSB script reports degraded | Old `/etc/init.d` pattern doesn't match systemd's expectations | `proteus -vv apply` to bypass the init wrapper while debugging |

### By persona

| Persona kind | Symptom | Cause | Action |
|---|---|---|---|
| `iphone-15` | `nmap -O` still says Linux | Persona shapes L2/L3/L4 + DHCP + mDNS, **not** TLS / payload — see `wiki/personas` | Verify the persona via `tcpdump 'port 67 or port 68'` for the DHCP fingerprint instead |
| any stealth | hostname doesn't render the persona template | `[persona] active = "..."` not set in config | `proteus persona use <id> --yes` |
| any stealth | DHCP option 60 missing in lease | The connection's `dhcp-vendor-class-identifier` is being suppressed by `[dhcp] suppress_vendor_class = true` (per-knob beats persona) | Either set `suppress_vendor_class = false` for that profile or accept persona is opt-in over your stricter config |
| randomizer | rotation cadence shorter than expected | Per-SSID override (`[per_ssid."<ssid>"].rotate_interval`) is winning | `proteus ssid show <ssid>` to see the resolved policy + source trace |
| custom user | `proteus persona use my-foo` errors `"persona 'my-foo' not found"` | File at `/etc/proteus/personas/my-foo.toml` missing or not validated | `proteus persona validate /etc/proteus/personas/my-foo.toml` for field-level errors |

### By exit code

| Code | Meaning | First action |
|---|---|---|
| 0 | success | — |
| 1 | generic error | read stderr, then `proteus -vv <last-cmd>` |
| 64 | not implemented (selected backend doesn't drive this path) | pin `[backend] driver = "nm"` if NM is present |
| 65 | config error or `--yes` missing | re-read the message; the helper text names the wiki page |
| 66 | permission error (not root) | `sudo proteus ...` |
| 70 | system not supported | doctor matrix; install the missing daemon |
| 75 | lock contention | retry; `PROTEUS_LOCK_TIMEOUT_MS=10000` if persistent |

## Cross-refs

- `proteus wiki doctor` — what each `proteus doctor` check does and how to read the output.
- `proteus wiki cli` — full CLI reference, exit codes, JSON schemas.
- `proteus wiki config` — every config knob with default and risks.
- `proteus wiki uninstall` — full removal procedure.
- `proteus wiki discovery` — mDNS, LLMNR, NetBIOS, SSDP, WSD details.
- `proteus wiki dns` — the one DNS knob and its hard guard.
- `proteus wiki bluetooth` — adapter alias, BLE RPA, IRK discussion.
- `proteus wiki captive-portals` — portal detection, classification, policies.
- Per-feature wiki pages for the feature you're debugging.
