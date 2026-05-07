If you only read one wiki page, read this one. The rest assumes the mental model below.

## Identifiers

A "network identifier" is anything broadcast or derivable when joining a network. By layer:

**L2.** MAC address — Wi-Fi, Ethernet, Bluetooth `BD_ADDR` (classic), BLE address. The most-fingerprinted thing on your laptop. Six bytes, the upper three are the OUI (manufacturer), the lower three identify the device. Burned-in by the vendor; software-overridable on every modern Linux NIC.

**L3.** IPv6 IID — the lower 64 bits of an IPv6 address, the "interface identifier". Under EUI-64 derivation it leaks the MAC directly; under stable-privacy it derives deterministically from the MAC plus a network-scoped key. Rotate the MAC, rotate the IID. DUID — DHCPv6 client identifier, sticky across reboots by default. ICMPv6 / NDP quirks — hop limits, router solicitation behavior, fingerprintable per-stack.

**L3-L4.** TCP timestamps leak system uptime monotonically (RFC 7323 §7.1). ICMP info-replies (type 15/16) and address-mask replies are an old OS-fingerprinting vector that most kernels still answer.

**Network-joining protocols.** DHCP option 12 (hostname), 60 (vendor class identifier — `dhcpcd-9.4.1` is a banner), 61 (client identifier, often the MAC), 81 (FQDN). mDNS service announcements (`_workstation._tcp`, `_smb._tcp`, etc.). LLMNR, NetBIOS, SSDP, WSD — Microsoft-era discovery protocols still chatty by default. WPAD — proxy auto-discovery, an exfil channel. NTP request signatures — version, mode, poll interval, reference ID.

**Application-layer-but-network-identity.** Hostname comes in three flavors the kernel and systemd track separately: kernel (`/proc/sys/kernel/hostname`), pretty (`/etc/machine-info`, human-readable), and transient (set over DHCP, lives only until reboot). Bluetooth alias is the discoverable name your phone shows when pairing.

Anything off this list is out of scope; see the bottom of this page.

## Rotation

Two triggers, one manual override.

**Scheduled.** Default every 2h via `proteus-rotate.timer`. Tunable in `config.toml`. Set to 0 to disable.

**Probe-driven.** Default every 5m via `proteus-check.timer`. If ≥3 of 4 probes fail, rotate. See `proteus wiki probes` for the quorum logic and why one DNS hiccup doesn't trigger a rotation.

**Per-network-join.** Known captive-portal SSIDs get a fresh MAC every visit. See `proteus wiki captive-portals`.

**What rotates together.** Wi-Fi MAC is primary. DUID is coupled by default — otherwise DHCP would leak the same client identity across MAC rotations. IPv6 IID is coupled too because under stable-privacy it derives from the MAC. Hostname rotation is opt-in via `hostname.rotate-with-mac`.

**Pinning.** `proteus pin <iface|connection>` freezes a MAC. For environments that lock you to one (corporate networks, hotel Wi-Fi after auth, MAC-bound DHCP reservations). `proteus unpin` releases. Pinned interfaces are skipped by both schedule and probe-driven rotation.

**Collision avoidance.** Rotation never picks a MAC matching the current gateway's MAC or anything else in the local ARP table. That would be hilarious and broken.

## Probes

**Quorum.** Contact 4 known endpoints in parallel. Declare "down" only if ≥3 fail. Single-endpoint flakiness shouldn't trigger a rotation.

**Cooldown.** 60s after a rotation before the next probe round. The freshly-rotated stack needs time to come up — DHCP, RA, IPv6 DAD all take real seconds.

**Method.** TCP-connect to a known port, ICMP echo as fallback. No HTTP, no DNS resolution from the probe path itself. Probes run against IPs to avoid letting a broken resolver cause rotations.

**Portal classification.** Probe failures classified as portal-caused never trigger MAC rotation. That's how you avoid the "rotate behind a portal forever" loop. The portal detector runs alongside probes; see below.

Tunable in `[probes]` — endpoints, quorum threshold, cooldown, interval. See `proteus wiki probes`.

## Captive portals

First-class, not a heuristic.

**Classification.** Four states:
- `clear` — Internet works, no interception
- `portal-required` — traffic intercepted, not yet authed
- `portal-authed` — authed, but the portal is still in the path (suppress rotation)
- `unknown` — probe inconclusive

**Detection.** Default target `nmcheck.gnome.org`. Configurable. Same connectivity-check pattern NetworkManager uses.

