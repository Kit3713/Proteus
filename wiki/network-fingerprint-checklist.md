A pre-flight inventory of every network-fingerprint surface and what addresses each. The page someone reads to understand what Proteus actually covers at a glance, with honest pointers to the per-feature pages and to the out-of-scope tools you need beside it.

For the underlying threat model, read `proteus wiki threat-model` first. For the full per-environment playbook, `proteus wiki hostile-environments`. For routine operation, `proteus wiki security-checklist`.

## The reference table

Every leak Proteus knows about, what mitigates it, and the tool or command that does the work. "Planned" means the work is on the roadmap; "pending PR" means a branch exists and is being merged; "out of scope" means a different tool owns this layer and Proteus refuses to overstep.

| Layer       | What leaks                  | Mitigation             | Tool/command                          |
|-------------|-----------------------------|------------------------|---------------------------------------|
| L1 RF       | Analog characteristics      | Hardware swap          | USB Wi-Fi                             |
| L1 RF       | Capture radius              | TX power               | `proteus rf` (planned)                |
| L2 MAC      | Wi-Fi/Eth MAC               | Rotation               | `proteus rotate`                      |
| L2 BT       | Bluetooth alias             | Generic alias          | `proteus bluetooth apply`             |
| L2 BT       | BLE address                 | RPA mode               | `proteus bluetooth apply`             |
| L2 BT       | Probe requests              | Per-scan random        | `proteus wifi-privacy` (planned)      |
| L3 IPv4     | DHCP options                | Suppression            | `proteus dhcp apply` (pending PR)     |
| L3 IPv6     | EUI-64 IID                  | stable-privacy         | `proteus ipv6 apply`                  |
| L3 IPv6     | Persistent DUID             | Link-layer DUID        | `proteus ipv6 apply`                  |
| L3-L4       | TCP timestamps              | Disable                | `proteus stack apply` (pending PR)    |
| L3-L4       | ICMP info-leaks             | nft drops              | `proteus nft apply` (pending PR)      |
| Discovery   | mDNS/LLMNR/NetBIOS          | Silence                | `proteus discovery apply` (planned)   |
| App-net     | Hostname                    | Rotation               | `proteus hostname apply`              |
| App         | DNS resolution              | Out of scope           | dnscrypt-proxy / Pi-hole              |
| App         | TLS fingerprint             | Out of scope           | Tor Browser                           |
| App         | App fingerprint             | Out of scope           | Browser sandbox                       |

The table is the page. The narrative below expands each row with the concrete leak, the concrete mitigation, and the failure modes worth knowing.

## L1 RF: analog characteristics

Every Wi-Fi and Bluetooth radio has unique analog imperfections — oscillator drift, IQ imbalance, transient power-spectrum at packet start. A passive SDR-equipped adversary close enough to capture clean signal can fingerprint your specific chip below the protocol layer. Software cannot fix analog hardware.

The real mitigation is a swappable USB Wi-Fi adapter. Different radio, different signature. Cheap cards are fine — you are buying RF identity, not throughput.

Cross-ref `proteus wiki rf-fingerprinting` for what the technique can and cannot do, and which adversary tier it actually applies to.

## L1 RF: capture radius

Independent of the analog signature, the audience that can hear you cleanly is bounded by your transmit power. Reducing TX power narrows the radius — does not change the signature, just shrinks who can read it.

Proteus offers an opt-in `wifi.tx-power-reduce` knob landing in the `proteus rf` subcommand. Status is "planned"; the underlying NetworkManager and `iw` knobs work today, the unified subcommand does not yet.

Treat this as defense-in-depth, not a defeat for RF fingerprinting. A determined attacker with a better antenna still hears you. Cross-ref `proteus wiki rf-fingerprinting`.

## L2 MAC: Wi-Fi and Ethernet

The single largest passive-tracking surface on a moving laptop. Stores, conferences, airports, and analytics aggregators key on MAC for cross-visit correlation. Proteus's primary defense.

`proteus rotate` produces a fresh MAC on demand. Two timers (`proteus-rotate.timer`, default 2h, and `proteus-check.timer`, default 5m for probe-driven loss) drive periodic rotation. NetworkManager's `wifi.cloned-mac-address` is the wire-side mechanism; rtnetlink is the fallback for non-NM-managed interfaces.

OUI realism matters: Proteus draws from Apple, Intel, Samsung, Dell, or random pools and biases toward your chipset's plausible vendor. An Apple OUI on an Intel chip is its own fingerprint.

