# Prior art

Tools that share scope with Proteus, in part or in whole. For each one: what it does, where it stops, where Proteus picks up. Proteus is a network-layer fingerprint eraser — wider than `macchanger`, narrower than Tails, complementary to the DNS and browser tools.

The grouping is rough: standalone CLI tools, in-tree network-stack features, OS-level platform behavior, whole-system privacy distros, and adjacent tools Proteus deliberately defers to. Where existing tools already cover ground that Proteus also touches, that's called out — there is real overlap, especially with NetworkManager.

## Standalone CLI tools

### macchanger

[github.com/alobbs/macchanger](https://github.com/alobbs/macchanger) — the classic. Spoofs the MAC of a single interface from the command line. Random, fixed, OUI-preserving, vendor-list lookup.

What it covers: one-shot MAC change on an interface that's down.

Where it stops: no scheduling, no probe-driven rotation, no captive-portal handling, no DHCP option suppression, no DUID rotation, no IPv6 stable-privacy coordination, no hostname, no Bluetooth, no integration with NetworkManager — in fact `macchanger` will fight NM if NM has its own cloned-MAC policy on the connection. Single-shot, single-interface, single-identifier.

Where Proteus picks up: schedules (Phase C), captive portals (Phase C), DHCP options 12/60/61/81 (Phase D), DUID + IPv6 stable-privacy alongside MAC (Phase D), hostname (Phase D), Bluetooth (Phase B), and goes through the NM dbus surface so the two don't fight.

If your need is "change my Wi-Fi MAC once before joining this coffee-shop network," `macchanger` is fine. If your need is "every identifier this laptop emits should look different two hours from now," that's Proteus.

## In-tree: NetworkManager, iwd, systemd-networkd

These ship with the OS. Proteus's job is to drive them, not replace them.

### NetworkManager — `wifi.cloned-mac-address` and `connection.stable-id`

Documented under [networkmanager.dev/docs/api/latest/settings-802-11-wireless.html](https://networkmanager.dev/docs/api/latest/settings-802-11-wireless.html) and the connection settings at [networkmanager.dev/docs/api/latest/settings-connection.html](https://networkmanager.dev/docs/api/latest/settings-connection.html). Reference docs at [networkmanager.dev/docs/api/latest/nm-settings-nmcli.html](https://networkmanager.dev/docs/api/latest/nm-settings-nmcli.html).

What it covers: per-connection MAC policy. `random` (new MAC every activation), `stable` (per-SSID deterministic, derived from `connection.stable-id`), `preserve` (whatever the interface had), `permanent`, or an explicit address. `connection.stable-id` parameterizes the stable derivation so you can rotate the per-SSID identity without changing connection. NM also handles DHCP client identifier and hostname settings on a per-connection basis.

Where it stops: no time-based rotation while connected, no probe-driven rotation on connectivity loss, no captive-portal classification, no coordinated rotation of MAC + DUID + hostname + IPv6 stable-privacy + DHCP options as one atomic identity flip. The DHCP option knobs exist but you're on your own to know which ones leak. No Bluetooth, no mDNS, no SSDP, no TCP-timestamp scrub. The `stable` mode is great if you want the same fake MAC every time you rejoin a given SSID — Proteus's default model is closer to "different MAC every two hours regardless."

Where Proteus picks up: drives NM via dbus to set `cloned-mac-address` and `stable-id` (Phase B, D), adds the scheduling and probe layer NM doesn't have (Phase C), bundles all the network-identity knobs into one rotation event, and adds the captive-portal state machine.

Honest overlap: if you only care about Wi-Fi MAC and you're happy with per-SSID stable randomization, NM alone covers your case. Proteus is the rest of the iceberg.

### iwd — built-in MAC randomization

iwd is the kernel-team's userspace Wi-Fi daemon. Project wiki at [iwd.wiki.kernel.org](https://iwd.wiki.kernel.org/), settings reference [iwd.wiki.kernel.org/networkconfigurationsettings.html](https://iwd.wiki.kernel.org/networkconfigurationsettings.html). Man pages at [man.archlinux.org/man/iwd.8](https://man.archlinux.org/man/iwd.8) and [man.archlinux.org/man/iwd.network.5](https://man.archlinux.org/man/iwd.network.5). Distribution overview at [wiki.archlinux.org/title/Iwd](https://wiki.archlinux.org/title/Iwd).

What it covers: `AddressRandomization` per network (`disabled`, `network`, `once`) plus `AlwaysRandomizeAddress` and `AddressOverride` at the global or per-network level. Roughly the same model as NetworkManager: per-network deterministic randomization, or once-per-association.

Where it stops: same gaps as NM — no scheduling, no probe-driven rotation, no captive-portal flow, no coordinated identity flip, no Bluetooth/discovery/stack-level work.

Where Proteus picks up: Proteus's primary integration target is NetworkManager (see Phase A and the README's requirements), so iwd is on the secondary-distro path. If iwd ships as the active Wi-Fi daemon (e.g. some Arch setups), the per-interface knobs Proteus reads via netlink still work for status; full mutation support for iwd-only systems is not a v1 commitment.

### systemd-networkd — `MACAddressPolicy=random`

Documented at [freedesktop.org/software/systemd/man/latest/systemd.network.html](https://www.freedesktop.org/software/systemd/man/latest/systemd.network.html) under the `[Link]` section and the related `systemd.link(5)`.

What it covers: link-level MAC policy. `MACAddressPolicy=random` generates a random locally-administered MAC at link-up; `persistent` derives a stable one from `/etc/machine-id`; `none` leaves it alone. Set in `.link` files under `/etc/systemd/network/`.

Where it stops: link policy is link-up time only. There is no rotate-on-schedule, no rotate-on-probe-fail, no captive-portal awareness, no DHCP/IPv6/hostname coordination. The `persistent` mode also derives from `machine-id`, which is exactly the identifier Proteus refuses to rotate (TPM/journald/dbus all reference it — see the README's "Invariants").

Where Proteus picks up: Proteus targets NetworkManager-managed systems, but the underlying netlink reads in Phase A work regardless of which manager is in charge. systemd-networkd-only deployments aren't a v1 target.

## OS-level platform behavior, for comparison

Not Linux tools, but worth comparing because they shape user expectations.

### Apple Private Wi-Fi Address (iOS 14+, iPadOS 14+, watchOS 7+, macOS 12+)

[support.apple.com/en-us/102509](https://support.apple.com/en-us/102509) is the user-facing doc.

What it covers: per-SSID deterministic MAC, on by default. Same SSID gets the same MAC across rejoins; different SSID gets a different MAC. iOS 18 / macOS 15 added a "Rotating" option for some networks that changes the per-SSID MAC every couple of weeks.

Where it stops: per-SSID stable, weeks-scale rotation at most. No DHCP option scrub the user can see, no hostname rotation. Closed platform — no equivalent on Linux without third-party tooling.

Where Proteus picks up: the Linux equivalent, with hours-scale rotation by default, plus everything else (DHCP, IPv6, hostname, mDNS, Bluetooth, TCP). Apple's model is a useful baseline for "what's normal in 2026" — Proteus is more aggressive on schedule, broader on scope.

### Android per-network MAC randomization (Android 10+)

[source.android.com/docs/core/connect/wifi-mac-randomization-behavior](https://source.android.com/docs/core/connect/wifi-mac-randomization-behavior) is the AOSP doc.

What it covers: per-SSID persistent random MAC by default since Android 10. Android 12 added a "non-persistent" option that re-randomizes every 24h or on every connection in some conditions.

Where it stops: Android-only, Wi-Fi-only, no user-space hooks for DHCP/hostname/discovery on a typical phone. Same general shape as Apple's: per-SSID stable.

Where Proteus picks up: Linux laptops, full network-identity stack, faster default rotation cadence. The Android doc is worth reading anyway — it's a clear writeup of how the per-SSID model works in practice and what the gotchas are (enterprise auth, captive portals).

## Bluetooth tooling

### `hciconfig`, `bdaddr`

`hciconfig` man page at [man.archlinux.org/man/hciconfig.1](https://man.archlinux.org/man/hciconfig.1) (deprecated upstream — replaced by `bluetoothctl` and `btmgmt`). `bdaddr` from BlueZ tools, source at [github.com/pauloborges/bluez/blob/master/tools/bdaddr.c](https://github.com/pauloborges/bluez/blob/master/tools/bdaddr.c), Arch man page [man.archlinux.org/man/bdaddr.1](https://man.archlinux.org/man/bdaddr.1). BlueZ project at [github.com/bluez/bluez](https://github.com/bluez/bluez).

What it covers: `hciconfig` lets you set adapter name, class, and discoverable state. `bdaddr` claims to write the BD_ADDR (Bluetooth address) of an adapter — but only on chips it knows the vendor-specific HCI command for. The BlueZ source has separate code paths for CSR, TI, Ericsson, Zeevo, ST, and Marvell, and falls through with "device not supported" for everything else.

Where it stops: BR/EDR (classic) BD_ADDR rotation is a per-vendor HCI command. There is no standard. If your adapter isn't in `bdaddr.c`'s list, you can't rotate the address from software — and even if it is, some chips reject the command after first boot, some lose calibration data, some need a controller reset that confuses the host stack. This is exactly why Proteus defers BR/EDR BD_ADDR rotation (Phase B note, README "Out of scope").

Where Proteus picks up: the safe Bluetooth bits via BlueZ over zbus — generic adapter alias, `discoverable=off` by default, BLE Resolvable Private Address mode where the controller advertises support (Phase B). BR/EDR rotation stays out until there's a known-good chipset matrix.

## Whole-system privacy distros

Different problem class — these make the whole OS amnesiac or compartmentalized. Worth listing because users sometimes reach for them when a single CLI tool would have done.

### Tails

[tails.net](https://tails.net/). Live USB, amnesic by design, all traffic forced through Tor. MAC spoofing is on by default at boot.

What it covers: everything network-adjacent and a lot besides — full anonymity stack with Tor as the correlation defense.

Where it stops: it's a separate OS. You can't run it as your daily driver and use Steam, KDE Connect, or your work VPN.

Where Proteus picks up: Tails is the right answer when the threat model needs Tor and amnesia. Proteus is the right answer when the threat model is "I keep using my Linux laptop daily, I just don't want every coffee shop and conference Wi-Fi to be able to recognize it across visits."

### Whonix

[whonix.org](https://www.whonix.org/). Two-VM design — a gateway VM forces all traffic from a workstation VM through Tor. Strong correlation resistance.

What it covers: traffic-correlation defense via forced Tor isolation. Sane defaults for the workstation VM.

Where it stops: VM-shaped. Doesn't address what your host's L2/L3 identifiers look like to the network you're physically attached to. Whonix and Proteus are orthogonal — you could run Whonix on a host that uses Proteus to scrub the host's network identity.

### Qubes OS

[qubes-os.org](https://www.qubes-os.org/). Compartmentalization OS — every workload in its own VM, network stack in a sys-net qube, firewall in a sys-firewall qube.

What it covers: isolation. A compromised browser qube can't see your work qube.

Where it stops: not a fingerprint eraser. The sys-net qube still has a MAC, still does DHCP, still sends a hostname. Qubes documents MAC randomization recipes for sys-net but doesn't ship the kind of scheduled, coordinated identity flip Proteus is built for.

Where Proteus picks up: a Linux-distro-agnostic CLI for the network-identity layer. Could in principle be installed inside a Qubes sys-net qube; not a tested configuration in v1.

## Adjacent tools Proteus deliberately defers to

These cover ground Proteus does not. Listed so users know which tool to reach for and so the scope boundary is explicit. The README's "What it isn't" section and `[../CONTRIBUTING.md](../CONTRIBUTING.md)`'s scope list say the same thing in fewer words.

### DNS

- **dnscrypt-proxy** — [github.com/DNSCrypt/dnscrypt-proxy](https://github.com/DNSCrypt/dnscrypt-proxy). Local DNS proxy with DoH/DoT/DNSCrypt upstream, anonymized DNS relays, blocklists.
- **NextDNS** — [nextdns.io](https://nextdns.io/). Hosted DoH/DoT resolver with per-profile policy, blocklists, logging controls.
- **AdGuard Home** — [adguard.com/en/adguard-home/overview.html](https://adguard.com/en/adguard-home/overview.html). Self-hosted network-wide DNS filter.
- **Pi-hole** — [pi-hole.net](https://pi-hole.net/). Self-hosted DNS sinkhole, the original network-wide ad-blocking resolver.
- **knot-resolver** — [knot-resolver.cz](https://www.knot-resolver.cz/). Modern caching validating recursor.

These are DNS tools. They do DNS well. Proteus has exactly one DNS knob — strip EDNS Client Subnet from systemd-resolved when systemd-resolved is the active resolver and no other DNS-privacy tool is detected (Phase D). If any of the above are present, Proteus refuses to touch the resolver and names what it found in `proteus status`. The user's DNS setup wins, every time.

### Browser fingerprints

- **Tor Browser** — [torproject.org](https://www.torproject.org/). The reference implementation of browser-fingerprint resistance — Letterboxing, font/canvas blocking, normalized `User-Agent`, the works.
- **LibreWolf** — [librewolf.net](https://librewolf.net/). Firefox fork with privacy-focused defaults, including `privacy.resistFingerprinting` on.
- **Brave** — [brave.com/privacy-features](https://brave.com/privacy-features/). Built-in fingerprint randomization (farbling) for Canvas, WebGL, audio, fonts.

Browser fingerprints (Canvas, WebGL, fonts, screen, language, JS quirks) are solved from inside the browser process. Nothing on the host can normalize what JavaScript reads back. Proteus doesn't try.

### Tracker blocking

- **Pi-hole** — see above.
- **NextDNS** — see above.
- **uBlock Origin** — [github.com/gorhill/uBlock](https://github.com/gorhill/uBlock). Browser extension, the default content-blocker recommendation.

Tracker IDs ride inside application traffic. Network-layer tools can't see them. Use a content blocker.

### Traffic correlation

- **Tor** — see above.
- **Mullvad VPN** — [mullvad.net](https://mullvad.net/).

If the threat model includes a global passive adversary correlating traffic across networks, that's a Tor or VPN problem. Proteus rotating your MAC every two hours doesn't help against an adversary who can see both ends of your TCP flows.

### SSH client fingerprint (HASSH)

Your `ssh_config` is yours. Proteus does not edit `/etc/ssh/ssh_config` or `~/.ssh/config` — that would fight Fedora's `crypto-policies` (see [README](../README.md) "What it doesn't do") and surprise users in ways the tool can't predict. If you want a different HASSH, edit your config.

## Where Proteus sits

Roughly:

```
narrower ←                                                        → wider
macchanger   NM/iwd/networkd     PROTEUS         Tails / Whonix / Qubes
(one MAC)    (per-conn MAC)      (network        (whole-system privacy)
                                  identity)
```

Proteus assumes you want to keep using your daily-driver Linux laptop with NetworkManager and systemd, and you want the network-visible identifiers (L1 RF aside — that's hardware) to look different on a schedule. It does that and stops there.

For DNS, browsers, trackers, correlation, SSH, TLS — use the tools above. The wiki page `threat-model` (Phase F) will spell this out feature-by-feature so users don't over-trust Proteus.

## Further reading

- Wikipedia overview of MAC spoofing: [en.wikipedia.org/wiki/MAC_spoofing](https://en.wikipedia.org/wiki/MAC_spoofing)
- Linux kernel networking docs: [docs.kernel.org/networking](https://docs.kernel.org/networking/)
- GNOME NetworkManager wiki entry point: [wiki.gnome.org/Projects/NetworkManager](https://wiki.gnome.org/Projects/NetworkManager)
- Arch Wiki on iwd (good operational notes): [wiki.archlinux.org/title/Iwd](https://wiki.archlinux.org/title/Iwd)
- BlueZ source for the per-vendor BD_ADDR mess: [github.com/bluez/bluez](https://github.com/bluez/bluez)
- macchanger source for the OUI-vendor list approach: [github.com/alobbs/macchanger](https://github.com/alobbs/macchanger)
