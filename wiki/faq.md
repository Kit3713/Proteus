Frequently asked questions. Short answers. Cross-refs point at the deeper page.

## What does Proteus actually do?

Erases the network identifiers your Linux laptop hands out when joining a network: Wi-Fi/Ethernet/Bluetooth MAC addresses, DHCP options 12/60/61/81, IPv6 IID, hostname (kernel/pretty/transient), DUID, mDNS announcements, and a few more. Cross-ref `proteus wiki intro`.

## Why not Tails or Whonix?

Those are whole-system privacy distros. Proteus runs on your normal Fedora install and only touches network-layer identity. Different problem, different scope. Cross-ref `proteus wiki concepts` and `docs/PRIOR-ART.md`.

## Why not just `macchanger`?

macchanger only does MAC, no scheduling, no captive portal handling, no DHCP/IPv6/hostname coordination. A new MAC with the same DHCP fingerprint, IPv6 IID, and `localhost.localdomain` hostname is still you. Proteus does the whole network-joining identity surface. Cross-ref `docs/PRIOR-ART.md`.

## Does Proteus protect me from Google tracking me?

No. That is an application-layer or browser problem. Use Tor Browser, librewolf, or Brave for browser fingerprinting. Use Pi-hole or uBlock for tracker blocking. Cross-ref `proteus wiki threat-model`.

## Will this break my home Wi-Fi?

Probably not. Some routers do MAC-based DHCP reservations or parental controls keyed on MAC; if you rely on those, pin the MAC for that connection with `proteus pin <ssid>`. Pinning is per-connection, so other networks still rotate. Cross-ref `proteus wiki mac-recipes`.

## Will this break my corporate Wi-Fi?

Possibly. 802.1X anonymous outer identity is opt-in for a reason — some corporate auth servers reject mismatched outer/inner identities. Don't enable `enterprise_wifi.anonymous_outer_identity` without testing. Cross-ref `proteus wiki enterprise-wifi`.

## Why isn't there a daemon?

Two systemd timers (`proteus-rotate.timer` 2h, `proteus-check.timer` 5m) and a boot oneshot are all that's needed; the CLI is the whole product. Less code, less surface area, fewer moving parts, no long-lived process holding root. Cross-ref `proteus wiki rotation`.

## Can I use this with my VPN?

Yes. Proteus operates below the VPN layer. The VPN sees a normal-looking Linux system; Proteus changes what shows up to the local LAN, the upstream DHCP server, and any passive observer between you and the VPN entry node before traffic hits the tunnel.

## Can I use this with Mullvad, NextDNS, dnscrypt-proxy, or Pi-hole?

Yes. Proteus has a hard guard: if it detects another DNS-privacy tool managing systemd-resolved, the ECS-strip knob bows out. Your DNS setup wins. Cross-ref `proteus wiki dns`.

## What's the binary size?

Phase A is around 1.3 MB stripped. Project invariant: ≤3 MB. Any dependency that adds more than 200 KB needs a justification. Cross-ref `CONTRIBUTING.md` quality bar.

## Does this work on Ubuntu, Debian, or Arch?

Probably; we test against Fedora 43+. Other systemd+NetworkManager distros should work but are secondary targets. Open an issue with your specifics if something breaks.

## Does it work without NetworkManager?

The current architecture is NM-aware (managed via DBus, no `nmcli` shelling). On systems running plain `dhclient` and `wpa_supplicant`, Proteus's NM-specific features skip cleanly with a `skipped (no NetworkManager)` line in `proteus status`.

## Will rotating MAC kick me off my current connection?

Yes, briefly. NetworkManager will reconnect with the new MAC. Expect a few seconds of dropped traffic. The boot oneshot tries to do the first rotation before any user traffic.

## How often should I rotate?

Default 2h. Shorter (30m, 1h) means more privacy and more DHCP renewals — some networks throttle or rate-limit this. Longer (4h, 8h) means less network noise but a bigger correlation window. The default trades the two; change it with `rotation.interval` in `/etc/proteus/config.toml`. Cross-ref `proteus wiki rotation`.

