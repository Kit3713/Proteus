# Proteus Wiki

Curated guide to the embedded wiki. Run `proteus wiki <page>` to read any
entry below, or `proteus wiki search <term>` for full-text search.

## Start here

- `intro` — what Proteus is and what it doesn't do
- `quickstart` — install, apply, verify in five minutes
- `getting-started` — first-run walkthrough with explanations
- `concepts` — vocabulary you'll see across the wiki

## Core features

- `rotation` — MAC rotation policy and timer cadence
- `mac-recipes` — common MAC scenarios (pinning, OUI pools, per-iface)
- `hostname-recipes` — hostname rotation and pinning
- `bluetooth` — adapter alias, discoverable state, BLE RPA

## Fingerprint-reduction knobs

- `discovery` — mDNS/LLMNR/SSDP/WSD silencing
- `dns` — EDNS-Client-Subnet strip on systemd-resolved
- `dhcp` — DHCP option suppression (12/60/61/81 + DUID/IAID)
- `ipv6` — stable-privacy + temporary addresses + DUID rotation
- `stack-fingerprint` — TCP/ICMP/NDP sysctl hardening
- `enterprise-wifi` — 802.1X anonymous outer identity (opt-in)
- `rf-fingerprinting` — OS-controllable RF surface (TX power, probe behavior, chip inventory) and the hardware-analog limits Proteus cannot fix
- `captive-portals` — detection + known-portal SSIDs
- `kill-switch` — emergency network shutdown
- `ip-rotation` — DHCP lease release/renew

## Day-to-day

- `cli` — full command reference
- `config` — config file shape and defaults
- `profiles` — the six functional profiles (`off`, `min`, `low`, `med`, `high`, `agr`)
- `per-ssid` — per-SSID profile policies (override persona / profile / pin per network)
- `timer` — systemd timer management
- `doctor` — self-diagnostic checks
- `probes` — connectivity probe rounds
- `troubleshooting` — when things don't go as expected
- `real-world-testing` — verifying Proteus on a coffee shop, hotel, or conference network
- `distro-support` — supported init systems, backends, architectures, and package layouts
- `uninstall` — clean removal

## Reference

- `threat-model` — what Proteus protects against (and what it doesn't)
- `network-fingerprint-checklist` — every observable Proteus touches
- `security-checklist` — operational hygiene
- `glossary` — term definitions
- `faq` — frequently asked questions
- `internals` — how the pieces fit together
- `verifying` — how to confirm Proteus is doing what it says
- `reproducible-builds` — build determinism notes
- `throttling-detect` — adversary throttling detection
- `hostile-environments` — coffee-shop / hotel / conference Wi-Fi tactics
- `journald-network-logs` — reading the journal for network events
- `wpa-supplicant-hardening` — supplicant configuration hardening
- `recipes` — assorted multi-feature recipes
