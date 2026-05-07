When you join a network, your machine starts talking before you do. mDNS announces your hostname, LLMNR asks the LAN to resolve names, NetBIOS broadcasts on UDP 137, SSDP shouts UPnP capabilities at 1900, WSD hawks itself on 3702. Every one of those packets carries identifying information that long outlives the session. This page covers the protocols Proteus silences, the protocols it leaves alone, and the breakage tradeoffs you sign up for when you flip the opt-in knobs.

> **Status (audit 2026-05):** the `[discovery]` config section exists in `src/config.rs` today (`mdns_silence`, `llmnr_silence`, `ssdp_block`, `wsd_block` — all default `false`), but the writers that actually apply these settings are split across multiple pending PRs:
> - mDNS responder + LLMNR drop-in for systemd-resolved: planned, phase E
> - NetBIOS / SSDP / WSD nftables rules: **pending PR #70** (nftables rule manager, DIRTY)
> - sysctl drop-in for stack hardening (referenced from this page): **pending PR #69** (DIRTY)
> - mDNS responder silencing via systemd-resolved drop-in is **planned with no PR yet**
> - LLMNR / NetBIOS silencing is **planned with no PR yet**
>
> Today, `proteus apply` reports discovery and stack as `not yet implemented`. The configuration knobs documented further down (`mdns_responder`, `mdns_resolve`, `llmnr`, `netbios`, `wpad`, `ntp_normalize`) are the planned schema; the schema that ships today only has `mdns_silence`, `llmnr_silence`, `ssdp_block`, `wsd_block` (all booleans, default off).

## The discovery surface

Joining a Wi-Fi or Ethernet network typically triggers, in the first few seconds:

- **Hostname leakage.** mDNS responder publishes `<host>.local` on multicast. NetBIOS broadcasts the same name on UDP 137. Whoever is sniffing the LAN now has your machine's name.
- **Service capability leakage.** SSDP advertises UPnP services. WSD advertises Web Services for Devices. Both enumerate your machine's capabilities to anyone listening.
- **Bonjour / AirPrint preferences.** mDNS service announcements (`_workstation._tcp`, `_smb._tcp`, `_airplay._tcp`) say what kinds of services you're interested in or expose.
- **Domain membership questions.** LLMNR asks the LAN to resolve names a Windows-style hostname lookup would. The questions themselves leak what your machine is looking for.

Proteus silences each of these where it can do so without breaking workflows you rely on. SSDP and WSD blocking are opt-in because they break real things — KDE Connect and WS-Discovery-only printers respectively.

## mDNS (Multicast DNS)

mDNS is two roles in one protocol: a **responder** that answers queries for `<host>.local` on the LAN, and a **resolver** that asks the LAN for `*.local` names so you can print to `printer.local` or stream to `livingroom.local`.

- **Responder** — `avahi-daemon` answers `<host>.local` queries on UDP 5353 multicast. Proteus disables this via a systemd-resolved drop-in: `MulticastDNS=resolve` (resolve only, do not respond). If `avahi-daemon` is installed and active, Proteus does not stop it directly — it disables responder behavior at the systemd-resolved layer and surfaces a note in `proteus status` if avahi is also running.
- **Resolver** — kept enabled by default. You probably want to print to AirPrint or talk to a Sonos. Configurable: set `[discovery] mdns_resolve = false` in `config.toml` to disable resolution as well.

The drop-in lives at `/etc/systemd/resolved.conf.d/10-proteus-discovery.conf` and carries the standard managed-file header (see `proteus wiki concepts`). Effect: nothing on the LAN can enumerate your machine via `<host>.local` lookups, but you can still resolve `printer.local` to print.

## LLMNR (Link-Local Multicast Name Resolution)

RFC 4795. Microsoft's mDNS-equivalent. Used in Windows networks to resolve hostnames without WINS. Linux's systemd-resolved supports it and answers by default.

- Disable via systemd-resolved: `LLMNR=no`.
- Drop-in lives at the same path as the mDNS change: `/etc/systemd/resolved.conf.d/10-proteus-discovery.conf`. One file, both keys.

LLMNR has no analogue to the mDNS resolver-vs-responder split: when it's off, it's off. There is no workflow on a typical Linux laptop that relies on answering LLMNR queries. Default off in Proteus.

## NetBIOS (UDP 137-139)

Pre-mDNS Microsoft name resolution. samba's `nmbd` provides it on Linux when installed.

- If `nmbd` is installed: `systemctl disable --now nmbd`. Otherwise no-op.
- Belt-and-suspenders firewall: nft rules drop UDP 137/138 and TCP 139 inbound and outbound on managed interfaces. Cross-ref `proteus wiki stack-fingerprint` for how Proteus manages its nft table.

If you actually need NetBIOS — joining a legacy Windows workgroup — set `[discovery] netbios = true` to skip the disable and the firewall rules. Default off.

## SSDP (Simple Service Discovery Protocol, UDP 1900)

UPnP discovery. KDE Connect uses this. Some printers, smart TVs, and media servers advertise via SSDP.

- **Default off** (configurable: `[discovery] ssdp_block = false`). Opt-in because blocking SSDP **breaks KDE Connect**. If you use KDE Connect to mirror your phone's notifications, share files between phone and laptop, or use your phone as a presenter remote — leave this alone.
- When enabled: nft rule drops UDP 1900 inbound and outbound on managed interfaces.
- KDE Connect users have two paths: leave SSDP unblocked (default), or use a different cross-device mechanism (Syncthing for files, web-based notifications, etc.) and block SSDP.

