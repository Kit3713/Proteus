Read this before trusting Proteus with anything that matters. The point of the page is to under-promise so users don't over-trust the tool, don't assume out-of-scope features, and don't skip the other tools in their privacy stack because Proteus is installed.

Structure: scope, what Proteus resists, what Proteus does not resist (with the right tool to reach for instead), invariants Proteus refuses to weaken, composition with other tools, worked scenarios.

## Who this is for

Linux users on Fedora 43+ (or other modern systemd + NetworkManager systems) who want to reduce every fingerprint their laptop locally controls when joining or transmitting on a network — L2 identifiers, DHCP/IPv6/discovery chatter, TCP/ICMP/NDP stack quirks, Bluetooth radio policy, and the OS-controllable parts of the RF surface. Home Wi-Fi, public Wi-Fi, conference Wi-Fi, work Wi-Fi, hotel Wi-Fi.

Not a privacy panacea. Not a daily-driver replacement for Tor Browser, Mullvad, dnscrypt-proxy, Pi-hole, or any other tool that owns one problem well. A focused CLI for the network-joining identity layer, designed to compose with the rest.

If you need full anonymity, reach for Tails or Whonix. If you need traffic-correlation defense, reach for Tor or Mullvad. Proteus is the network-identity floor underneath those, not a replacement for them.

The audience this page assumes: someone who already understands MAC addresses, DHCP, and the broad shape of what their OS broadcasts when it joins a network. The `proteus wiki concepts` page is the prerequisite mental model.

## What Proteus is designed to resist

Each item below is a real, mostly-passive observation an attacker can make on a network you join, and Proteus's job is to make that observation either impossible or uncorrelated with your previous visits.

- **Passive L2 tracking on public Wi-Fi.** Stores, conferences, airports, malls, and transit hubs log MAC addresses for analytics, footfall counting, and behavioral profiling. Proteus rotates Wi-Fi and Ethernet MACs on a schedule (default 2h) and on probe-driven connectivity loss (default 5m), so each visit is uncorrelated with the last. Captive-portal SSIDs get a fresh MAC every visit.
- **DHCP correlation across networks.** DHCP option 12 (hostname), 60 (vendor-class identifier — `dhcpcd-9.4.1` is a banner), 61 (client identifier, often the MAC), and 81 (FQDN) tie your device across networks even when the MAC changes. Proteus suppresses or rotates each of these.
- **mDNS, LLMNR, NetBIOS, SSDP, WSD broadcasts on local networks.** These announce your hostname, capabilities, and services to anyone on the LAN. Proteus silences them, with SSDP and WSD opt-in because they break KDE Connect and WS-Discovery printers.
- **TCP timestamps that leak system uptime.** RFC 7323 timestamps are a monotonic clock anyone you talk to can read. Proteus disables them by default, with the documented PAWS edge case for high-bandwidth long-lived flows surfaced in the wiki.
- **ICMP info-replies and address-mask replies.** Old OS-fingerprinting vectors (ICMP types 15/16/17/18) that most kernels still answer. Proteus drops them.
- **EDNS Client Subnet on systemd-resolved.** Leaks a /24 prefix of your IP to upstream resolvers, which is enough to geolocate you. Proteus strips it — but only when systemd-resolved is the active resolver and no more-specialized DNS-privacy tool is detected.
- **Bluetooth adapter alias broadcast.** A passive observer near you can read the discoverable name your Bluetooth adapter advertises and link it to you. Proteus generic-aliases it and turns discoverable off by default.
- **BLE advertising address.** A passive observer can track BLE devices by their advertising address. Proteus enables Resolvable Private Address mode where the controller supports it.
- **IPv6 derivation correlation.** Under EUI-64 the IPv6 IID leaks the MAC directly; under stable-privacy it derives deterministically from the MAC plus a network-scoped key. Proteus rotates the IID alongside the MAC and uses temp addresses by default.
- **DUID stickiness across MAC rotations.** A static DHCPv6 client identifier defeats MAC rotation. Proteus rotates DUID alongside MAC.
- **OS-controllable RF surface.** Probe-request bursts naming every saved SSID, active-scan-when-passive-would-do behavior, and unconstrained TX power all leak identity beyond the L2 frame. Proteus tightens the supplicant's scan behavior, can reduce TX power on demand, and surfaces your chip + firmware inventory in `proteus status` so you can cross-reference the RF-fingerprinting research for your hardware. The hardware-analog half (oscillator drift, DAC nonlinearity, IQ imbalance) is documented separately under "what Proteus does not resist" — only a hardware swap fixes that.