## What happens if I lose connectivity mid-rotation?

The probe-quorum logic (default at least 3 of 4 fail → rotate) detects this and triggers a fresh rotation. There's a 60s cooldown to avoid loops. Cross-ref `proteus wiki probes`.

## What happens behind a captive portal?

First-class handling. Default policy `rotate-before-auth`: get a fresh MAC, then auth. Periodic rotation suppressed while authed. No loops behind portals. Cross-ref `proteus wiki captive-portals`.

## Is `proteus revert` safe?

Yes — it's a hard project invariant. Restores cached originals (MAC, hostname), removes our drop-ins, removes our nft rules, restores NM per-connection settings. Cross-ref `proteus wiki concepts`.

## Where does Proteus store state?

- Config: `/etc/proteus/config.toml`
- State (cached originals plus managed state): `/var/lib/proteus/state.json`
- Drop-ins: `/etc/sysctl.d/`, `/etc/systemd/resolved.conf.d/`, `/etc/systemd/timesyncd.conf.d/`
- nft table: `inet proteus`

Cross-ref `proteus wiki internals`.

## How do I uninstall?

`sudo proteus uninstall --purge --yes`. This runs `revert` first, then removes the binary and (with `--purge`) clears `/etc/proteus` and `/var/lib/proteus`. Cross-ref `proteus wiki uninstall`.

## Does Proteus phone home or collect telemetry?

No. Project invariant: "No network egress beyond the configured probe targets. Ever. No telemetry, no update checks." See `docs/PLAN.md` "Invariants I'm holding myself to".

## What's the license?

GPL-3.0-or-later. Distribute a modified version, you must release the source under GPLv3+. See `LICENSE`.

## How do I contribute?

Read `CONTRIBUTING.md`. Phase-aware: most useful contributions vary by phase. Pre-Phase A: feedback on the plan. Phase A onwards: code, wiki, integration tests.

## Will this break Fedora's hardening?

No. Project invariant: Proteus does not touch `crypto-policies`, `/etc/ssh/ssh_config`, or `/etc/machine-id`. Anything that could weaken hardening or break a working setup is opt-in, default off, and the wiki page lists the concrete failure modes. Cross-ref `proteus wiki threat-model`.

## Does it rotate `/etc/machine-id`?

No, deliberately. systemd-journal, dbus, and TPM-bound state all reference machine-id; rotating it is a real breakage risk for not much network-layer gain. Out of scope. Cross-ref `proteus wiki threat-model`.

## Will Proteus stop my ISP from seeing what I do?

No. Your ISP sees IPs and SNI regardless of MAC. Use a VPN or Tor for that. Proteus changes what your laptop looks like to the local network and the DHCP server, not what the WAN sees.

## Can I script Proteus from my own tooling?

Yes. Read commands take `--json`, mutating commands take `--yes`, exit codes are stable and documented. The CLI is meant to be wrappable — a future GUI is supposed to wrap it, not scrape a TUI. Cross-ref `proteus wiki cli`.

## What if I tinker and break my config?

`sudo proteus reset` clears your config back to defaults and re-applies. Deliberately does not touch the cached original-MACs or rotation history — that's `revert` plus `uninstall --purge` territory. Cross-ref `proteus wiki troubleshooting`.

## How do I check Proteus is actually doing what it claims?

Use the recipes in `proteus wiki verifying` — `tcpdump` for DHCP option leakage, `avahi-browse` for mDNS, `nmap` for ICMP/TCP fingerprint. Don't trust the tool, verify it. Cross-ref `proteus wiki troubleshooting`.

## Why named Proteus?

After the shapeshifter. Read `README.md`.

## Cross-refs

- `proteus wiki intro`, `proteus wiki concepts`, `proteus wiki threat-model`
- `proteus wiki troubleshooting` for "this broke" questions
- `proteus wiki cli` for "how do I X" questions