SSDP blocking is the canonical opt-in: silent by default, defer the decision to the user. See `proteus wiki troubleshooting` for the recovery path if you enable this and then realize you needed KDE Connect after all.

## WSD (Web Services for Devices, UDP 3702 + TCP 5357)

Newer Windows-style discovery. Some printers — notably HP, Brother, and a few network-attached scanners — advertise via WSD only. macOS and Linux print via mDNS / IPP; WSD is a Windows-first world.

- **Default off** (configurable: `[discovery] wsd_block = false`). Opt-in because blocking WSD **breaks WSD-only printers**.
- When enabled: nft rules drop UDP 3702 and TCP 5357 inbound and outbound on managed interfaces.

If your printer shows up in CUPS via mDNS or you set it up by IP, WSD blocking is safe for you. If your printer was discovered through Windows' "Add a printer" wizard and copied over to Linux without changing protocol, you may be on WSD — check `lpstat -v` and look for `wsd://` URIs. See `proteus wiki troubleshooting` for the recovery path.

## WPAD (Web Proxy Auto-Discovery)

DNS- and DHCP-based proxy discovery. A famous CVE source: a malicious DHCP server on the network you just joined hands you a `wpad.dat` URL, your browser fetches it, and now your traffic routes through their proxy.

- Disable per-connection via NetworkManager: `connection.wpad = no` set on every managed connection profile.
- Default ON in Proteus. There is no workflow on a typical laptop that requires WPAD; corporate laptops on managed networks generally configure proxy explicitly via PAC URL or system settings.

## NTP (Network Time Protocol)

NTP itself is not a discovery protocol, but the client request format — version, mode, poll interval, reference ID — fingerprints the OS and stack, and vendor distros often ship with branded NTP pools that identify you down to the distro and major version.

- Drop-in at `/etc/systemd/timesyncd.conf.d/10-proteus.conf`.
- Sets a generic NTP pool (`pool.ntp.org`).
- Removes any vendor-specific server lists (e.g., Fedora's `0.fedora.pool.ntp.org`).
- **Detect-and-defer**: if `chronyd` or `ntpd` is installed and active, Proteus skips the timesyncd config entirely and surfaces the skip in `proteus status` with the detected daemon named. Same pattern as DNS — see `proteus wiki concepts` for the rule.

If you actively manage your time daemon, Proteus stays out of your way. If you don't, Proteus normalizes the request signature so it doesn't ship a Fedora-branded pool to every NTP server it talks to.

## Configuration

```toml
[discovery]
mdns_responder = false      # disable mDNS announcements (default true → off)
mdns_resolve = true         # keep mDNS resolution working (printers etc.)
llmnr = false               # disable LLMNR (default true → off)
netbios = false             # disable NetBIOS (default true → off)
ssdp_block = false          # opt-in; breaks KDE Connect
wsd_block = false           # opt-in; breaks WSD printers
wpad = false                # disable WPAD (default true → off)
ntp_normalize = true        # use generic pool (skipped if chrony/ntpd)
```

The keys read as "should the protocol stay on" for the protocols Proteus disables by default, and "should the block apply" for the protocols Proteus leaves alone by default. The asymmetry is honest about the tradeoff: defaults reflect "no breakage in the common case", not "maximally silent at any cost".

## Verification

A handful of commands to confirm Proteus actually silenced what it claims to.

- `avahi-browse -arp` — should show no entry for your machine after the responder is off. Run from another machine on the LAN for a true external view.
- `tcpdump -i wlan0 -nn 'multicast and (port 5353 or port 5355 or port 1900 or port 3702)'` — watch multicast discovery traffic. After `proteus apply` you should see no outbound packets from your machine on these ports beyond mDNS resolver queries you initiated.
- `nmap --script broadcast-ms-sql-discover` — Microsoft-flavored discovery sweep. Should not enumerate your machine.
- `nmap -sU -p 137,138,139,1900,3702,5353,5355 <your-ip>` — port scan from another machine on the LAN. After Proteus, the responder ports should be closed or filtered.
- `resolvectl status` — confirms `MulticastDNS=resolve` and `LLMNR=no` on the global section.
- `proteus status` — names every discovery feature as `applied / skipped (reason) / failed (reason)`. The truth, per feature.

## Reverting

`sudo proteus revert` undoes everything this page describes:

- Removes `/etc/systemd/resolved.conf.d/10-proteus-discovery.conf`.
- Removes `/etc/systemd/timesyncd.conf.d/10-proteus.conf`.
- Removes the nft rules for NetBIOS / SSDP / WSD blocks.
- Restores per-connection `wpad` settings on NetworkManager profiles.
- Re-enables `nmbd` only if Proteus is the one that disabled it; otherwise leaves it as found.

`proteus revert` is an invariant. If a feature on this page can't be backed out cleanly, that's a bug — file it.

## Cross-refs

- `proteus wiki dns` — for the DNS resolver detect-and-defer pattern this page mirrors for NTP.
- `proteus wiki concepts` — for the detect-and-defer rule and the managed-file header format.
- `proteus wiki stack-fingerprint` — for how Proteus manages its nft table and which firewall rules live where.
- `proteus wiki troubleshooting` — for KDE Connect recovery if you enabled `ssdp_block` and regretted it, and for WSD-only printer recovery.
