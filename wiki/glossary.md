Definitions of terms used throughout Proteus and its docs. Alphabetical, terse. Cross-refs point at the deeper wiki page where it exists or is planned.

## 802.1X
Port-based network access control, common in enterprise Wi-Fi (WPA-Enterprise / EAP). The supplicant authenticates before being granted network access. Proteus offers an opt-in anonymous outer identity for EAP methods that support it. See `proteus wiki enterprise-wifi`.

## Anonymous outer identity
The cleartext identity sent in the outer EAP tunnel before the encrypted inner authentication. Setting it to `anonymous@realm` (or similar) hides the real username from on-path observers. Opt-in and default off because some corporate auth servers reject mismatched outer identities. See `proteus wiki enterprise-wifi`.

## ARP table
The kernel's cache of IPv4 address to MAC address mappings on the local link. Proteus reads the ARP table during MAC rotation to avoid picking a MAC that collides with the gateway or another live host. See `proteus wiki mac-recipes`.

## Avahi
The most common mDNS / DNS-SD implementation on Linux. It runs as `avahi-daemon` and both responds to and queries multicast service discovery on `224.0.0.251`/`ff02::fb`. Proteus disables the local mDNS responder via systemd-resolved drop-in rather than touching Avahi directly when Avahi is the active stack. See `proteus wiki discovery`.

## BD_ADDR
Bluetooth Device Address, the 48-bit identifier of a Bluetooth Classic (BR/EDR) controller. Functionally a MAC address for Bluetooth. Rotating BD_ADDR is chipset-specific HCI-command territory and stays deferred in Proteus; only adapter alias and BLE addressing are touched in v1. See `proteus wiki bluetooth`.

## BLE (Bluetooth Low Energy)
The low-power variant of Bluetooth introduced in Bluetooth 4.0. Supports privacy mode with rotating Resolvable Private Addresses, which Proteus enables where the controller exposes it. See `proteus wiki bluetooth`.

## BlueZ
The Linux Bluetooth stack, exposed over D-Bus on `org.bluez`. Proteus talks to BlueZ via zbus to set the adapter alias, toggle discoverability, and configure BLE privacy. See `proteus wiki bluetooth`.

## BR/EDR (Bluetooth Classic)
Basic Rate / Enhanced Data Rate, the original Bluetooth radio mode used for headsets, file transfer, and HID. BD_ADDR rotation in BR/EDR is vendor-specific and not portable; Proteus defers it. See `proteus wiki bluetooth`.

## Captive portal
A network access gate, typically an HTTP intercept that holds traffic until the user authenticates through a web form. Proteus classifies portals as `clear` / `portal-required` / `portal-authed` / `unknown` and suppresses periodic rotation while authed to avoid logout loops. See `proteus wiki captive-portals`.

## ClientID (DHCP option 61)
A DHCP client identifier, often defaulting to the interface MAC and therefore correlating across MAC rotations. Proteus suppresses option 61 via NetworkManager so a rotated MAC is not undone by a stable client identifier. See `proteus wiki dhcp`.

## Detect-and-defer
Proteus's pattern of inspecting the host for a more specialized tool before acting, and bowing out cleanly when one is present. Used today for DNS (defer to dnscrypt-proxy / Pi-hole / AdGuard Home / custom resolv.conf) and NTP (defer to chrony / ntpd). The skipped feature is named in `proteus status`. See `proteus wiki concepts`.

## DHCPv4 / DHCPv6
The Dynamic Host Configuration Protocol families for IPv4 (RFC 2131) and IPv6 (RFC 8415). Both leak identity in their option payloads — hostname, vendor class, client ID, FQDN, DUID — which Proteus suppresses or rotates via NetworkManager. See `proteus wiki dhcp`.

## DUID (DHCPv6 Unique Identifier)
A persistent client identifier sent in DHCPv6 (RFC 8415 §11), sticky across reboots and even MAC rotations by default. Proteus rotates the DUID alongside the MAC so DHCPv6 cannot re-correlate the new identity to the old. See `proteus wiki dhcp` and `proteus wiki ipv6`.