Cross-ref `proteus wiki mac-recipes` for the rotation patterns, OUI pools, pinning, and the captive-portal interactions.

## L2 Bluetooth: alias

The discoverable name your Bluetooth adapter advertises is the system hostname by default, which leaks the hostname Proteus is otherwise scrubbing. Anyone within Bluetooth range during pairing or while the adapter is discoverable reads it.

`proteus bluetooth apply` replaces the alias with a generic string (`BT Device` by default) and sets `Discoverable=false`. The latter means inquiry scans from random nearby devices do not see you at all unless you flip discoverable on temporarily for pairing.

Cross-ref `proteus wiki bluetooth` for the BlueZ DBus mechanics and the BR/EDR-versus-BLE split.

## L2 Bluetooth: BLE address

BLE devices broadcast an advertising address while idle. By default this is the controller's static public address — a stable identifier readable by any BLE-aware device in range.

`proteus bluetooth apply` enables Resolvable Private Address (RPA) mode where the controller supports it. The controller rotates the on-air address every ~15 minutes; devices that hold your Identity Resolving Key (paired devices) can resolve back to a stable identity, the rest cannot.

BR/EDR (classic) BD_ADDR is fixed in v1 — vendor-specific HCI commands with bricking risk. The honest answer for classic Bluetooth is "turn the radio off when you do not need it". Cross-ref `proteus wiki bluetooth` and the threat-model page.

## L2 Bluetooth: Wi-Fi probe requests

Distinct from Bluetooth probes — a Wi-Fi card with saved networks emits probe requests carrying the SSIDs it knows, even before associating. A passive listener with a card in monitor mode reads the SSID list of every laptop in the room.

Modern kernels randomize the source MAC of pre-association probe frames; the SSIDs themselves remain in cleartext. The mitigation is `mac_addr=2` in wpa_supplicant (per-scan random source MAC) plus operational hygiene around the saved-networks list.

`proteus wifi-privacy` will land as a unified subcommand for the supplicant-level knobs and a saved-network-audit helper. Status is "planned". Until then, the mitigation is manual: cross-ref `proteus wiki wpa-supplicant-hardening` for the supplicant config and `proteus wiki hostile-environments` for the saved-networks discussion.

## L3 IPv4: DHCP options

DHCP DISCOVER and REQUEST broadcasts carry option 12 (hostname), 60 (vendor class identifier — `dhcpcd-9.4.1` is a banner), 61 (client identifier, often the MAC), 81 (FQDN). Even with a fresh MAC every join, the option payload can correlate sessions back to the same device, OS, or human.

`proteus dhcp apply` writes per-NM-connection settings to suppress 12/60/81 and couple 61 plus DHCPv6 DUID to the current MAC. Status is "pending PR" — the work is on `phase-d/dhcp-suppression` and merging soon.

The DUID coupling is the load-bearing piece. Without it, the v6 client ID is stable across MAC rotations and silently undoes most of the MAC-rotation point. Cross-ref `proteus wiki dhcp` for the option-by-option breakdown and the verification recipe.

## L3 IPv6: EUI-64 IID

The legacy IPv6 Interface Identifier derives directly from the MAC by inserting `ff:fe` in the middle and flipping the universal/local bit. The result: the IID leaks the MAC, every IPv6 packet carries it, MAC rotation alone does nothing.

`proteus ipv6 apply` enables RFC 7217 stable-privacy addressing. The IID derives deterministically from a network-scoped key plus the MAC; rotate the MAC and the IID rotates with no extra action. Temp addresses (RFC 8981) are flushed on the same boundary.

If you have manually disabled stable-privacy on an interface, Proteus surfaces which mode is active in `proteus status`. Cross-ref `proteus wiki ipv6` for the address-mode story and the kernel sysctl knobs.

## L3 IPv6: persistent DUID

DHCPv6 client identity. Persistent across MAC rotations by default — spec-blessed and fingerprint-grade. A static DUID defeats MAC rotation.

`proteus ipv6 apply` (and the DHCP work) sets NM's `ipv6.dhcp-duid = "ll"` for a link-layer DUID derived from the current MAC. Per-interface scope, smaller blast radius if a DHCPv6 server caches aggressively.

Cross-ref `proteus wiki ipv6` and `proteus wiki dhcp`.

## L3-L4: TCP timestamps

RFC 7323 timestamps carry a 32-bit value derived from a per-boot monotonic clock. The clock origin leaks system uptime; the timestamp itself is unique per host on the segment. Survives MAC rotation, DHCP scrubbing, and most VPNs.