If your concern is on the list above, Proteus is the right tool for that part of your stack. If your concern is not on the list, read the rest of this page before assuming Proteus helps; the right answer is almost always a different tool.

## The threat model in plain terms

Proteus assumes a passive listener. Someone — a Wi-Fi access point, a network operator, a local attacker with a packet capture, an analytics company aggregating store visits — is watching the L1 through L4 traffic the host emits when it joins a network and shortly after. They are not modifying your packets. They are not actively probing you. They are trying to recognize the host across visits and across networks.

The defense is to make the recognition fail. Different MAC every two hours. Different DHCP banner. Different hostname (or no hostname). No mDNS chatter to identify the OS. No TCP timestamp clock to read. No BLE address that matches across visits. Nothing in the packet stream that says "this is the same device that was here last Tuesday."

This is a useful threat model for ordinary public-Wi-Fi life. It is not the right threat model for surveillance-state evasion, for journalism in hostile environments, or for anything that requires anonymity rather than non-correlation. For those, you need Tor, you need careful operational security, and you need a wider toolset than Proteus.

## TLS and browser fingerprinting

These are real fingerprinting surfaces. Proteus does not address them and never will. They are out of scope by design.

- **JA3, JA4, and other TLS ClientHello fingerprints.** A hash of the cipher suites, extensions, and elliptic curves your TLS library negotiates. Software-impossible to normalize from outside the application process. Different libraries (NSS in Firefox, BoringSSL in Chrome, rustls, OpenSSL, GnuTLS, Go's crypto/tls) emit different ClientHellos. Applications choose their library. The host can't normalize across them without a man-in-the-middle proxy that re-handshakes every TLS session, which is its own large can of worms.
- **Browser fingerprinting.** Canvas, WebGL, fonts, screen resolution, language, plugins, audio context, JS engine quirks, every API the browser exposes to a script. Solved from inside the browser process by tools that own the rendering pipeline. The host stack cannot lie convincingly to JavaScript about what GPU it has.
- **What to use.** Tor Browser is the gold standard — full anti-fingerprinting plus Tor as the correlation defense. Mullvad Browser is Tor Browser without Tor, useful when you want the fingerprint resistance without the latency. LibreWolf is a Firefox fork with `privacy.resistFingerprinting` on by default. Brave has built-in randomization (farbling) for Canvas, WebGL, audio, and fonts. These tools own the browser process. Proteus does not, and any attempt to fake it from outside would either fail or break things.

## SSH client fingerprint (HASSH)

- **HASSH** is a hash of your SSH client's KEX algorithms, ciphers, MACs, and compression algorithms in their negotiated order. It identifies your client and version reliably enough to fingerprint individual users across SSH sessions, regardless of MAC or IP changes.
- **What to use.** Edit your `~/.ssh/config` (or per-host blocks) and set explicit `KexAlgorithms`, `Ciphers`, `MACs`, and `HostKeyAlgorithms`. Your SSH config is yours. Proteus refuses to touch `/etc/ssh/ssh_config` because doing so would fight Fedora's `crypto-policies` and surprise users in ways the tool can't predict. If two users share an SSH config, they share a HASSH; that is the level of normalization the SSH config layer can give you.

## DNS resolution policy beyond ECS-strip

Proteus has exactly one DNS knob — strip EDNS Client Subnet from systemd-resolved when systemd-resolved is the active resolver and no other DNS-privacy tool is detected. Everything else about DNS is out of scope.

- **Encrypted DNS (DoT, DoH, DNSCrypt).** Proteus does not configure your DNS resolver to use encrypted transport.
- **Tracker blocking via DNS.** Proteus does not block trackers at the DNS layer.
- **Local DNS caching.** Proteus does not add a local cache.
- **DNSSEC validation.** Proteus does not turn DNSSEC on or off.
- **Custom resolver selection.** Proteus does not pick your upstream resolver.
- **What to use.** dnscrypt-proxy (DoH/DoT/DNSCrypt with anonymized relays). NextDNS (commercial DoH/DoT with per-profile policy). AdGuard Home (self-hosted network-wide DNS filter). Pi-hole (the original network-wide DNS sinkhole). knot-resolver (modern caching validating recursor). Each does DNS far better than Proteus ever could. The detect-and-defer rule applies: if any of the above are present, Proteus's ECS-strip refuses to apply, names what it found in `proteus status`, and exits clean. Your DNS setup wins, every time. See `proteus wiki dns`.

## Tracker blocking in app traffic

- Tracking pixels, advertising calls, analytics beacons, telemetry, OAuth-tracked redirects, and other identifiers ride inside HTTP(S) traffic. Network-layer tools can't see inside encrypted application flows.
- **What to use.** Pi-hole or AdGuard Home for network-wide blocking. NextDNS for cloud blocking. uBlock Origin in the browser. Proteus operates at L1-L4 and the network-joining protocols. Application-layer trackers are a content-blocker problem.

## Traffic correlation

- Timing analysis, packet-size analysis, traffic-flow correlation. A passive observer with visibility on both ends of a flow can sometimes link your traffic to you across rotated MACs by analyzing patterns alone. A global passive adversary is even more effective. Rotating your L2 identity does not help against an adversary who can see both ends of your TCP flows.
- **What to use.** Tor Browser for low-volume web traffic — adds a multi-hop circuit with thousands of other users sharing the path. Mullvad VPN or another reputable VPN for higher-volume traffic — single hop, but adds a middlebox between you and the destination and aggregates your traffic with that of other users on the same exit. Proteus rotating your MAC every two hours does nothing against an adversary watching both ends.

## RF L1 fingerprinting (analog hardware characteristics)

- Every Wi-Fi radio has unique analog quirks — clock-skew offsets, IQ imbalance, transmit power ramp shapes, frequency-error patterns, intermodulation products. These are physical-layer fingerprints that survive every software-level identifier change. Software cannot fix analog hardware quirks. Cross-ref `proteus wiki rf-fingerprinting` for the longer write-up.
- **What to use.** A swappable USB Wi-Fi adapter is the only real answer — change the radio, change the fingerprint. Proteus's only contribution is opt-in TX power reduction (`wifi.tx-power-reduce`), which narrows the capture radius for passive listeners. Smaller capture radius is not invisibility; a determined attacker with a better antenna can still hear you. Treat TX power reduction as a small defense-in-depth knob, not as a defeat for RF fingerprinting.

## Bluetooth BR/EDR (classic) BD_ADDR rotation

- Per-vendor HCI commands. There is no standard. The BlueZ `bdaddr` tool has separate code paths for CSR, TI, Ericsson, Zeevo, ST, and Marvell, and falls through with "device not supported" for everything else. Some chips reject the command after first boot, some lose calibration data, some need a controller reset that confuses the host stack. Real bricking risk. Cross-ref `proteus wiki bluetooth`.
- **What to use.** Use BLE-only devices where possible — BLE has Resolvable Private Address mode, which Proteus enables. For BR/EDR, accept the BD_ADDR is fixed; rely on Proteus's adapter alias rotation and `discoverable=off` defaults to limit the surface area. BR/EDR rotation may land in a future Proteus version once there's a known-good chipset matrix; it is deliberately not a v1 commitment.

## Application-layer identifiers

- Cookies, account logins, browser local storage, IndexedDB, cache entries, app installation IDs, advertising IDs (IDFA / GAID equivalents), OAuth tokens, session JWTs, Matrix device IDs, XMPP resource strings, and the long tail of per-app identity. Network-layer rotation does nothing about any of these.
- **What to use.** Separate browser profiles per identity. Don't reuse accounts across identities. LibreWolf containers or Firefox Multi-Account Containers for in-browser separation. Private browsing mode for one-off sessions (limited — ephemeral storage only, doesn't change your account state). Treat your account boundaries as the application-layer identity boundary they are. No network tool can paper over reusing the same Google account across two MACs you wanted to be uncorrelated.

