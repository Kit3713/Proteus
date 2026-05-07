# Real-world testing

The 248 unit tests are a good floor. They cover the orchestrator, the per-module apply paths, the state-file invariants, and the JSON schemas. They do not cover the diversity of DHCP servers, captive-portal quirks, BlueZ versions, and wpa_supplicant behaviors that exist in the wild — no test suite can. This page is the field guide for confirming Proteus actually works on a given network, and for recovering quickly when it does not. Read it once before taking the binary on the road for the first time.

## Pre-flight

`proteus doctor` is the first command to run. It is read-only, works without root, and prints `ok / warn / fail / skip` per check. The exit code is `0` when no checks fail and `1` when any do; `warn` and `skip` never cause a non-zero exit. Read the full output before applying anything.

```sh
proteus doctor
```

The warnings on a healthy minimal install are usually the detect-and-defer ones. If `dnscrypt-proxy`, `chrony`, `Pi-hole`, or `AdGuard Home` is on the system, `doctor` flags them as `warn`. Those are not failures — they tell the operator that Proteus's DNS or NTP knobs will skip in favor of the other tool. The same is true when `nft` is missing and the system uses firewalld instead: `doctor` prints `warn`, and the modules that need nftables surface as `skipped` at apply time rather than rolling back the whole run.

A `skip` line means the check could not run. The most common cause is "not root" — read commands work for any user, but a few checks need root for full detail. Re-run with `sudo proteus doctor` when something looks incomplete. The other common cause is a missing optional dependency, like BlueZ on a system without a Bluetooth adapter. Both are informational and do not block apply.

A `fail` line means apply will not be able to do its job until the underlying issue is fixed. Each fail line carries a remediation pointer like `see: systemctl start NetworkManager`. The two most frequent failures are NetworkManager not running (Proteus targets NM as the connection manager) and the config file failing to parse (a typo in `/etc/proteus/config.toml`). Fix the named cause and run `proteus doctor` again before moving on.

For the per-check reference and what each id measures, see `proteus wiki doctor`.

## Choose a profile

The `profile` field at the top of `/etc/proteus/config.toml` selects a coherent baseline of feature toggles. Six profiles ship: `off`, `min`, `low`, `med`, `high`, `agr`. The default is `med`. For first-time testing on a public network, `med` is the right choice — it adds mDNS and LLMNR silencing on top of the rotation and DHCP suppression in `low`, but does not enable any knob that could break a service on the network the operator cares about.

The `high` and `agr` profiles enable knobs that can interact poorly with specific services. `agr` adds SSDP and WSD blocks (which break KDE Connect and Windows printer discovery), anonymous outer identity for 802.1X (which some Microsoft NPS configurations reject), gratuitous ARP suppression (which breaks VRRP failover detection), and per-visit MAC rotation for known captive portals (which can cause re-prompting on hotel networks that bind sessions to the MAC). Try those only after `med` is confirmed working on the target network. The full breakdown lives in `proteus wiki profiles`.

The profile can be changed from the CLI without hand-editing the config file:

```sh
sudo proteus config set-profile med --yes
```

Per-knob overrides survive profile changes. `proteus config show` annotates each value with its origin so the operator can tell at a glance which knobs come from the profile baseline and which were explicitly set.

## Apply

When the doctor checks come back clean, apply:

```sh
sudo proteus apply --yes
```

The orchestrator runs each module in order — MAC, hostname, Bluetooth, IPv6, DHCP, DNS, stack, nft — and prints a per-module summary as it goes. The output looks like this on a typical public-Wi-Fi laptop:

```
apply summary:
  mac        applied   (ok)
  hostname   applied   (ok)
  bluetooth  applied   (ok)
  ipv6       applied   (ok)
  dhcp       applied   (ok)
  dns        applied   (ok)
  stack      applied   (ok)
  nft        applied   (ok)
totals: applied=8 skipped=0 failed=0
```

