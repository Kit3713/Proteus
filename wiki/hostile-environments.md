A field guide for using Proteus where the network is actively trying to identify you. Cafes and hotels and conferences and airports and the worse end of the spectrum. This page is honest about what Proteus does for you in those places, what it cannot do, and which other tools you need beside it.

Read `proteus wiki threat-model` first if you have not. This page is the operational follow-up to that one. The threat-model page covers what Proteus is and is not for; this one is the playbook for using what Proteus is for in the places it matters most.

The voice on this page is deliberately under-promising. Network-layer fingerprint erasure is one piece of a privacy stack; running Proteus and assuming it makes you anonymous is the most common way users hurt themselves. Proteus is the floor underneath the rest of the stack — it makes the network-side leak stop being the weakest link. That is a useful and specific thing. It is not the same as anonymity.

## What "hostile environment" means

A network you do not control where someone — the venue operator, an analytics vendor, a state actor, a casual local attacker — is trying to recognize your device across visits, link your behavior to your identity, or both. Five archetypes, listed in roughly increasing threat level.

- **Coffee shop Wi-Fi.** A retail point-of-presence wired into a vendor portal that exists to count footfall, build presence histories, and sell that data. Your MAC is the join key. Bluetooth scanning beacons are common. The Wi-Fi controller logs every probe request whether you join or not.
- **Hotel networks.** Per-MAC charging on some chains, captive portal binding your room number to your MAC for the duration of the stay, marketing trackers downstream of the portal. Multi-night stays compound the surface.
- **Conferences.** Vendor-supplied analytics for attendee tracking, peer-snooping by other attendees with `airodump-ng` open on a laptop, sponsor-booth Bluetooth beacons collecting BD_ADDRs in exchange for trivial swag. The crowd density makes correlation easier, not harder.
- **Airports.** Conference-grade ambient surveillance plus law-enforcement passive collection at major hubs and across borders. Free Wi-Fi at the terminal is run by an analytics vendor; the lounge Wi-Fi is run by a different one. Captive portals demand an email. Bluetooth pings every gate.
- **Hostile state actors.** Border crossings, repressive networks, journalism in non-permissive environments, surveillance states. Proteus is necessary but not sufficient — you need the rest of the stack and operational discipline. See the dedicated section below.

The common thread: you are joining a network you did not configure, run by people who do not have your interests at heart, and you would prefer that joining it twice not produce two correlated entries in someone else's database.

## The threat layers and what addresses each

Eight layers worth thinking about. Proteus owns some of them, partially owns others, and stays out of the way for the rest. Honest accounting matters here.

1. **L1 RF fingerprint.** Every Wi-Fi radio has analog quirks unique to the silicon. Software cannot fix analog. Proteus offers opt-in TX power reduction (`wifi.tx-power-reduce`) to narrow the audience who can hear you cleanly, plus chipset reporting in `proteus status`. The real fix is a swappable USB Wi-Fi adapter. Cross-ref `proteus wiki rf-fingerprinting`.
2. **L2 MAC tracking.** Proteus's primary defense. MAC rotation per join, per schedule, and per probe-driven connectivity loss. DUID rotated alongside, IPv6 IID rotated implicitly under stable-privacy. Cross-ref `proteus wiki mac-recipes`.
3. **L2 Bluetooth tracking.** Proteus generic-aliases the adapter, sets `Discoverable=false` by default, and enables BLE Resolvable Private Address mode where the controller supports it. BR/EDR (classic) BD_ADDR is fixed in v1 — turn the radio off when you do not need it. Cross-ref `proteus wiki bluetooth`.
4. **L3 IP correlation.** Your IP comes from gateway DHCP. Rotating MAC forces a fresh DHCP exchange and (usually) a fresh lease, so your IP rotates with your MAC. For wider IP rotation across networks, add a VPN (Mullvad accepts cash) or Tor. Proteus does not pick your egress.
5. **L3-L4 stack fingerprint.** TCP timestamps off, ICMP info-replies and address-mask replies dropped, NDP hardened. Cross-ref `proteus wiki stack-fingerprint`.
6. **L7 application fingerprint.** Browser fingerprint, JA3/JA4, WebGL, fonts, the entire iceberg. Proteus does not touch this and never will. Use Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling. Cross-ref `proteus wiki threat-model`.
7. **DNS.** Proteus has one knob — strip EDNS Client Subnet on systemd-resolved when no other DNS-privacy tool is present. For encrypted DNS, run `dnscrypt-proxy`, NextDNS, AdGuard Home, or a real DNS-privacy tool. Cross-ref `proteus wiki dns`.
8. **Tracker IDs in app traffic.** Pi-hole, NextDNS, AdGuard Home, uBlock Origin. Out of scope for Proteus.