## `/etc/machine-id` rotation

- TPM, journald, dbus, systemd-networkd's `persistent` MAC derivation, and a long tail of other daemons all reference `/etc/machine-id`. Rotation breaks systemd's bootchart, journald rotation tracking, dbus session resumption, and miscellaneous daemons that cache it. Real breakage risk including potential boot failure or daemon misbehavior.
- **Why we don't.** The reward (one more identifier rotated) does not justify the cost. Changing machine-id silently breaks too much. The identifier doesn't leave the host except when an application chooses to send it, and the things that do leak it (some D-Bus interfaces, some application telemetry) are application-layer problems. If your threat model genuinely requires a fresh machine-id, you want a fresh install — not a `dd` over the file.

## Hardware-level supply-chain compromise

- Backdoors in chipset firmware. Malicious USB devices. Bad SMM, bad ME, bad PSP. UEFI-resident malware. Hardware implants on the motherboard. Compromised cellular modems on laptops with built-in WWAN.
- **Out of scope entirely.** Software cannot audit hardware it runs on. If your threat model includes hardware compromise, you need hardware countermeasures (verified-boot, physical inspection, air-gapped systems, hardware that you trust the supply chain of), not a network-identity CLI. Proteus running on compromised hardware is not safer than no Proteus.

## Targeted active attackers