Each line is one of `applied`, `skipped (reason)`, or `failed (reason)`. A skipped module is informational — the most common reasons are "no BlueZ adapters detected" on systems without Bluetooth, "nftables not installed" on systems using firewalld, and "detected dnscrypt-proxy" when a competing DNS-privacy tool is present. None of those are bugs; they are detect-and-defer in action. A failed module needs a closer look. Run `proteus status --json | jq '.features.<name>'` for the structured failure reason, then cross-ref `proteus wiki troubleshooting`.

When the active profile enables a knob that can break a service, `proteus apply` prints a risk-warning banner above the summary. The banner names the specific knob, points at the wiki page that explains the breakage, and asks the operator to confirm. This is the orchestrator's way of surfacing the trade-off before it commits.

To preview without committing, use:

```sh
proteus dry-run apply
```

Dry-run prints what each module would do — files written, sysctl keys touched, NM connection settings updated — without changing anything. Useful for double-checking the plan on a network where a misstep would be disruptive.

The orchestrator is idempotent. Running `apply` ten times converges to the same state as running it once. That is a deliberate invariant: the field-guide pattern of "apply, check, apply again if something looks off" does not produce drift, double-rotation, or duplicate drop-ins. The only side effect of a redundant apply is the time it takes to walk every module.

After apply, the configured systemd timers take over. `proteus-rotate.timer` fires on the configured interval (default 2h) and rotates MACs across managed interfaces. `proteus-check.timer` fires every 5 minutes and runs the probe quorum to detect connectivity loss; on a portal-suspected classification it suppresses rotation rather than triggering a fresh round. Both timers are on by default after apply; verify with `proteus timer status`. Cross-ref `proteus wiki timer` for cadence syntax and `proteus wiki rotation` for the full trigger story.

## Verify

The deep dive lives in `proteus wiki verifying`, which has a section per identifier with the exact tcpdump and nmap commands to confirm rotation on the wire. The quick checklist for the road is shorter.

- `proteus session` — a one-screen snapshot of the current network session: active interface, SSID, when joined, captive-portal state, the MAC Proteus rotated to and how recently, and when the next scheduled rotation fires. The fastest way to confirm Proteus is doing what it claims on the network it is currently joined to.
- `proteus current` — the live identifiers the system is handing out right now: MAC per interface, hostname, DUID, Bluetooth alias. Compare against `proteus original` to confirm rotation happened.
- `proteus original` — the cached pre-Proteus values. Captured the first time Proteus saw the system; sacred and never re-captured. If `original` and `current` match for an interface, that interface has not been rotated.
- `proteus status` — per-feature `applied / skipped (reason) / failed (reason)`. The single most useful command after `proteus session`. Same data as the apply summary but readable any time.
- `ip link show` — the kernel's view of every interface, including the live MAC. Confirms the rotation actually reached the radio rather than stalling in NetworkManager.
- `hostname` — the static hostname, when the active profile enables hostname rotation. If the system's `[hostname]` config keeps the original hostname, this command shows the original; that is by design, not a bug.

All of these are read-only and work without root. If any of them shows an identifier that disagrees with what Proteus claims, that is worth investigating; cross-ref the per-feature wiki page from the troubleshooting section below.

A working end-to-end verification on a coffee-shop network looks like this. Before joining, run `proteus original` and note the cached MAC for the wireless interface. Join the network and complete the captive portal if any. Run `proteus session` and confirm the SSID matches, the captive-portal state is `cleared` or `unauthed` as expected, and the MAC field shows a rotated value distinct from the cached original. Run `ip link show <iface>` and confirm the live MAC matches the rotated value Proteus claims. Run `proteus status` and confirm every module is `applied` or `skipped` with a known reason; nothing should be `failed`. The whole loop is sub-second and works without root.

When the active profile rotates hostname alongside MAC, an additional check is worthwhile: compare `hostnamectl` output before and after a rotation, and confirm the static, pretty, and transient names all agree. Disagreement between the three is the symptom of a stalled `hostnamed` write — `proteus revert` followed by `proteus apply` usually clears it. Cross-ref `proteus wiki hostname-recipes` for the rotate-with-mac trade-offs and the wordlist-versus-generic mode discussion.