If your concern is on layers 2, 3, or 5, Proteus is the right primary tool. If your concern is on layers 1, 6, 7, or 8, Proteus is at most a secondary defense and you need the named tool to do the work.

## What the adversary actually has

The threat model only matters if you are honest about who you are defending against. Hostile environments split into rough capability tiers; matching the tier to your defenses prevents both under-protection and security-theater over-protection.

- **Tier 1 — venue analytics.** A retail Wi-Fi controller with a vendor portal (Cisco Meraki, Aruba, the dozen smaller players). Logs MAC addresses, association events, presence dwell times. Sells the data to the venue or to a presence-analytics aggregator. No active probing, no MITM, no per-user targeting. Defense: Proteus's defaults work. Rotate MAC, suppress DHCP options, silence mDNS. Done.
- **Tier 2 — analytics aggregators.** SafeGraph, Skyhook, Veraset, the BLE-beacon networks. Cross-venue presence histories sold to advertisers and to law enforcement (yes, there is a market). Active beacon scanning is common. Defense: Proteus plus Bluetooth off plus saved-network hygiene. The aggregators rely on long-term correlation across venues; Proteus breaks the L2 correlation. The Bluetooth radio is the bigger leak in this tier than the Wi-Fi one.
- **Tier 3 — local active attackers.** A bored attendee at a conference with airodump-ng, a malicious co-tenant on a hotel Wi-Fi, the kid in the cafe with a Pineapple. Active probing, deauth attacks, evil-twin APs. Defense: Proteus plus a VPN to encrypt L3 and up, plus Tor Browser for low-volume sensitive traffic. The L4-and-above threats here matter more than the L2 ones; rotation is a small piece.
- **Tier 4 — state-level passive collection.** Bulk-collection programs, lawful-intercept at carrier or border, transparent proxies on national networks. Visibility on both ends of your TCP flows. Defense: Proteus is necessary but small. Tor with bridges, VPN with multi-hop, account compartmentalization, and operational discipline carry most of the weight here.
- **Tier 5 — targeted state actors.** You are specifically of interest. Active probing tailored to your stack, supply-chain compromise of your hardware, physical surveillance, legal compulsion of intermediaries you trust. Defense: Proteus does not solve this. Tails on a known-clean USB, compartmentalized burner devices, in-person key exchange, physical operational security. Read Surveillance Self-Defense from EFF and the Freedom of the Press Foundation guides.

Most readers of this page are at tier 1 to 3. Tier 4 and 5 are real but rare; if you are there, you already know it. The honest answer for tier 1-3 is that Proteus plus a browser-fingerprint-resistant browser plus a VPN handles the realistic threat model in those tiers fully.

## Pre-trip checklist

Run these before you walk out the door. Every command exits cleanly with a structured reason if it cannot do what you asked, so you find out at home rather than in the cafe.

```sh
# 1. Verify Proteus is healthy.
proteus doctor

# 2. Verify the originals cache is intact (sacred, never re-captured).
proteus original

# 3. Verify the config is what you think it is.
proteus config show

# 4. Set an aggressive cadence for the trip. 30m is a reasonable
#    "I am moving between networks and don't trust any of them" interval.
sudo proteus config set mac.rotation_interval 30m --yes
sudo proteus timer set rotate --interval 30m

# 5. Enable hostname rotation if you want it (off by default; many users
#    keep a stable hostname).
sudo proteus config enable hostname --yes

# 6. Set Bluetooth to the silent profile (alias generic, discoverable off,
#    BLE RPA on where supported). Apply will pick this up.
sudo proteus apply --yes

# 7. Dump the resulting state for audit and put it somewhere you can read
#    on the road if something looks wrong.
proteus status --json > /tmp/proteus-pretrip-$(date -u +%F).json
```

If `proteus doctor` returns non-zero, fix that before you leave. The goal is no surprises in the field.

## Coffee shop / cafe playbook

The casual end of the spectrum. Routine privacy hygiene, not paranoia.