- Someone with admin access to your network, or with the ability to actively probe your client, or with court-order access to your ISP, can deanonymize you regardless of what Proteus does. So can a skilled local attacker with the right hardware and time on their hands. Active fingerprinting (sending crafted packets and reading responses) is more powerful than the passive listener model Proteus assumes.
- **What Proteus is for.** Passive-listener and casual-correlation defense. The goal is to make routine, automated tracking ineffective — not to stop a determined adversary who is specifically interested in you. If you are being personally targeted, you need operational security advice, a wider toolset (Tor, compartmentalization, an air gap), and probably a lawyer. Proteus is for everyone else: the people who don't want every coffee shop and conference Wi-Fi to recognize them across visits.

## Deferred to a future version

Things Proteus could plausibly do but does not in v1. Listed here so you do not assume they exist.

- **Per-SSID profiles.** Different rotation cadence, hostname policy, or DHCP option set per network. The config schema reserves the namespace. Useful for "treat home Wi-Fi differently from the airport." Deferred to v2.
- **Bluetooth BR/EDR (classic) BD_ADDR rotation.** Per-vendor HCI commands with bricking risk. Deferred until there is a known-good chipset matrix. See the Bluetooth section above.
- **macOS / Windows ports.** The CLI, config, and wiki layers stay portable thanks to the `Platform` trait. No commitment, no v1 work.
- **A GUI.** Proteus is CLI-first. The CLI is designed to be wrappable so someone can build a GUI later without forking.
- **Telemetry, update checks, analytics.** Never. Not deferred — actively refused. Proteus does not phone home.
- **A "stealth" mode that disables logging.** No. Proteus logs to journald so you can audit what it did. Logging is a feature, not a leak.

## Failure modes you should expect

Proteus is not magic. Here are the realistic ways it does not deliver on its threat model, and what to do about each.

- **You log in to the same account from two MACs.** The application correlates you across MAC rotations by your account. Proteus cannot help. Use account boundaries that match your identity boundaries.
- **You join the same captive-portal SSID twice without it being on the known-portal list.** The first visit's MAC may be reused by the portal's session cookie. Mark the SSID as a known portal so Proteus rotates per visit.
- **You leave Bluetooth discoverable on for pairing.** Anyone in range during that window sees the (now generic-aliased) name and the BD_ADDR. Proteus's `discoverable=off` default means you have to actively re-enable it; turn it back off when pairing is done.
- **You're behind a corporate enterprise-Wi-Fi network with 802.1X.** Your inner identity (the username) authenticates you regardless of MAC. Proteus's anonymous outer identity feature is opt-in and may be rejected by some corporate auth servers; if your IT department recognizes you by username, no MAC change helps. This is by design.
- **You use NTP without checking what your client sends.** Proteus normalizes systemd-timesyncd config (deferring if chrony or ntpd is installed) but cannot stop chrony or ntpd from emitting their own client signature. If you care, use systemd-timesyncd or accept the leak.
- **The system has multiple radios (Wi-Fi + WWAN).** Proteus targets Wi-Fi and Ethernet by default. Cellular WWAN modems have their own identifiers (IMSI, IMEI) that Proteus does not touch. Use airplane mode for cellular if it is in scope for you.
- **You manually edit a file Proteus manages.** Proteus's managed files carry a SHA header; `proteus diff` flags drift from the expected content. If you edit a managed file directly, your change is preserved until the next `proteus apply`, at which point Proteus either re-asserts its content (default) or warns and skips depending on your config.
- **You expect Proteus to fix a problem on a network you don't control.** Proteus controls what the host emits. It does not control what the network sees of other devices, what the access point logs, or what an upstream ISP collects. Network-side adversaries with control of the infrastructure are a different threat class.