## Common surprises

Each of these is a real-world failure mode the project has seen. None of them are bugs in Proteus per se — they are environmental quirks the operator can resolve in a few seconds once they know what to look for.

**The MAC did not rotate.** First, confirm with `ip link show <iface>` that the live MAC is the one Proteus claims. If it is not, run `sudo proteus rotate --iface <iface> --yes` for that interface specifically and watch the output. Some USB Wi-Fi adapters — especially older Realtek chips — reject MAC writes through NetworkManager and need a per-interface workaround. Check `proteus session` to see whether the chipset Proteus detected matches the adapter in use.

**DHCP options were not suppressed.** Confirm that NetworkManager is the active connection manager: `nmcli general status` should show NM as the active state. Systems running systemd-networkd alone or wpa_supplicant in standalone mode are out of scope; Proteus targets NM as the primary backend. Cross-ref `proteus wiki dhcp` for the per-option rationale and the load-bearing role of `dhcp-client-id=mac`.

**DNS resolves but ECS strip did not apply.** Proteus defers when it detects `dnscrypt-proxy`, Pi-hole, AdGuard Home, knot-resolver, or a custom `/etc/resolv.conf`. Run `proteus dns status` to see what was deferred to and why. The other tool wins, every time — that is by design.

**Bluetooth alias did not change.** Confirm a BlueZ adapter is present: `bluetoothctl list` should show at least one. Proteus skips the Bluetooth module cleanly when no adapter is detected. If an adapter is present and the alias still did not change, check `proteus status --json | jq '.features.bluetooth'` for the structured reason. The most common cause is a paired device holding the bond open across a Powered=false / Powered=true cycle.

**The nft rules did not apply.** Confirm `nftables` is installed: `which nft` should return a path. firewalld does not count — Proteus writes its own table (`table inet proteus`) and does not interoperate with firewalld's chain layout. Install nftables and re-run apply, or accept that the stack-fingerprint hardening will skip on this host.

**The captive-portal flow looped.** The portal classifier runs against a small fixed set of probe targets and uses a quorum rule to decide between `clear`, `down`, `portal-suspected`, and `inconclusive`. On a network with an unusual portal flow — captive-portal-walled-garden academic Wi-Fi, vendor-specific authentication appliances, or any setup where the probe targets themselves are blocked — the classifier may not detect the portal correctly. Cross-ref `proteus wiki captive-portals` for the policy matrix and the loop-prevention invariants. The `proteus probe` command runs one probe round on demand and prints the per-endpoint outcome plus the classification, which is the right diagnostic for this case.

**A specific OS-level identifier still leaks.** Application-layer identifiers — browser fingerprint, account cookies, software-update probes, NTP client identifiers — are out of scope for Proteus. The first place to look is the `proteus wiki network-fingerprint-checklist` page, which enumerates every observable Proteus touches. If the leaking identifier appears on that list, file a bug. If it does not, the right tool is somewhere else in the stack: cross-ref `proteus wiki threat-model` for the composition story.

## Recover

Three escape hatches, in increasing intensity.

```sh
sudo proteus revert --yes
```

`revert` undoes Proteus's network-layer changes. It restores the cached original MAC and hostname, removes the systemd drop-ins, removes the nft rules, and reverts the per-connection settings Proteus wrote to NetworkManager. The originals cache is preserved (it is sacred and never re-captured), so a subsequent `proteus apply` re-applies the configured policy from a clean slate.

```sh
sudo proteus uninstall --purge --yes
```

`uninstall` runs revert first, then removes the binary, the systemd timers and services, and (with `--purge`) the config and state directories. The `--purge` form removes the originals cache too — only reach for it when the operator is done with the tool.

For the moment when the environment is no longer trustworthy and zero packets should leave the laptop right now:

```sh
sudo proteus kill --yes
sudo proteus resume --yes
```

`kill` brings every interface administratively down, disables the NM radios, and powers off Bluetooth. `resume` reverses each step. Both are idempotent; running `kill` while already killed exits `0`, and the same for `resume`. The full operational discussion lives in `proteus wiki kill-switch`.