## ECS (EDNS Client Subnet)
An EDNS(0) option (RFC 7871) that includes a prefix of the client's IP in DNS queries to upstream resolvers, ostensibly for geo-targeted CDN answers. It leaks subnet-level location to every authoritative server you transitively query. Proteus's one DNS knob strips ECS on systemd-resolved when no other DNS-privacy tool is detected. See `proteus wiki dns`.

## EUI-64
A 64-bit interface identifier derived directly from a 48-bit MAC by inserting `FF:FE` and flipping the universal/local bit (RFC 4291 Appendix A). The legacy IPv6 IID derivation; leaks the MAC into every IPv6 address. Replaced by stable-privacy (RFC 7217) in modern stacks. See `proteus wiki ipv6`.

## Fingerprint (in this project's sense)
A combination of network-layer identifiers and behaviors that distinguish your device across networks, sessions, or time. Proteus targets the network-joining slice of this surface — MAC, DHCP options, IPv6 derivations, hostname, mDNS, TCP quirks, Bluetooth name. Application-protocol fingerprints (TLS, SSH, browser) are explicitly out of scope. See `proteus wiki threat-model`.

## FQDN (DHCP option 81)
The Fully Qualified Domain Name option in DHCP (RFC 4702 / RFC 4704), letting the client tell the server its FQDN and request a corresponding DNS update. Proteus suppresses option 81 because it leaks hostname even when option 12 is silenced. See `proteus wiki dhcp`.

## Hostname (kernel / pretty / transient)
Three names systemd tracks separately. Kernel is `/proc/sys/kernel/hostname`, the value most networking code reads. Pretty is the human-readable label in `/etc/machine-info` (e.g. "Cam's ThinkPad"). Transient is set over DHCP and lives only until reboot. Proteus rotates all three over `org.freedesktop.hostname1`. See `proteus wiki hostname-recipes`.

## ICMP info-reply
ICMP message types 15/16 (Information Request/Reply) and 17/18 (Address Mask Request/Reply), legacy diagnostics that most kernels still answer. They are an old OS-fingerprinting vector. Proteus drops these via nftables. See `proteus wiki stack-fingerprint`.

## IID (Interface Identifier, IPv6)
The lower 64 bits of an IPv6 address — the per-interface portion. Derivation can be EUI-64 (leaks MAC), stable-privacy (RFC 7217, deterministic per network), or temporary (RFC 8981, rotated periodically). Proteus prefers stable-privacy plus temporary, with the IID rotated when the MAC rotates. See `proteus wiki ipv6`.

## IRK (Identity Resolving Key, BLE)
A 128-bit key shared between bonded BLE peers that lets them resolve each other's Resolvable Private Addresses to a stable identity (Bluetooth Core Spec, Volume 3, Part H). Bonded peers still recognize you across RPA rotations; unbonded scanners do not. See `proteus wiki bluetooth`.

## JA3 / JA4
TLS ClientHello fingerprint hashes, derived from the cipher list, extensions, elliptic curves, and EC point formats a client offers. Out of scope for Proteus — TLS stacks (NSS, BoringSSL, rustls, OpenSSL, GnuTLS) cannot be normalized from outside, and apps override anyway. Use Tor Browser, librewolf, or Brave for browser TLS randomization. See `proteus wiki threat-model`.

## LAA (Locally Administered Address)
A MAC with the second-least-significant bit of the first octet set to 1, signalling that the address is software-assigned rather than vendor-burned (IEEE 802 §3.2.3). Proteus's `locally-administered-random` OUI pool produces LAAs. See `proteus wiki mac-recipes`.

## LLMNR (Link-Local Multicast Name Resolution)
A Microsoft name-resolution protocol (RFC 4795) that multicasts queries on the link when DNS fails. Chatty, fingerprintable, and superseded by mDNS in practice. Proteus disables the LLMNR responder via systemd-resolved drop-in. See `proteus wiki discovery`.

