# Proteus

A Rust CLI that erases the network identifiers your Linux laptop hands out every time it joins a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name. Rotates MACs on a schedule and on connectivity loss. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

Named after the shapeshifter.

## Status

Pre-release. The v1 plan is in [`docs/PLAN.md`](docs/PLAN.md). Phase A (project skeleton) hasn't started — there is nothing to install yet. The most useful contribution today is feedback on the plan.

## What it does

L2 — Wi-Fi MAC, Ethernet MAC, Bluetooth adapter name and discoverability, BLE Resolvable Private Address mode where the controller supports it. 802.1X anonymous outer identity for enterprise Wi-Fi (opt-in).

L3 — IPv6 stable-privacy and temp addresses, DUID rotated alongside MAC, ICMPv6/NDP fingerprint hardening.

L3-L4 — TCP timestamps off, ICMP info-replies dropped, optional gratuitous-ARP suppression.

DHCP — options 12, 60, 61, 81 suppressed.

Discovery — mDNS responder + resolver, LLMNR, NetBIOS silenced; SSDP and WSD blocked behind opt-in flags because they break KDE Connect and WS-Discovery printers.

Hostname — kernel/pretty/transient rotatable from a wordlist, generic-default option, optional rotate-with-MAC.

Captive portals — first-class detection, fresh MAC per visit to known portals, no rotation loops while authed.

DNS — one narrow knob: strip EDNS Client Subnet on systemd-resolved, with a hard guard that defers to dnscrypt-proxy / Pi-hole / AdGuard Home / custom resolv.conf when present.

RF — opt-in TX power reduction (smaller capture radius for passive listeners) plus chipset reporting in `proteus status`.

Full feature list and rationale in [`docs/PLAN.md`](docs/PLAN.md). Comparison to existing tools in [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md).

## What it doesn't do

This is a network-layer fingerprint eraser. It is not:

- a TLS or browser fingerprint tool — use Tor Browser, librewolf, or Brave's randomization
- a DNS-privacy tool beyond the one ECS-strip knob — use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole
- a tracker blocker — use Pi-hole, NextDNS, or uBlock Origin
- a traffic correlation defense — use Tor or Mullvad VPN
- a hardening framework — Proteus refuses to weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`, or rotate `/etc/machine-id`

The wiki page `threat-model` (planned for phase F) will spell this out so users don't over-trust it.

## Requirements

- Linux with systemd
- NetworkManager (managed via dbus, no `nmcli` shelling)
- systemd-resolved
- Optional: BlueZ for the Bluetooth features, firewalld or nftables for discovery blocks
- Glibc or musl
- Fedora 43+ is the primary target; other modern systemd distros are secondary

## Installing

Not yet. The plan is `./install.sh` once the binary exists.

## Building from source

Not yet. Cargo project lands in phase A.

## Documentation

- [`docs/PLAN.md`](docs/PLAN.md) — what's being built and in what order
- [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md) — what already exists and where Proteus fits
- [`SECURITY.md`](SECURITY.md) — how to report vulnerabilities
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to help

The full wiki ships embedded in the binary once Phase A lands. `proteus wiki` and `proteus help` are the user-facing entry points.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). If you distribute a modified version of Proteus, you must release the source under GPLv3 (or later) as well.

Contributions are accepted under the same terms.