When neither `revert` nor `kill` clears the symptom, the manual hatch is to remove the Proteus drop-ins by hand: `find /etc -name '*proteus*'` lists every file Proteus has written under `/etc/`, and each one carries a `# managed by proteus — do not edit` header so it is unambiguous. Removing the drop-ins and restarting the affected services (`systemd-resolved`, `NetworkManager`, `nftables`) returns the system to a Proteus-free state. Reinstall later if needed; the originals cache is still intact in `/var/lib/proteus/state.json`.

## Reporting back

When something does not work on the network in front of the operator and the troubleshooting section above did not help, file a bug. The project tracker lives on the upstream repository; cross-ref `CONTRIBUTING.md` for the filing conventions.

Useful diagnostic output to attach:

```sh
proteus doctor --json
proteus session --json
proteus status --json
journalctl -t proteus -n 200 --no-pager
```

The four commands above produce a complete read-only snapshot of what Proteus thinks it did, what the system actually shows, and the recent log output. The first three are JSON for easy filtering; the fourth is plaintext journald output. Include all four with the bug report, plus the failing command and the network type (residential, coffee shop, hotel, conference, airport, mobile hotspot). Skim the known-issue list on the tracker first — many environmental quirks recur, and a one-line "yes, that one" comment is more useful than a duplicate report.

Two reporting patterns are particularly valuable to the project. The first is a working report from a network type that is under-represented in the test matrix — corporate Wi-Fi with a vendor-specific captive portal, regional ISP-rented routers with non-standard DHCP options, eduroam at a non-US university, airport networks behind a non-Boingo consortium. A confirmed-working report is data; the project relies on field reports to know which environments are well-covered. The second is a clean reproduction of a failure: the exact sequence of commands, the network type, the output of the four commands above, and ideally a packet capture if the failure is on the wire (`sudo tcpdump -i <iface> -nn -vv 'udp port 67 or udp port 68' -c 4` for DHCP-side failures). A reproducible bug almost always gets fixed; a one-line "this did not work for me" report rarely does.

Sensitive identifiers in the JSON output — the cached MAC, the SSID, the captive-portal hostname — can be redacted before filing. The structure of the report matters more than the literal values; the project does not need the operator's actual MAC to debug a missing rotation, only the shape of the state file and the failing command output.

## A first-trip recipe

The shortest path from "binary installed" to "confident on a public network" is a six-step recipe that runs at home, then at the first untrusted network the operator joins.

At home, the night before:

```sh
proteus doctor
proteus original
proteus current
sudo proteus apply --yes
proteus session
```

The first two are read-only and confirm the system is ready and the originals cache is intact. The third confirms what the system currently exposes; on a fresh install before any rotation, `current` matches `original`. The fourth applies the configured profile and prints the per-module summary. The fifth confirms, at a glance, what the network session looks like after apply. If every line is `applied` or `skipped (known reason)` and `proteus session` shows the rotated MAC, the system is ready to take on the road.

At the first untrusted network the next morning:

```sh
proteus session
ip link show
```

Two read-only commands. The first is the Proteus view; the second is the kernel view. They should agree. If they do, Proteus is doing what it claims on the network in question. If they disagree, the next thing to run is `sudo proteus rotate --iface <iface> --yes` for the specific interface, followed by `proteus session` again to confirm the rotation reached the hardware. Anything still disagreeing after a forced rotation is worth filing.

This recipe takes under thirty seconds and is the floor for trusting Proteus on the road. Run it at home before a trip and on the first network of the trip; do not run it in transit for the first time.

## Networks worth testing on

A short list of network types and what each one exercises. The point is to confirm Proteus behaves on the diversity of real networks an operator will join, not to chase a comprehensive matrix.