## MAC address
The 48-bit hardware address of an Ethernet, Wi-Fi, or Bluetooth interface (IEEE 802). Upper 24 bits are the OUI (manufacturer); lower 24 identify the device. The most-fingerprinted thing on a laptop. Software-overridable on every modern Linux NIC. See `proteus wiki mac-recipes`.

## Managed file
Any file Proteus writes under `/etc/`. Carries a two-line header — `# managed by proteus — do not edit` and `# expected-sha256: <hex>` — so `proteus diff` can detect manual edits and either re-apply, accept the drift, or back out. See `proteus wiki concepts`.

## mDNS (Multicast DNS)
RFC 6762 multicast name resolution on the `.local` TLD, paired with DNS-SD (RFC 6763) for service discovery. Broadcasts both the hostname and a list of services the system offers. Proteus disables the local responder and resolver via systemd-resolved drop-in. See `proteus wiki discovery`.

## NetworkManager
The dominant Linux network configuration daemon. Proteus drives it over D-Bus via zbus and never shells out to `nmcli`. All MAC, DHCP, IPv6, and 802.1X changes go through NM connection profiles. See `proteus wiki concepts`.

## nft / nftables
The modern Linux packet-classification framework (`nft` is the userspace tool). Proteus uses nftables directly — or via firewalld where present — for ICMP info-reply drops, SSDP/WSD blocks (opt-in), and NetBIOS silencing. See `proteus wiki discovery` and `proteus wiki stack-fingerprint`.

## NM connection profile
A NetworkManager-managed configuration object representing a specific network (an SSID, a wired connection, a VPN). Stored under `/etc/NetworkManager/system-connections/`. Proteus's `pin` and most per-network settings operate at the connection-profile level rather than the interface level. See `proteus wiki mac-recipes`.

## OUI (Organizationally Unique Identifier)
The upper 24 bits of a MAC, assigned by the IEEE to a manufacturer. Proteus's randomization pools draw from real-vendor OUIs (Apple, Intel, Samsung, Dell) plus a locally-administered pool, so the rotated MAC blends rather than screaming "randomized". See `proteus wiki mac-recipes`.

## PAWS (Protection Against Wrapped Sequence numbers)
A TCP mechanism (RFC 7323 §5) that uses timestamps to disambiguate sequence numbers on long-lived high-bandwidth flows. Disabling TCP timestamps removes PAWS, which can hurt throughput on >1 Gbps long flows. The wiki page documents the edge case so users can opt back in. See `proteus wiki stack-fingerprint`.

## Permanent MAC
The vendor-burned MAC of a NIC, exposed by the kernel as the interface's permanent address (distinct from the currently-assigned address when overridden). Proteus snapshots the permanent MAC of every NIC the first time it sees the system and never re-captures it; the snapshot is the "sacred original". See `proteus wiki concepts`.

## Pinning (in Proteus's sense)
Freezing a specific MAC on an interface or NetworkManager connection profile via `proteus pin`, exempting it from both scheduled and probe-driven rotation. For environments that lock you to one MAC: corporate networks, hotel Wi-Fi after auth, MAC-bound DHCP reservations. Released with `proteus unpin`. See `proteus wiki mac-recipes`.

## Platform trait
The Rust trait abstracting all OS-specific operations — netlink, D-Bus, file paths, sysctl. Today only `LinuxPlatform` is implemented; a future macOS or Windows port would be a backend swap rather than a fork. See `proteus wiki internals`.

## Probe quorum
Proteus's connectivity-loss test: contact 4 known endpoints in parallel and declare "down" only when at least 3 fail. Single-endpoint flakiness does not trigger rotation. Cooldown of 60s after each rotation. See `proteus wiki probes`.

## Rotation (in Proteus's sense)
Replacing the current MAC (and coupled identifiers — DUID, IPv6 IID, optionally hostname) with a fresh value drawn from the configured OUI pool. Triggered on a 2h schedule, on probe-driven connectivity loss, on join to a known captive-portal SSID, or manually via `proteus rotate`. See `proteus wiki rotation`.

## RPA (Resolvable Private Address, BLE)
A BLE address generated from an Identity Resolving Key, rotated periodically by the controller (Bluetooth Core Spec). Bonded peers can resolve it back to your identity using the shared IRK; arbitrary scanners cannot. Proteus enables RPA mode where the controller supports it. See `proteus wiki bluetooth`.