`proteus stack apply` writes a sysctl drop-in setting `net.ipv4.tcp_timestamps = 0`. Status is "pending PR" on `phase-e/stack-sysctl`.

PAWS edge case (long-lived high-bandwidth flows) is documented; if you are moving terabytes over one connection, keep them on. Cross-ref `proteus wiki stack-fingerprint`.

## L3-L4: ICMP info-leaks

ICMP type 13/14 (Timestamp Request/Reply) leaks the system clock. Type 15/16 (Information Request/Reply) is a pre-DHCP-era discovery vector. Reply 16 leaks the subnet mask. Nothing in modern userspace asks for these; many kernels still answer.

`proteus nft apply` installs nft drop rules in the `proteus` table for ICMP types 13 and 15 inbound on managed interfaces. ICMPv6 Redirect drops via per-interface sysctl. Status is "pending PR" on `phase-e/nft-rules`.

Cross-ref `proteus wiki stack-fingerprint`.

## Discovery: mDNS, LLMNR, NetBIOS, SSDP, WSD

When you join a network, your machine starts talking. mDNS announces `<host>.local`, LLMNR asks the LAN to resolve names, NetBIOS broadcasts on UDP 137, SSDP shouts UPnP capabilities, WSD advertises Web Services for Devices. Each one carries identifying info.

`proteus discovery apply` is the unified subcommand: systemd-resolved drop-in for `MulticastDNS=resolve` and `LLMNR=no`, nmbd disable, optional SSDP and WSD blocks via nft. Status is "planned" — much of the work exists in branches today.

SSDP and WSD blocks are opt-in because they break KDE Connect and WSD-only printers respectively. Honest defaults: silent where there is no breakage, opt-in where there is. Cross-ref `proteus wiki discovery` for the protocol-by-protocol breakdown and the breakage matrix.

## App-net: hostname

`hostname1` over DBus controls kernel hostname, pretty hostname, and transient hostname. Default is whatever you set during install (often your name, your machine's purpose, or your distro's default). Joins of public networks broadcast the hostname through DHCP option 12, mDNS, NetBIOS, and any application that includes the hostname in its outgoing identity (some HTTP user-agents do, some IRC clients do, some chat apps do).

`proteus hostname apply` rotates kernel/pretty/transient names from a curated 534-entry router-flavored wordlist, or pins to a generic (`fedora`). The DHCP suppression work covers the wire-side leak independently; this rotates the underlying value so any leak that escapes suppression still produces something uncorrelated.

Cross-ref `proteus wiki hostname-recipes` for the rotation patterns and the wordlist.

## App: DNS resolution

The big one Proteus deliberately does not own. DNS resolution policy — encrypted transport (DoT/DoH/DNSCrypt), tracker blocking, custom resolver selection, DNSSEC validation, local caching — is its own complex world that deserves its own tooling.

Proteus has exactly one DNS knob: strip EDNS Client Subnet on systemd-resolved when systemd-resolved is the active resolver and no other DNS-privacy tool is detected. If `dnscrypt-proxy`, Pi-hole, AdGuard Home, NextDNS, or a custom `/etc/resolv.conf` is present, Proteus refuses to apply, names what it found in `proteus status`, and exits clean.

Use **dnscrypt-proxy** for DoH/DoT/DNSCrypt with anonymized relays. **NextDNS** for cloud-side filtering. **AdGuard Home** or **Pi-hole** for self-hosted network-wide. **knot-resolver** for a local validating recursor. Pick one and let Proteus's ECS-strip defer to it.

Cross-ref `proteus wiki dns` for the detect-and-defer rule and the threat-model page for the boundary discussion.

## App: TLS fingerprint

JA3, JA4, and other TLS ClientHello hashes. A passive observer hashes the cipher suites, extensions, and elliptic curves your TLS library negotiates; the result identifies the library, version, and often the specific application.