**Policies.**
- `rotate-before-auth` (default) — fresh MAC, then auth. Whoever runs the portal can't correlate visits.
- `preserve-mac` — for SMS-bound portals where the auth ticket is tied to your MAC. Rotating mid-session locks you out.
- `ask` — interactive. Useful when you don't know yet.

**While authed.** Periodic rotation is suppressed until you leave the network or auth expires.

**Per visit.** Known-portal SSIDs get a fresh MAC every join, regardless of schedule.

**Helper.** `proteus portal open` launches the portal page in your default browser. See `proteus wiki captive-portals`.

## Managed files

Anything Proteus writes to `/etc/` carries a header:

```
# managed by proteus — do not edit
# expected-sha256: <64 hex chars>
```

`proteus diff` (phase G) compares the live file's SHA against the expected one. Drift from manual edits gets flagged loudly so you can decide: re-apply, accept the local change, or back the whole thing out with `proteus revert`.

The original-MAC cache in `/var/lib/proteus/state.json` is sacred. Captured the first time Proteus sees a system, never re-captured. The original hostname is captured the same way at the same time. If you tinker, `proteus reset` clears your config but never touches the cache. `proteus uninstall --purge` is the only thing that removes it. This is so you can always get back to your system's original identity, no matter how badly you've broken the config.

See `proteus wiki internals` for the full state schema.

## The Platform trait

All OS-specific operations live behind a `Platform` trait — netlink calls, dbus calls, file paths, the lot. A future macOS or Windows port (no commitment) would be a backend swap rather than a fork. The CLI, config, and wiki layers stay portable for free.

Today there is only `LinuxPlatform`. Other backends are theoretical.

## Detect-and-defer

Two places where Proteus looks at your existing setup before acting and bows out cleanly if a more specialized tool is already there:

**DNS.** If `dnscrypt-proxy`, Pi-hole, AdGuard Home, a custom `/etc/resolv.conf`, or any non-Proteus drop-in under `/etc/systemd/resolved.conf.d/` exists, the ECS-strip knob refuses to apply, names the detected tool in `proteus status`, and exits clean. Your DNS setup wins. See `proteus wiki dns`.

**NTP.** If `chrony` or `ntpd` is installed, the systemd-timesyncd config normalization is skipped. Same pattern.

The rule: detect first, defer to the more-specialized tool, surface the decision in `proteus status` so you know exactly what was skipped and why.

## Idempotency

`proteus apply` is idempotent. Running it ten times converges to the same state as running it once. Implementations that aren't idempotent are bugs — file them.

`proteus revert` is an invariant: it must work at every commit. If a feature can't be backed out cleanly, the feature isn't shipped. This is the safety net that lets you try Proteus without committing to it.

Together these two rules mean apply / revert / apply / revert is a no-op cycle. Try things.

## No silent failures

Every error names what failed and points at a wiki page or `proteus help <feature>` where applicable. Per-feature status in `proteus status` is one of:

- `applied` — feature is on, working
- `skipped (reason)` — feature is off because of detect-and-defer or unsupported hardware; reason names the cause
- `failed (reason)` — feature should be on but isn't; reason names the cause and suggests a fix

Never a silent skip. If you see Proteus do nothing, that's a bug.

## Out-of-scope identifiers

Identifiers Proteus does not touch, and what to use instead. Full discussion in `proteus wiki threat-model`.

- **TLS ClientHello (JA3/JA4)** — not normalizable from outside the process. Tor Browser, librewolf, Brave's randomization solve this from inside.
- **SSH client fingerprint (HASSH)** — your `ssh_config` is yours. Touching it is a hardening regression.
- **DNS resolution policy beyond ECS-strip** — `dnscrypt-proxy`, NextDNS, AdGuard Home, Pi-hole, knot-resolver. DNS is its own world.
- **Tracker IDs in app traffic** — Pi-hole, NextDNS, uBlock Origin.
- **Traffic correlation** — Tor, Mullvad VPN.
- **L1 RF (analog transmitter characteristics)** — software can't fix this. A swappable USB Wi-Fi adapter is the real answer; Proteus only narrows the capture radius via opt-in TX power reduction. See `proteus wiki rf-fingerprinting`.
- **Bluetooth BR/EDR (classic) BD_ADDR rotation** — chipset-specific HCI, deferred until there's a known-good chipset matrix. BLE address rotation is supported where the controller exposes privacy mode.
- **`/etc/machine-id`** — TPM, journald, dbus all reference it. Real breakage risk; not worth it.

If your threat model needs any of the above, layer the right tool on top of Proteus. Proteus is the network-layer floor, not the whole stack.