1. Do not connect immediately on arrival. NetworkManager remembers the previous network's MAC. Let Proteus's join-time rotation produce a fresh one, or run `sudo proteus rotate --yes` first if you want to be deliberate about it.
2. Connect, complete the captive portal if any, browse. Periodic rotation is suppressed while you are authed behind the portal — that is the captive-portal loop fix, not a bug. Cross-ref `proteus wiki captive-portals`.
3. Before leaving, rotate manually: `sudo proteus rotate --yes`. The cafe's analytics platform now sees this visit's MAC disappear and cannot stitch it to the next one.
4. Or trust the timer. Come back tomorrow and join again — Proteus will produce a fresh MAC at join, and the visit looks new from the operator's perspective.
5. If you mark the SSID as a known portal (`proteus portal mark "Cafe Free WiFi"`), every visit gets a fresh MAC at join regardless of the schedule. Useful for spots you frequent.

What this gets you: the cafe's footfall analytics see uncorrelated visits. What it does not get you: anonymity from anything you actually log into. If you sign into your real Google account every visit, the network-layer rotation is not the link in that chain.

A note on probe requests. Even when you do not connect, your Wi-Fi card emits probe requests for SSIDs in your saved-networks list. The cafe's controller sees those whether you join or not. Proteus does not silence probe requests in v1 (the kernel's randomized probe-MAC behavior helps, but the SSID list does not). If you want full silence on entry, `nmcli radio wifi off` before you walk in, on after you sit down.

## Conference playbook

Higher density of adversaries and analytics. The peer-snooping risk is real — Wi-Fi is broadcast, anyone with a card in monitor mode and patience can build a presence map.

1. **Pre-conference.** Rotate your MAC, set hostname mode to wordlist or generic (`hostname.mode = "wordlist"` or `"generic"` in config), and set the Bluetooth alias to its generic default. Verify with `proteus current --json`.
2. **Carry a separate USB Wi-Fi adapter** if you want stronger rotation. A different radio rotates the L1 fingerprint as well as the L2 identifier. Cheap cards are fine — you are buying RF identity, not throughput. Cross-ref `proteus wiki rf-fingerprinting`.
3. **During.** `proteus status` shows current state. `proteus current` shows the live identifiers you are emitting. If your conference Wi-Fi is one of the captive-portal-walled-garden kinds (university Wi-Fi often is), set `[captive_portal] policy = "preserve-mac"` so you are not rotating mid-session and re-authing.
4. **Bluetooth.** Sponsor booths sometimes hand out swag in exchange for a Bluetooth scan. Treat this as adversarial — the swag is paid for by the BD_ADDR you donated. Keep `Discoverable=false`, BLE RPA enabled, and consider `rfkill block bluetooth` for the duration of the show floor.
5. **Post-conference.** `sudo proteus revert --yes` clears session state and returns Proteus's view of the originals. Or keep going — there is no requirement to revert.

What conferences do not care about: your hostname rotating mid-session. The vendor-supplied analytics platform is keying on MAC and behavior, not on whether the host says `fedora` or `seven-mauve-coyote.local`. The hostname work matters more for the LAN-side leak (mDNS, NetBIOS) than for the analytics platform.

Two patterns worth knowing. **Conference Wi-Fi often pre-installs a profile via QR code.** Read it before you scan — some profiles install a CA cert that lets the operator MITM your TLS for the duration. Decline anything that wants a root cert installed. **Sponsor talks sometimes "track engagement"** by handing out RFID badges that scan attendance at sessions. That is its own L7 identifier, unrelated to Proteus, but worth knowing it is happening and unrelated to the network-layer protections this tool offers.

## Airport playbook

Heavy passive collection environment. Multiple operators, multiple legal regimes, dwell-time analytics tied to gate-area presence. Treat airports as conferences with worse legal exposure.

1. **Pre-flight.** Rotate everything. `sudo proteus rotate --yes`, `sudo proteus apply --yes`. Set hostname to generic. Disable Bluetooth at the kernel level: `sudo rfkill block bluetooth`. Bluetooth on at an airport is broadcasting an identity to every gate.
2. **At the gate.** Do not connect to the airport Wi-Fi unless you need to. Cellular has its own threats but they are different threats; for short gate dwells the cellular-data path leaks less to local infrastructure. If you must connect, do it briefly and intentionally — captive-portal email is fake, traffic is over a VPN, and you are off the network as soon as you do not need it.
3. **In the lounge.** Lounge Wi-Fi is a different operator from the terminal Wi-Fi and the airline app. Each one is its own correlation event. Treat each network join as a fresh hostile event — let Proteus rotate.
4. **After landing.** `sudo proteus rotate --yes` before you leave the terminal. `sudo proteus revert --yes` after you are home if you want a clean slate.
5. **Customs and border crossings.** A different problem. See the hostile-state-actor section below.