## Sacred original
The cached permanent MAC and original hostname Proteus snapshots the first time it sees a system, stored in `/var/lib/proteus/state.json`. Never re-captured under any circumstance. `proteus reset` does not touch them; only `proteus uninstall --purge` clears them. The guarantee that you can always get back to your system's original identity. See `proteus wiki concepts`.

## SSDP (Simple Service Discovery Protocol)
The UPnP discovery protocol on UDP/1900, multicast to `239.255.255.250`. Used by KDE Connect, media servers, smart-home gear. Proteus blocks SSDP behind an opt-in flag (default off) because the block breaks KDE Connect. See `proteus wiki discovery`.

## Stable-privacy address (RFC 7217)
An IPv6 IID derived deterministically from the MAC plus a network-scoped secret key, so the IID stays stable per network but does not leak the MAC and does not correlate across networks. The default IID derivation in modern Linux stacks; Proteus prefers it over EUI-64. See `proteus wiki ipv6`.

## Stub command
Proteus's pattern for subcommands that parse with full clap help in earlier phases but exit with `not yet implemented in this phase, see phase X` until their phase ships. Lets the CLI surface stay stable from phase A. See `proteus wiki cli`.

## systemd-resolved
The systemd DNS resolver and stub listener on `127.0.0.53`. Proteus's ECS-strip and mDNS/LLMNR disables ship as drop-ins under `/etc/systemd/resolved.conf.d/`, and only when no other resolver tool is detected. See `proteus wiki dns` and `proteus wiki discovery`.

## systemd-timesyncd
The systemd SNTP client, default NTP implementation on most modern systemd distros. Proteus normalizes its config via a drop-in to avoid leaking NTP-client signatures, but skips entirely when `chrony` or `ntpd` is installed. Same detect-and-defer pattern as DNS. See `proteus wiki stack-fingerprint`.

## TCP timestamps option
A TCP option (RFC 7323 §3) that lets endpoints exchange timestamps, used for RTT estimation and PAWS. The local timestamp is monotonic and leaks system uptime. Proteus disables it via sysctl by default; the PAWS edge case is documented. See `proteus wiki stack-fingerprint`.

## Temporary IPv6 address (RFC 4941/8981)
A short-lived IPv6 address with a randomized IID, used for outgoing connections and rotated periodically (default daily). Used alongside the stable-privacy address: stable for incoming, temporary for outgoing. Enabled by default in modern Linux. See `proteus wiki ipv6`.

## Vendor Class Identifier (DHCP option 60)
A DHCP option (RFC 2132 §9.13) that lets the client advertise its DHCP implementation — strings like `dhcpcd-9.4.1` or `MSFT 5.0` — usable as both a fingerprint and a version-tracking signal. Proteus suppresses it via NetworkManager. See `proteus wiki dhcp`.

## Wordlist
The curated list of about 500 router-flavored hostname candidates Proteus draws from when rotating the hostname (e.g. `linksys-3a`, `netgear-orbi`, `tplink-archer`). Alternative is a generic-default option (`fedora`) or a user-provided list. See `proteus wiki hostname-recipes`.

## WPAD (Web Proxy Auto-Discovery)
A protocol for clients to find an HTTP proxy via DHCP option 252 or DNS lookup of `wpad.<domain>`. A long-standing exfiltration vector and a fingerprint when enabled. Proteus disables WPAD via NetworkManager. See `proteus wiki discovery`.

## WSD (Web Services for Devices)
Microsoft's discovery protocol for printers and similar devices, on UDP/3702. Proteus blocks WSD behind an opt-in flag (default off) because the block breaks WS-Discovery printers. See `proteus wiki discovery`.

## zbus
A pure-Rust async D-Bus client crate. Proteus uses zbus to talk to NetworkManager, BlueZ, hostname1, and timedate1 without shelling out to `dbus-send` or distro CLIs. Keeps the binary small and the failure modes legible. See `proteus wiki internals`.