## How to verify Proteus is doing what it claims

Trust but verify. The `proteus wiki verifying` page has detailed recipes; the gist:

- `tcpdump -i wlan0 -nn 'port 67 or port 68'` — watch your DHCP requests. Confirm option 12, 60, 61, 81 are absent or rotated.
- `avahi-browse -ar` from another host on the same LAN — confirm the host is not announcing services.
- `nmap -O <your-ip>` from another host — see what OS fingerprint your stack still leaks. Compare before and after `proteus apply`.
- `proteus current` — show what Proteus thinks the current identifiers are.
- `proteus status` — show per-feature `applied / skipped (reason) / failed (reason)`.

If any of these shows a leak Proteus claims to suppress, that is a bug. File it.

## Hardening invariants Proteus refuses to weaken

Some lines Proteus will not cross, even if you ask. These exist because the cost of breaking them is unbounded — silent failures in security-critical subsystems are worse than no feature at all.

- **Crypto-policies (`update-crypto-policies`).** Never touched. Proteus does not write to `/etc/ssl/openssl.cnf`, does not override system-wide cipher choices, does not weaken Fedora's hardening defaults. If your TLS or SSH crypto policy needs changing, use the system tool.
- **`/etc/ssh/ssh_config`.** Never touched. Your SSH client config is yours. See the HASSH section above.
- **`/etc/ssh/sshd_config`.** Never touched. Your SSH server config is yours.
- **`/etc/machine-id`.** Never rotated. See the section above.
- **NetworkManager defaults that protect security.** Proteus adds identity-rotation knobs on top of NM. It does not disable NM's security features, does not weaken WPA negotiation, does not turn off certificate validation for enterprise Wi-Fi. The 802.1X anonymous outer identity feature is opt-in, default off, because some corporate auth servers reject mismatched outer identities.

If you find Proteus doing any of the above, that is a bug. File it.

## Composition with other tools

Proteus is one layer in a privacy stack, never the whole stack. It owns L1 (RF capture radius via opt-in TX power reduction), L2 (MAC), L3 (IPv6 derivations, DUID), L3-L4 (TCP timestamps, ICMP), and the network-joining protocols (DHCP options, hostname, mDNS, LLMNR, NetBIOS, SSDP, WSD, BLE address). Cross-ref `proteus wiki concepts` for the full identifier list.

A complete personal-privacy setup composes several tools, each owning one problem well:

- **Network identity layer.** Proteus.
- **Browser fingerprint layer.** Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling on.
- **DNS layer.** dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole, or knot-resolver.
- **Content blocker layer.** uBlock Origin in the browser. Pi-hole or AdGuard Home network-wide.
- **Traffic correlation layer.** Tor for low-volume, Mullvad or another reputable VPN for higher-volume.
- **Account boundary layer.** Separate profiles per identity. Don't reuse accounts.
- **SSH client layer.** Your `~/.ssh/config`, with explicit algorithm choices.

Practical combinations:

- **Proteus + Tor Browser.** Best combination for low-volume privacy-sensitive browsing on public Wi-Fi. Proteus handles the network-join identity; Tor Browser handles browser fingerprint and traffic correlation.
- **Proteus + Mullvad VPN.** Common high-bandwidth setup. Proteus rotates the L2 identity at the edge; Mullvad masks your IP and aggregates your traffic with other Mullvad users. The Mullvad client itself emits a TLS ClientHello and a UDP fingerprint; that's a Mullvad concern.
- **Proteus + Pi-hole or dnscrypt-proxy.** Proteus's ECS-strip detects the upstream resolver and defers. Your DNS setup wins; Proteus does not interfere.
- **Proteus + LibreWolf or Brave.** Add browser-layer fingerprint resistance on top of Proteus's network-layer rotation. Each tool is unaware of the other and that is fine.
- **Proteus + a corporate VPN.** Proteus rotates the L2 identity before the VPN tunnel comes up. Inside the tunnel, your VPN client identifies you to your employer; Proteus does not interfere. If the VPN binds to a specific MAC, use `proteus pin` to freeze that interface's MAC for the duration.

What Proteus does not compose well with: tools that try to do the same thing. Don't run macchanger and Proteus together. Don't run two MAC-rotation timers. Don't have NetworkManager set to `random` cloned-mac-address while Proteus is also rotating; pick one source of truth (Proteus, by virtue of driving NM via dbus, takes that role).

No single tool covers everything. Pick tools that own one problem well, then compose them. That is the only honest answer.

## Worked scenarios

Three concrete scenarios, what Proteus does and does not cover, and what to add.

**Coffee-shop Wi-Fi, casual browsing.** Proteus rotates your MAC before joining if the SSID is a known captive portal, suppresses DHCP options 12/60/61/81, silences mDNS and LLMNR, drops the TCP timestamp leak, and disables ICMP info-replies. The shop's analytics platform can no longer link this visit to last week's visit by MAC. What Proteus does not cover: the website you log into recognizes you by cookie; your browser's TLS ClientHello is the same; your ad-network identifiers are unchanged. Add: a browser with anti-fingerprinting on, uBlock Origin or equivalent, and don't sign in to anything that ties this visit to your real identity.

**Conference Wi-Fi, working a few hours.** Proteus rotates your MAC every two hours by default. The conference's vendor-supplied analytics may still see two or three MAC presences from "your seat" but won't link them by hardware ID, hostname, or DHCP banner. What Proteus does not cover: your work email client's IMAP login authenticates with your work credentials. Your VPN client identifies the corporate endpoint. Your video-conferencing app sends a stable installation ID. Add: accept that the work tools identify you to your employer and to themselves; that's their job. Proteus is for the surrounding ambient identity, not the one you actively log in with.

**Hotel Wi-Fi, multi-night stay.** Many hotel captive portals bind your auth to your MAC for the duration of the stay. Proteus's default `rotate-before-auth` policy gives you a fresh MAC, which you then auth, which then becomes your stable identity for that stay (rotation is suppressed while authed). Across stays at different hotels of the same chain, your identity is uncorrelated. What Proteus does not cover: the hotel's loyalty-program app on your phone, the booking confirmation tied to your name. Add: nothing — the hotel knows your name from the reservation; Proteus doesn't claim to anonymize that.

## Documentation invariant

Privacy tools tend to over-claim. Proteus tries not to. Every feature in `proteus status` reports `applied / skipped (reason) / failed (reason)` so you know exactly what is and is not working on your system. Every detect-and-defer choice is named so you know which more-specialized tool Proteus stepped aside for. Every failure mode above is real; they are documented so you can plan for them rather than discover them.

If you find a place in Proteus's documentation, error messages, or marketing that over-claims relative to what the code actually does, that is a documentation bug. File it.

## Cross-refs

- `proteus wiki concepts` — Proteus's mental model: identifiers by layer, rotation triggers, captive portals, managed files, idempotency, no silent failures. The prerequisite for this page.
- `proteus wiki rf-fingerprinting` — RF L1 limits in detail. What a swappable USB adapter buys you and what TX power reduction does not.
- `proteus wiki bluetooth` — BR/EDR rotation limits in detail. The per-vendor HCI mess and why BLE RPA is the supported path.
- `proteus wiki dns` — the one ECS-strip knob and its hard guard. The detect-and-defer rule for dnscrypt-proxy, Pi-hole, AdGuard Home, custom resolv.conf.
- `proteus wiki verifying` — tcpdump, avahi-browse, and nmap recipes to confirm Proteus is doing what it claims. Lands in phase F.
