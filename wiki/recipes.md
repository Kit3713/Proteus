A cookbook of common scenarios with full command sequences. Each recipe is self-contained: when it applies, what to run, how to verify, and where to look if it breaks.

For the mental model behind these flows, read `proteus wiki concepts` first. For the per-command reference, see `proteus wiki cli`. For the symptom-based recovery guide, see `proteus wiki troubleshooting`.

## Scenario: First install + verify

You just installed the binary. Confirm the system is healthy, see what the system currently exposes, and read the cached originals before doing anything mutating.

```sh
proteus --version
proteus doctor
proteus status
proteus current
proteus original
```

**Verify:**

```sh
proteus doctor --json | jq '.checks[] | select(.status=="fail")'
proteus current --json | jq .interfaces
proteus original --json | jq .
```

`doctor` exits `0` when no checks fail. `current` and `original` should match on a fresh install — nothing has been rotated yet.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`. Cross-ref `proteus wiki getting-started` for the full first-time walkthrough.

## Scenario: Adjust to your home network

Trusted LAN, your own AP, no captive portal. The default 2h rotation cadence is fine; you only want a few small relaxations to avoid breaking household integrations (Bluetooth audio, AirPlay, mDNS-based file sharing).

```sh
sudo proteus config disable bluetooth --reason "trusted home, AirPods" --yes
sudo proteus config set mac.rotation_interval 4h --yes
sudo proteus apply --yes
```

**Verify:**

```sh
proteus config show
proteus status --json | jq '.features[] | select(.name=="bluetooth")'
proteus timer status
```

The `disable --reason` text appears in `proteus status` so you remember why it's off. Cross-ref `proteus wiki bluetooth` for the BLE RPA tradeoffs and `proteus wiki rotation` for cadence guidance.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Adjust for public Wi-Fi user

You spend most of your time on cafe, library, and coworking Wi-Fi. The standard preset plus a faster cadence and stricter discovery silencing is the right starting point.

```sh
sudo cp examples/standard.toml /etc/proteus/config.toml
sudo proteus config set mac.rotation_interval 1h --yes
sudo proteus config enable bluetooth --yes
sudo proteus config enable discovery --yes
sudo proteus apply --yes
sudo proteus timer set rotate --interval 1h
```

**Verify:**

```sh
proteus status --json | jq .features
proteus current --json | jq .
proteus timer status
```

Faster cadence costs more DHCP renewals; `1h` is a reasonable floor on most networks. Cross-ref `proteus wiki hostile-environments` for the per-environment playbook (cafe, conference, hotel, airport) and `proteus wiki discovery` for the WSD/SSDP breakage warnings.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Check status before traveling

Pre-trip 30-second sanity check. Run this before you walk out the door so any surprises happen at home, not in the cafe.

```sh
proteus doctor
proteus status
proteus current
proteus original
proteus timer status
```

**Verify:**

```sh
proteus doctor --json | jq '.checks[] | select(.status=="fail")'
proteus status --json | jq '.features[] | select(.state=="failed")'
proteus original --json | jq .original_macs
```

If `doctor` returns non-zero, fix that before you leave. The originals cache should never be empty after first run; if it is, your state file got wiped and you've lost your revert anchor — stop and investigate. Cross-ref `proteus wiki security-checklist` and `proteus wiki hostile-environments`.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Manually rotate before joining a new network

You're about to join a network you don't trust. NetworkManager remembers the previous network's MAC for a few seconds; rotating first prevents leaking the previous identity into the new operator's logs.

```sh
sudo proteus rotate --yes
proteus current
```

**Verify:**

```sh
proteus current --json | jq '.interfaces[].mac'
ip link show wlan0 | grep ether
```

The `--yes` flag is required when rotating without `--iface` because the command touches every managed interface at once. Pinned interfaces are skipped silently. Cross-ref `proteus wiki mac-recipes` for OUI pool selection and `proteus wiki rotation` for the full trigger story.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Pin your MAC for one specific network

Your hotel charges per-MAC, your corporate Wi-Fi binds 802.1X to a cert plus MAC, your home AP has a DHCP reservation. Pin the MAC to that connection profile so neither the schedule nor the probe-driven trigger touches it.

```sh
sudo proteus pin --connection "HotelWiFi"
```

**Verify:**

```sh
proteus status --json | jq .pinned
proteus current --json | jq '.interfaces[] | select(.name=="wlan0").mac'
```

Connection-scoped pins are preferred over interface-scoped pins when both apply. Release the pin when you no longer need it: `sudo proteus unpin --connection "HotelWiFi"`. Cross-ref `proteus wiki mac-recipes` for the full pin/unpin model.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Disable Bluetooth tracking

You don't use Bluetooth at this venue, or you use it for audio only and want the adapter generic-aliased with discoverability off and BLE Resolvable Private Address mode on. Proteus handles all three knobs without touching your pairings.

```sh
sudo proteus config enable bluetooth --yes
sudo proteus apply --yes
```

**Verify:**

```sh
proteus status --json | jq '.features[] | select(.name=="bluetooth")'
bluetoothctl show | grep -E "Alias|Discoverable"
```

For full silence at high-threat venues, drop the radio entirely: `sudo rfkill block bluetooth`. Cross-ref `proteus wiki bluetooth` for the RPA limits and the BR/EDR (classic) caveats.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Change rotation cadence

Default cadence is 2h. Faster (`30m`, `1h`) means more privacy at the cost of more DHCP renewals; slower (`4h`, `8h`) is quieter but widens the correlation window. Set both the config knob and the timer override so they agree.

```sh
sudo proteus config set mac.rotation_interval 1h --yes
sudo proteus timer set rotate --interval 1h
```

**Verify:**

```sh
proteus config get mac.rotation_interval
proteus timer status
systemctl cat proteus-rotate.timer
```

To restore the default: `sudo proteus timer reset rotate` and `sudo proteus config reset mac --yes`. Cross-ref `proteus wiki timer` for cadence syntax (`30m`, `hourly`, `*-*-* 06:00:00`) and `proteus wiki rotation` for the full trigger story.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Corporate Wi-Fi rejects me after rotation

Enterprise 802.1X is fussy. Some Microsoft NPS configs reject mismatched outer/inner identity and silently drop you; others bind the cert to a specific MAC. Disable the opt-in anonymous-outer-identity knob for that connection and pin the MAC if needed.

```sh
sudo proteus config set enterprise_wifi.anonymous_outer_identity false --yes
sudo proteus pin --connection "MyOrgWiFi"
sudo proteus apply --yes
```

**Verify:**

```sh
journalctl -u NetworkManager -n 100 | grep -E "EAP-Failure|auth failed"
proteus status --json | jq .pinned
```

Re-attempt the connection. If it works, you've found the cause. Cross-ref `proteus wiki enterprise-wifi` for the 802.1X knobs and `proteus wiki troubleshooting` for the per-connection disable pattern.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Captive portal kicked me out

You authed against MAC `X`, the rotation timer fired, and the portal now sees an unknown client. The default `rotate-before-auth` policy plus the periodic-rotation suppression while authed should prevent this — but for SMS-bound portals where the auth ticket is tied to your MAC, switch to `preserve-mac` and rejoin.

```sh
sudo proteus config set captive_portal.policy preserve-mac --yes
sudo proteus apply --yes
proteus portal mark "Boingo Hotspot"
```

**Verify:**

```sh
proteus status --json | jq .portal
proteus probe --json | jq .classification
proteus portal list
```

`preserve-mac` keeps your MAC across rotation events for any portal-classified network. The known-portal list ensures fresh-MAC-per-visit on next reconnect. Cross-ref `proteus wiki captive-portals` for the policy matrix and the loop-prevention invariants.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: DHCP slow on a network

Your ISP or venue throttles DHCP renewals; aggressive rotation triggers a fresh lease each time and the network blacklists you. Increase the rotation interval and confirm.

```sh
sudo proteus config set mac.rotation_interval 4h --yes
sudo proteus timer set rotate --interval 4h
sudo proteus apply --yes
```

**Verify:**

```sh
proteus config get mac.rotation_interval
proteus timer status
journalctl -t proteus -n 100 --no-pager | grep rotate
```

Anything under `30m` means a fresh lease every half hour, which is too aggressive for many home and hotel networks. To disable rotation entirely on a known-friendly network, pin the connection: `sudo proteus pin --connection "<ssid>"`. Cross-ref `proteus wiki dhcp` and `proteus wiki rotation`.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Reset and start fresh

You tinkered, you broke something, you want defaults back. `reset` clears `/etc/proteus/config.toml` to defaults and re-applies. The cached original MACs and hostname are not touched — those are sacred and survive every reset.

```sh
sudo proteus reset --yes
proteus config show
sudo proteus apply --yes
```

**Verify:**

```sh
proteus original --json | jq .
proteus config show
proteus status
```

The originals cache should be unchanged. If you want the panic-button instead, `sudo proteus revert --yes` restores everything to the cached originals without changing your config. Cross-ref `proteus wiki concepts` for the sacred-originals rule.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Run alongside dnscrypt-proxy

You run `dnscrypt-proxy` for encrypted DNS. Proteus's one DNS knob (strip EDNS Client Subnet) refuses to apply when another DNS-privacy tool is detected. Verify the hard guard is doing its job.

```sh
systemctl is-active dnscrypt-proxy
proteus status
proteus dns status
```

**Verify:**

```sh
proteus status --json | jq .features[] | select(.name=="dns")
proteus doctor --json | jq '.checks[] | select(.id|contains("dns"))'
dig +short txt o-o.myaddr.l.google.com
```

The DNS feature should read `skipped (detected dnscrypt-proxy)`. The user's DNS setup wins, every time. If Proteus tries to apply the drop-in anyway, that's a bug — file it. Cross-ref `proteus wiki dns` for the full detect-and-defer list.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Run alongside Pi-hole

Same pattern as dnscrypt-proxy. Pi-hole's FTL daemon is the detection signal; Proteus defers cleanly and surfaces the deferral in status.

```sh
systemctl is-active pihole-FTL
proteus status
proteus dns status
```

**Verify:**

```sh
proteus status --json | jq '.features[] | select(.name=="dns")'
proteus doctor --json | jq '.checks[] | select(.id|contains("dns"))'
cat /etc/resolv.conf
```

DNS should read `skipped (detected pi-hole)`. Proteus does not touch your resolver, does not touch `/etc/resolv.conf`, does not write a drop-in under `/etc/systemd/resolved.conf.d/`. Cross-ref `proteus wiki dns`.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: Pre-flight check before traveling

Going to a less-trusted environment. Tighten the cadence, lock down Bluetooth, verify the originals cache, dump status for offline reference. Five commands, sixty seconds.

```sh
proteus doctor
sudo proteus config set mac.rotation_interval 30m --yes
sudo proteus timer set rotate --interval 30m
sudo proteus apply --yes
proteus status --json > /tmp/proteus-pretrip-$(date -u +%F).json
```

**Verify:**

```sh
proteus status --json | jq .features
proteus current --json | jq .
proteus timer status
proteus original --json | jq .original_macs
```

Carry the dumped status JSON somewhere you can read on the road. If something looks wrong in the field, you have a known-good baseline to diff against. Cross-ref `proteus wiki hostile-environments` for the full per-environment playbook (cafe, conference, hotel, airport) and `proteus wiki security-checklist`.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Scenario: After-trip cleanup

You're home. Drop the trip's session state, restore your home cadence, confirm originals are still intact.

```sh
sudo proteus revert --yes
sudo proteus config set mac.rotation_interval 2h --yes
sudo proteus timer reset rotate
sudo proteus apply --yes
```

**Verify:**

```sh
proteus original --json | jq .
proteus config get mac.rotation_interval
proteus current --json | jq .
proteus status
```

`revert` restores everything to the cached originals; the originals cache itself is untouched. Re-applying brings your home config back into effect with the default cadence. Browser cleanup (cookies, site data, local storage for any sites you visited) is a separate step Proteus does not handle — that's application-layer state. Cross-ref `proteus wiki security-checklist` for the post-trip checklist and `proteus wiki hostile-environments` for the broader after-trip discussion.

**If something breaks:** `proteus doctor` then see `proteus wiki troubleshooting`.

## Cross-references

- `proteus wiki getting-started` — first-time setup walkthrough
- `proteus wiki concepts` — identifiers, sacred originals, managed files, idempotency
- `proteus wiki mac-recipes` — OUI pools, pinning, fresh-MAC-per-visit, DUID/IID coupling
- `proteus wiki hostname-recipes` — hostname modes, rotate-with-mac, RFC 1123 constraints
- `proteus wiki captive-portals` — policy matrix, classification, loop-prevention
- `proteus wiki hostile-environments` — cafe, hotel, conference, airport playbooks
- `proteus wiki security-checklist` — daily, weekly, monthly, pre-trip, post-trip routines
- `proteus wiki rotation` — schedule, probe-driven, boot oneshot
- `proteus wiki timer` — cadence syntax, drop-in overrides, the four named units
- `proteus wiki dns` — the one ECS-strip knob and its hard guard
- `proteus wiki bluetooth` — adapter alias, BLE RPA, BR/EDR limits
- `proteus wiki troubleshooting` — symptom-based recovery
- `proteus wiki cli` — full command reference, exit codes, JSON schemas
- `proteus wiki threat-model` — what Proteus does and does not address