- **Home Wi-Fi.** Baseline. The operator owns the network and can compare before-and-after captures with full visibility. The right place to confirm `med` works end-to-end before taking the binary on the road. mDNS responder silence is easiest to verify here, since a peer machine can run `avahi-browse -arpt` from the same LAN.
- **Coffee shop with captive portal.** Exercises the portal detection, the rotate-before-auth flow, and the periodic-rotation suppression while authed behind the portal. This is the single most common network type Proteus is used on; a working coffee-shop join is the floor for everything else.
- **Hotel Wi-Fi.** Per-MAC charging on some chains, captive portal binding the room number to the MAC for the duration of the stay, marketing trackers downstream of the portal. Exercises the per-connection MAC pin (`proteus pin --connection "HotelWiFi"`) for multi-night stays and per-visit MAC rotation when the active profile enables it. Cross-ref `proteus wiki hostile-environments` for the hotel playbook.
- **Conference Wi-Fi (e.g. eduroam).** 802.1X with anonymous outer identity, vendor-supplied analytics for attendee tracking, peer-snooping by other attendees with monitor mode. Exercises the enterprise-wifi module specifically. Some Microsoft NPS configurations reject mismatched outer/inner identity, so the conference is the right place to confirm the anonymous-outer knob works against the auth server in question.
- **Airport Wi-Fi (e.g. Boingo).** Saved-network leak via probe requests is the most prominent issue. The kernel emits probe requests for SSIDs in the saved-networks list whether the laptop joins or not, and the airport-Wi-Fi consortia operate at every airport in the chain. Exercises the operational hygiene around `nmcli connection show` and the probe-suppression pattern of `nmcli radio wifi off` until actively needed.
- **Mobile hotspot (phone tethering).** Edge case. The phone runs its own DHCP server and binds the lease to the MAC the laptop presents. MAC rotation mid-session can confuse the tether DHCP, especially on Android phones with aggressive lease management. Worth testing if the operator tethers regularly; the workaround is usually pinning the MAC for that connection profile.
- **Cellular / WWAN.** Out of Proteus's primary scope. The WWAN modem manages its own identifiers (IMSI, IMEI) and Proteus does not touch them. The network-side leak Proteus addresses is on the Wi-Fi and Ethernet interfaces, not the cellular one. Mentioned here so the operator does not expect Proteus to anonymize a cellular connection.

The more network types the binary is exercised on, the better the project's understanding of the diversity of real-world conditions. Bug reports from atypical environments are particularly welcome.

## JSON output for automation

Every read command on this page accepts `--json` and emits a stable schema. That makes the field-guide pattern wrappable: a wrapper script or a desktop GUI can poll `proteus session --json` for the current network state, `proteus doctor --json` for the health check, and `proteus status --json` for the per-feature apply state, without parsing human-readable output. The schemas carry a `schema_version` field that bumps only on backwards-incompatible changes; new fields do not bump it. Wrappers should ignore unknown fields defensively and key off `schema_version` for breaking-change handling.

The exit codes are stable too: `0` for success, `1` for generic error, `64` when a destructive command needs `--yes`, `66` when a command needs root. A wrapper testing whether any feature has failed can check `proteus status --json | jq '.summary.failed > 0'` and branch on the result. The same pattern works for any of the read commands.

For the full schema reference, cross-ref `proteus wiki cli`. The schemas are tested for stability and are part of the public contract; breaking changes go through a deprecation cycle and a `schema_version` bump.

## Cross-references

- `proteus wiki getting-started` — first-run tutorial with explanations.
- `proteus wiki hostile-environments` — adversary-aware tactics for cafes, hotels, conferences, airports, and the worse end of the spectrum.
- `proteus wiki troubleshooting` — symptom-based recovery when something breaks.
- `proteus wiki profiles` — choosing the right profile for the network at hand.
- `proteus wiki verifying` — verify Proteus is doing what it says, with the exact tcpdump and nmap commands per identifier.
- `proteus wiki captive-portals` — portal detection, classification, and the loop-prevention invariants.
- `proteus wiki kill-switch` — emergency network shutdown when the environment is no longer trustworthy.
- `proteus wiki doctor` — every read-only check `proteus doctor` runs and how to interpret each result.