Two specific airport gotchas worth flagging. **Saved-network history.** If you have ever joined the airport's free Wi-Fi at any airport in the chain (Boingo, Aircell, the global airport-Wi-Fi consortia), the host probes for that SSID at every airport. Either delete the saved network or `nmcli radio wifi off` until you actually need it. **Airline apps and gate kiosks.** Boarding-pass scanning, gate-display apps, in-flight Wi-Fi sign-on all want a stable account-level identity. Proteus does not help against any of those — those are application-layer correlation events, the airline already knows who you are from the ticket.

## Hotel playbook

Hotels often charge per-MAC and bind your captive-portal session to your MAC for the duration of the stay. Aggressive rotation here will charge you twice or kick you off the portal repeatedly.

1. **First night.** Let Proteus rotate normally at join. Auth the portal against that MAC. Periodic rotation is suppressed while authed (cross-ref `proteus wiki captive-portals`), so you should not get kicked off mid-session.
2. **Multiple nights.** Pin the MAC for the duration of the stay so you do not get re-charged or re-prompted on every reconnect: `sudo proteus pin --connection "HotelWiFi"`. The connection-scoped pin survives suspends and reconnects. Cross-ref `proteus wiki mac-recipes`.
3. **Last morning.** `sudo proteus unpin --connection "HotelWiFi"` before checkout. Rotation resumes on the next join.
4. **Across stays at the same chain.** Pinning is per-connection-profile; a different hotel of the same chain is a different SSID and connection profile, so your identity is uncorrelated across stays. The chain knows your name from the reservation regardless — Proteus is for the network-side leak, not the booking-side one.

A note on per-MAC charging: this is a practice, not a law. Some chains charge per device per day. Pinning is the operational answer; do not try to defeat the charge by rotating mid-stay, you will just confuse the captive portal and possibly trigger their abuse heuristics.

## Hostile state actor

The serious end of the spectrum. Border crossings into non-permissive jurisdictions, journalism on adversarial networks, dissident communications, anything where the network operator has the resources and the motivation to deanonymize you specifically.

Proteus is necessary but not sufficient here. The whole stack matters and operational discipline matters more.

1. **Stack the layers.** Proteus for L1-L4 and the network-joining protocols. Tor Browser for L7 fingerprint and traffic correlation. Tor or Mullvad for IP-layer correlation (Mullvad accepts cash so the payment trail is broken). dnscrypt-proxy or Tor for DNS resolution policy. uBlock Origin or NextDNS for tracker IDs. Cross-ref `proteus wiki threat-model` for the composition story.
2. **Disable Bluetooth entirely.** `sudo rfkill block bluetooth`. The classic BD_ADDR is fixed and broadcast on inquiry; do not give an adversary the chance to scan it.
3. **Disable Wi-Fi when you are not actively using it.** `nmcli radio wifi off`. Probe requests from a Wi-Fi card looking for known networks leak the SSIDs you have joined before. Off is the only way to silence this completely.
4. **Use a swappable USB Wi-Fi adapter.** A different radio rotates the analog L1 fingerprint. The internal card stays disabled. Cross-ref `proteus wiki rf-fingerprinting`.
5. **Do not log in with real-identity accounts.** No Google, no GitHub, no work email. Application-layer correlation defeats every L1-L4 rotation Proteus can give you. Use compartmentalized accounts you create from inside Tor Browser and never from the same machine you use for anything else.
6. **Bring different hardware if the threat model warrants it.** Tails on a USB stick, a burner laptop, a phone with GrapheneOS. Proteus on your daily driver is one tool in your kit — not the kit.
7. **Border crossings.** A device crossing a border with hostile customs is forensically imaged or compelled-disclosed. No software runs against that threat. Travel with a clean device, restore from backup at the destination, return with a different device.