Software-impossible to normalize from outside the application process. Different libraries (NSS in Firefox, BoringSSL in Chrome, rustls, OpenSSL, GnuTLS, Go's crypto/tls) emit different ClientHellos. Applications choose their library.

Use **Tor Browser** for the gold-standard anti-fingerprinting plus Tor as the correlation defense. **Mullvad Browser** is Tor Browser without Tor — useful for low-latency anti-fingerprinting. **LibreWolf** with `privacy.resistFingerprinting`. **Brave** with farbling. Each owns the browser process. Proteus does not, and any attempt to fake it from outside would either fail or break things.

Cross-ref `proteus wiki threat-model`.

## App: app fingerprint

Canvas, WebGL, fonts, screen resolution, language, plugins, audio context, JS engine quirks, every API the browser exposes to a script. Solved from inside the browser process by tools that own the rendering pipeline. The host stack cannot lie convincingly to JavaScript about what GPU it has.

Use **Tor Browser**, **Mullvad Browser**, **LibreWolf**, or **Brave** with farbling — same list as TLS, for the same reason. Browser-layer concern.

Account-layer identifiers (cookies, OAuth tokens, app installation IDs, advertising IDs) are also application-layer. Use separate browser profiles per identity, do not reuse accounts across compartments. No network tool can paper over reusing the same Google account across two MACs you wanted to be uncorrelated.

Cross-ref `proteus wiki threat-model` for the full out-of-scope discussion and the composition story.

## How to use this page

Two ways.

- **Pre-flight before a hostile environment.** Walk the table top to bottom. Confirm each "applied" row is `applied` in `proteus status`. Confirm each "out of scope" row has its companion tool installed and configured. Cross-ref `proteus wiki hostile-environments` for the per-environment playbook.
- **Triage when something feels off.** Find the layer the leak is plausibly at. Read the row, the linked wiki page, and the verification recipe on that page. Most leaks resolve to either "Proteus's component is `failed` and needs a re-apply" or "this is on the out-of-scope list and a different tool needs to handle it".

If you find a row in this table that is wrong — the mitigation does not work, the command no longer exists, the link rots — that is a documentation bug. File it.

## What to install alongside Proteus

The composition story in concrete tool names. Pick one from each layer.

- **Browser fingerprint and TLS.** Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling on.
- **DNS resolution policy.** dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole, or knot-resolver.
- **Tracker blocking.** uBlock Origin in the browser; Pi-hole or AdGuard Home network-wide.
- **Traffic correlation.** Tor for low-volume sensitive browsing; Mullvad VPN or another reputable VPN for higher-volume.
- **Hardware second factor.** A Yubikey or SoloKey so 2FA is not tied to your phone number.
- **Account compartmentalization.** Firefox Multi-Account Containers, LibreWolf containers, or just separate profile directories.

Three concrete stacks worth naming:

- **Daily driver, public Wi-Fi.** Proteus on the laptop with default config. LibreWolf or Brave. uBlock Origin. NextDNS or dnscrypt-proxy. Mullvad VPN on for sensitive sessions. Hardware token. Covers the realistic everyday-threat tier.
- **Higher-stakes work, mixed environments.** Add Tor Browser for sensitive browsing alongside the daily browser. Compartmentalize accounts. USB Wi-Fi adapter for adversarial environments. `rfkill block bluetooth` as default. Journalism-on-controversial-topics or activist-organizing tier.
- **Hostile-state-actor.** Tails on a known-clean USB stick. Separate hardware. Tor with bridges (obfs4 or snowflake). No real-identity accounts. In-person key exchange. Proteus is one tool in this stack, not the kit.

## Cross-refs

- `proteus wiki threat-model` — the prerequisite discussion of what Proteus does and does not defend against
- `proteus wiki hostile-environments` — per-environment playbook (cafe, hotel, conference, airport, hostile state)
- `proteus wiki security-checklist` — daily/weekly/monthly routines for confirming Proteus is working
- `proteus wiki concepts` — the mental model behind identifiers, rotation triggers, and the no-silent-failures rule
- `proteus wiki mac-recipes` — the L2 MAC rotation patterns
- `proteus wiki bluetooth` — the L2 Bluetooth alias and BLE RPA story
- `proteus wiki dhcp` — the L3 IPv4 DHCP option suppression story
- `proteus wiki ipv6` — the L3 IPv6 stable-privacy and DUID story
- `proteus wiki stack-fingerprint` — the L3-L4 TCP and ICMP story
- `proteus wiki discovery` — the mDNS/LLMNR/NetBIOS/SSDP/WSD silencing story
- `proteus wiki hostname-recipes` — the hostname rotation story
- `proteus wiki dns` — the one ECS-strip knob and its detect-and-defer guard
- `proteus wiki rf-fingerprinting` — the L1 RF limit and the USB-adapter answer
- `proteus wiki wpa-supplicant-hardening` — supplicant-level knobs that compose with the L2 work
- `proteus wiki journald-network-logs` — the on-disk shadow of every wire-side rotation
