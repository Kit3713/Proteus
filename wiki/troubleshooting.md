Common failure modes and concrete recovery commands. Read top-to-bottom or jump to your symptom. Cross-refs at the bottom.

## "I can't connect to my network anymore"

You just rotated and the link is dead. Start gentle, escalate.

- Try a revert: `sudo proteus revert --iface <iface>` (lands in phase G). Failing that, restart NetworkManager: `sudo systemctl restart NetworkManager`.
- Check whether the current MAC is wedged: `proteus status --json | jq .interfaces` and `proteus current --json | jq .interfaces[].mac`.
- Captive portal in path? See `proteus wiki captive-portals` for the loop fix. The portal detector should classify it; if it didn't, check `proteus status --json | jq .portal`.
- Verify the rotated MAC didn't collide with the gateway. This should never happen — Proteus checks the ARP table before assigning — but it's the first thing to rule out: `arp -a` then compare with `proteus current --json`.
- Recovery hatch: `sudo proteus reset` (phase G) clears your config back to defaults and re-applies. It does not touch the original-MAC cache, so this is safe.

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
- Disable per-connection: `sudo proteus enterprise-wifi disable --connection "MyOrgWiFi"` (phase D command). Until that lands, edit `/etc/proteus/config.toml` and run `sudo proteus apply`.
- Look for the EAP failure: `journalctl -u NetworkManager -n 100` and search for `EAP-Failure` or `auth failed`.
- If your org also pins MAC for 802.1X cert binding, pin Proteus to a stable MAC for that connection: `sudo proteus pin --connection "MyOrgWiFi"` (phase B).

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

- `proteus diff` (phase G) — config vs defaults vs live state, with drift flagged on managed files via the SHA in their headers.
- `cat /etc/proteus/state.json | jq .` — see what's been captured (original MACs, original hostname, history).
- `find /etc -name '*proteus*'` — see every drop-in Proteus has written.
- `sudo nft list ruleset` — see the firewall rules in the `proteus` table.
- `journalctl -t proteus -n 200 --no-pager` — recent log output.

## "I want to fully remove Proteus"

- One-shot: `sudo proteus uninstall --purge --yes` (phase G). Runs revert, removes the binary, clears `/etc/proteus/` and `/var/lib/proteus/`.
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
  ```
  sudo rm /etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf
  sudo systemctl restart systemd-resolved
  ```
- Cross-ref `proteus wiki dns` for the decision tree.

## "I see `not yet implemented` for command X"

Some commands are stubs in the current phase. This is by design — every subcommand parses, with help text, even before the implementation lands.

- Run `proteus help X` for the phase pointer (B, C, D, E, F, or G).
- The wiki page for that feature still describes the eventual behavior; check `proteus wiki <feature>`.

## "Permission denied"

- All mutating commands need root: `apply`, `rotate`, `revert`, `pin`, `unpin`, `reset`, `uninstall`. Prefix with `sudo`.
- Read commands work without root for any file the user can read. They degrade quietly when files aren't readable rather than failing loudly.

## "How do I run Proteus without systemd?"

You mostly can't. Proteus targets systemd as a primary dependency.

- Read commands work without systemd. `proteus status`, `proteus current`, `proteus original` will run.
- Mutating commands that modify systemd units fail cleanly with a message naming the missing dependency.
- Cross-ref `proteus wiki concepts` for the platform abstraction. Phase A is Linux + systemd only; other backends are theoretical.

## "Proteus says my config has drift"

Manual edits to a managed file get flagged loudly. This is a feature, not a bug.

- Run `proteus diff` (phase G) to see the path, expected SHA, and current SHA.
- Decide: re-apply Proteus's version with `sudo proteus apply`, or accept the local edit and update the header SHA, or back the whole thing out with `sudo proteus revert`.
- The header on every managed file looks like `# managed by proteus — do not edit` followed by `# expected-sha256: <hex>`. If you removed those headers, Proteus will treat the file as foreign and refuse to overwrite it.

## "Probes keep triggering rotations on a flaky link"

The probe quorum exists for exactly this — but if your network is bad enough, you can still trip it.

- Check the probe state: `proteus status --json | jq '.probes'`.
- Recent rotations: `journalctl -u proteus-check -n 100`.
- Loosen the quorum or extend the cooldown in `[probes]`. See `proteus wiki probes`.
- Or pin the interface for that environment: `sudo proteus pin --iface <iface>` (phase B). Pinned interfaces are skipped by both schedule and probe-driven rotation.

## Logs and diagnostics

When in doubt, look at the journal.

- Recent Proteus output: `journalctl -t proteus -n 200 --no-pager`.
- Rotate timer service: `journalctl -u proteus-rotate -n 100`.
- Probe-check service: `journalctl -u proteus-check -n 100`.
- Boot oneshot: `journalctl -u proteus-boot -n 100`.
- Run any command with `-vv` for debug: `proteus -vv status`. Verbose output goes to stderr and (under systemd) to journald.
- Status in JSON for programmatic inspection: `proteus status --json | jq .`. Same data the CLI uses.

## Cross-refs

- `proteus wiki cli` — full CLI reference, exit codes, JSON schemas (phase F).
- `proteus wiki config` — every config knob with default and risks (phase F).
- `proteus wiki uninstall` — full removal procedure.
- `proteus wiki discovery` — mDNS, LLMNR, NetBIOS, SSDP, WSD details.
- `proteus wiki dns` — the one DNS knob and its hard guard.
- `proteus wiki bluetooth` — adapter alias, BLE RPA, IRK discussion.
- `proteus wiki captive-portals` — portal detection, classification, policies.
- Per-feature wiki pages for the feature you're debugging.