If your threat model is at this level you should be reading more than just this page. Tails (https://tails.net/), Whonix, the Freedom of the Press Foundation guides, and the EFF's Surveillance Self-Defense site are starting points. Proteus is not a substitute for any of them.

A specific pattern for repressive networks worth calling out: traffic correlation by an adversary who controls the local network and has visibility on the upstream Tor or VPN guard. Proteus rotating your L2 identity does nothing against this. The defense is bridges (obfs4, snowflake, meek) for Tor, or a VPN with provider-side multi-hop and rotating exit IPs. Cross-ref `proteus wiki threat-model` for the boundary discussion. Proteus's job ends at the L3 boundary; everything past that egress is a different tool's responsibility.

## Things Proteus CANNOT do for you in a hostile environment

The honest accounting. None of these are bugs — they are the boundaries of what a network-layer tool can address.

- **Application-layer logins.** Your Google account, your work email, your Twitter handle is the same identity regardless of MAC. Proteus does not anonymize accounts. Account boundaries are application-layer boundaries; Proteus operates underneath that.
- **Browser fingerprint.** Canvas, WebGL, fonts, JA3/JA4 TLS handshake, audio context. Out of scope by design; Tor Browser is the answer. Cross-ref `proteus wiki threat-model`.
- **DNS resolution policy beyond ECS-strip.** Proteus does not run a DoH client, does not block trackers at the DNS layer, does not pick your resolver. Use `dnscrypt-proxy`, NextDNS, AdGuard Home, Pi-hole, or knot-resolver. Cross-ref `proteus wiki dns`.
- **Active TLS man-in-the-middle.** Proteus does not validate certificates for you, cannot tell when a captive-portal MITM is sniffing your TLS, cannot route around it. Tor Browser plus Tor handles this; a real VPN handles half of it.
- **Hardware-level RF identification.** Software cannot fix analog hardware quirks. A swappable USB Wi-Fi adapter is the only real defense. Cross-ref `proteus wiki rf-fingerprinting`.
- **Cellular IMSI/IMEI.** Proteus does not touch the WWAN modem. Airplane mode is the only off switch. If your threat model includes cellular-side identification, the WWAN modem is its own problem.
- **Behavioral analysis.** Typing cadence, scroll patterns, page-dwell times. These are application-layer or even biometric signals. Out of scope.
- **Operational mistakes.** Signing into your real Google account from a hostile network is the kind of mistake no tool can fix. Discipline is the answer.

If you find Proteus claiming to do any of these in its docs or output, that is a bug. File it.

## After you've been in a hostile environment

Coming back is its own checklist. The goal is to clear session state so the next environment is uncorrelated with this one.

1. **Rotate immediately on the way out** if you have not already. `sudo proteus rotate --yes`.
2. **Revert if you want a clean slate.** `sudo proteus revert --yes` drops the session state. The originals cache is preserved (it is sacred and never re-captured), so a subsequent `proteus apply` re-applies the configured policy fresh.
3. **Close every browser tab** from sessions on that network. Cookies and local storage outlive your network identity rotation.
4. **Clear browser history if your threat model warrants it.** Better: use Tor Browser or a containerized browser profile so the history was already ephemeral.
5. **Disable any one-shot exceptions you set for the trip.** If you raised the rotation cadence, dropped it back. If you marked an SSID as a known portal for the trip, unmark it. `proteus config show` to audit.
6. **If you used your real account on the network, accept that account-level tracking saw you there.** No tool fixes that retroactively. Note it for next time.

## Saved networks and probe requests

A specific leak worth its own section because it is invisible by default and matters in every hostile environment.

When your Wi-Fi card is on, the kernel emits probe requests for SSIDs in your saved-networks list to find a network to join. Anyone within range with a card in monitor mode can read those probes. The list of SSIDs you have ever joined is leaked every time you walk into a coffee shop, regardless of whether you join the cafe's network.

Modern kernels randomize the source MAC on probe requests when Wi-Fi is not associated, which helps. The SSIDs themselves are still in cleartext. A saved-networks list of `Home`, `Work`, `Mom's House`, `Boingo_Hotspot`, `Marriott_Bonvoy_Conference_2024` is a high-quality identity profile that survives every MAC rotation.

Three remediations, in increasing intensity:

- **Audit and prune.** `nmcli connection show` lists every saved network. Delete the ones you no longer need. Hidden-SSID networks (`802-11-wireless.hidden = yes`) are the worst offender — the kernel actively probes for them by name even when no AP is broadcasting.
- **Disable Wi-Fi when not in use.** `nmcli radio wifi off` silences probe requests entirely. On for the cafe session, off when you walk out the door. The kernel will not probe what is rfkilled.
- **Compartmentalize.** Use a separate user account or a separate machine for hostile environments, where the saved-networks list is short and disposable. The daily-driver list does not need to leak at the conference.

Proteus does not silence probe requests in v1. The kernel-level randomization handles the source MAC; the SSID-list problem is operational hygiene the user has to manage. A future version may add an SSID-list audit command — see `docs/PLAN.md`.

## Common mistakes

The patterns we see (and have made ourselves). None of these are bugs in Proteus — they are operational errors that make Proteus less effective than it could be.

- **Joining a network before rotating.** NetworkManager remembers the previous network's MAC for a few seconds. If you connect immediately after the previous join, you can leak the old MAC into the new network's logs. Default NM behavior plus Proteus's join-time rotation handle this in most cases, but a manual `sudo proteus rotate --yes` first is cheap insurance.
- **Leaving Bluetooth on without thinking.** Classic BD_ADDR is fixed in v1; an adversary scanning Bluetooth in a venue gets a stable identifier from the host even when Wi-Fi is rotating. The fix is `rfkill block bluetooth` when you do not need it. Cross-ref `proteus wiki bluetooth`.
- **Reusing accounts across compartments.** Same Google account at the cafe and at home means the cafe knows you live wherever home is. Application-layer correlation cannot be undone by L2 rotation. Use separate profiles or separate accounts; treat the boundary as deliberate.
- **Saved networks proliferating.** Every saved network adds an SSID your card probes for at every other network. Audit periodically: `nmcli connection show` and remove anything you do not actively use.
- **Treating revert as a panacea.** `proteus revert` undoes Proteus's network-layer changes. It does not clear cookies, does not log out of accounts, does not reset Bluetooth pairings. Composition matters; revert is one step, not a magic eraser.
- **Trusting a captive portal.** Captive portals MITM your TLS by definition (some only HTTP, some both). Do not log into anything past the portal page. Treat captive portals as adversarial intermediaries until you are off them.
- **Overreacting at low threat tiers.** Running Tor Browser on cafe Wi-Fi for normal browsing hurts your usability more than it helps your threat model. Match the defense to the tier (see above). Save the heavy tools for environments that warrant them.

## Verifying Proteus is doing what it claims in the field

Trust but verify. Every command on this list is read-only and works without root, so you can audit Proteus from a hotel room without escalating.

- `proteus status` — per-feature `applied / skipped (reason) / failed (reason)`. The single most useful command. If a feature reads `skipped` with a reason, that is informational; if it reads `failed`, you have a problem.
- `proteus current --json | jq` — the live identifiers Proteus is emitting right now. MAC per interface, hostname, DUID, Bluetooth alias.
- `proteus original` — the cached permanent MAC and original hostname. Should never have changed since the day you installed Proteus. If it has changed, that is a serious bug — file it.
- `proteus probe` — runs one probe round on demand and prints the per-endpoint outcome plus the classification (`clear`, `down`, `portal-suspected`, `inconclusive`). Useful for sanity-checking the captive-portal classifier on a new network.
- `proteus timer status` — confirms which timers are running and when each fires next.
- A second machine on the same LAN running `avahi-browse -ar` — confirms the host is not announcing services. Run before and after `proteus apply` to see the difference.
- A second machine running `nmap -O <your-ip>` — see what stack fingerprint the kernel still leaks. Compare with the same scan against an unprotected machine.

If any of these show a leak Proteus claims to suppress, that is a bug. File it from the trip — `journalctl -t proteus -n 200 --no-pager` is the log dump worth attaching.

## Defense in depth

A complete personal-privacy stack for hostile environments. Each layer is its own tool because each layer is its own complex world. Compose them, do not expect any one to do the whole job.

- **Network identity** — Proteus. This tool. L1 (TX power), L2 (MAC, Bluetooth alias, BLE RPA), L3 (IPv6, DUID), L3-L4 (TCP timestamps, ICMP, NDP), and the network-joining protocols (DHCP options, hostname, mDNS, LLMNR, NetBIOS, SSDP, WSD, WPAD).
- **Browser fingerprint** — Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling on. Owns the rendering pipeline and TLS ClientHello of the browser process.
- **DNS resolution policy** — `dnscrypt-proxy` for DoH/DoT/DNSCrypt with anonymized relays. NextDNS for cloud-side filtering. AdGuard Home or Pi-hole for self-hosted network-wide filtering. knot-resolver for a local validating recursor. Pick one and let Proteus's ECS-strip defer to it.
- **IP correlation and traffic analysis** — Tor for low-volume privacy-sensitive browsing. Mullvad VPN (cash payment supported) for higher-bandwidth traffic. Both add a middlebox between you and the destination and aggregate your traffic with other users.
- **Account boundaries** — separate browser profiles per identity. Do not reuse accounts across compartments. Firefox Multi-Account Containers, LibreWolf containers, or just separate profile directories.
- **Mobile** — GrapheneOS or LineageOS for Android, both of which sandbox apps more aggressively than stock and let you control identifiers more fully. iPhone with Lockdown Mode is a smaller-surface alternative.
- **Second factor** — a hardware token (Yubikey, SoloKey) so 2FA is not tied to your phone number, which is its own correlation key.
- **Operational** — separate hardware for separate threat levels. A daily driver with Proteus, a Tails USB for the hostile-state-actor end of the spectrum, a clean burner for the worst case.

Three concrete stacks worth naming, in increasing intensity:

- **Daily driver, public Wi-Fi.** Proteus on the Linux laptop with default config. Brave or LibreWolf as the browser. uBlock Origin loaded. NextDNS or `dnscrypt-proxy` as the DNS layer. Mullvad VPN on for sensitive sessions, off for everything else. Hardware token for 2FA. This is the realistic everyday-threat-tier-1-to-3 setup; nothing special about it, nothing exotic, but it covers the routine cases.
- **Higher-stakes work, mixed environments.** Add Tor Browser for sensitive browsing alongside the daily browser. Compartmentalize accounts: one profile for work, one for personal, one for research, none cross-pollinating. USB Wi-Fi adapter you use only for adversarial environments. `rfkill block bluetooth` as the default. This is the journalism-on-controversial-topics, security-research, activist-organizing tier — still tier 3, but with discipline.
- **Hostile-state-actor.** Tails on a known-clean USB stick, separate hardware from your daily driver, Tor with bridges (obfs4 or snowflake), no real-identity accounts, in-person key exchange. Proteus is in this stack only inasmuch as Tails uses MAC randomization by default (Tails has its own implementation; Proteus is not what runs there). The composition story is different at this tier — fewer specialized tools, more single-purpose hardened systems.

No single tool covers everything. Proteus is one layer. The honest answer is composition.

## Cross-refs

- `proteus wiki threat-model` — what each layer addresses, what is and is not in scope, the composition story. The prerequisite for this page.
- `proteus wiki captive-portals` — portal detection, classification, the loop-prevention rules, known-portal SSIDs.
- `proteus wiki rf-fingerprinting` — L1 limits, what TX power reduction does and does not buy you, the USB-adapter-swap answer.
- `proteus wiki dns` — the one ECS-strip knob and its hard guard. Detect-and-defer for `dnscrypt-proxy`, Pi-hole, AdGuard Home, custom resolv.conf.
- `proteus wiki mac-recipes` — MAC pools, pinning per interface or connection, fresh-MAC-per-visit for known portals, OUI realism.
- `proteus wiki bluetooth` — adapter alias, BLE RPA, the BR/EDR limits, paired-device behavior.
- `proteus wiki stack-fingerprint` — TCP timestamps, ICMP info-replies, NDP hardening, the nft table layout.
- `proteus wiki rotation` — the timers, the boot oneshot, event-driven hooks, cooldown rules.
- `proteus wiki probes` — the probe quorum that decides "the network is down", the cooldown logic, the `portal-suspected` exit.
- `proteus wiki discovery` — mDNS, LLMNR, NetBIOS, SSDP, WSD, WPAD silencing and the breakage tradeoffs.
- `proteus wiki troubleshooting` — when something breaks in the field, start here.

External:

- EFF Surveillance Self-Defense — https://ssd.eff.org/
- Freedom of the Press Foundation guides — https://freedom.press/training/
- Tails — https://tails.net/
- Tor Project — https://www.torproject.org/
- Mullvad VPN — https://mullvad.net/
