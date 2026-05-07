Proteus is a Rust CLI that erases the network-layer identifiers your Linux laptop hands out every time it joins a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name. Single binary, embedded wiki, runs on Fedora 43+ with systemd and NetworkManager.

This page is the first five minutes. For installation and recipes, see `proteus wiki quickstart`. For the mental model, see `proteus wiki concepts`.

## First five minutes

Three read-only commands work without root and tell you everything before you change anything:

- `proteus status` — what is currently applied on this host, what was skipped, and what failed. Per-feature `applied / skipped (reason) / failed (reason)`.
- `proteus current` — the live identifiers your machine is handing out right now: MAC per interface, hostname, DUID, Bluetooth alias.
- `proteus original` — the cached permanent MAC and original hostname Proteus snapshotted on first run. Sacred, never re-captured.

When you are ready to apply, `sudo proteus apply` is idempotent: running it ten times converges to the same state as running it once. `sudo proteus revert` (planned, phase G) puts everything back. `sudo proteus rotate` forces a fresh MAC immediately on the active interfaces.

## What gets erased

Audit-aware list — `(today)` means shipped on current main; `(planned, phase X)` or `(pending PR #N)` flags features described accurately for the eventual implementation.

**L2** — Wi-Fi MAC, Ethernet MAC (today), Bluetooth adapter alias and discoverability (today), BLE Resolvable Private Address mode where the controller supports it (today).

**L3** — IPv6 stable-privacy and temp addresses, DUID rotated alongside MAC, ICMPv6/NDP fingerprint hardening (planned, phase D/E — no PR yet for IPv6; sysctl drop-in is pending PR #69).

**L3-L4** — TCP timestamps off, ICMP info-replies dropped (planned, pending PR #69 sysctl + PR #70 nft); optional gratuitous-ARP suppression (planned).

**DHCP** — options 12, 60, 61, 81 suppressed (planned, pending PR #73).

**Discovery** — mDNS responder and resolver, LLMNR, and NetBIOS silenced (planned, no PR yet for mDNS/LLMNR/NetBIOS via systemd-resolved drop-ins). SSDP and WSD blocked behind opt-in flags (planned, pending PR #70's nft writer).

**Hostname** — kernel, pretty, and transient names rotatable from a router-flavored wordlist (today), with a generic-default option and an optional rotate-with-MAC (today; user-configurable `generic_value` is planned).

**Captive portals** — first-class detection, fresh MAC per visit to known portals, no rotation loops while authed (pending PR #66).

**DNS** — one narrow knob: strip EDNS Client Subnet on systemd-resolved (pending PR #71). Defers to dnscrypt-proxy, Pi-hole, AdGuard Home, or a custom `/etc/resolv.conf` when present.

**RF** — opt-in TX power reduction so the capture radius for passive listeners is smaller (planned, no PR yet). Chipset reported in `proteus status` (planned).

## What Proteus is not

This is a network-layer fingerprint eraser. It is not a privacy suite. It will not pretend to solve problems that belong to other tools.

- Not a TLS or browser fingerprint tool. Use Tor Browser, librewolf, or Brave's randomization.
- Not a DNS-privacy tool beyond the one ECS-strip knob. Use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole.
- Not a tracker blocker. Use Pi-hole, NextDNS, or uBlock Origin.
- Not a traffic correlation defense. Use Tor or Mullvad VPN.
- Not a hardening framework. Proteus refuses to weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`, or rotate `/etc/machine-id`.

The wiki page `threat-model` (planned for phase F) spells this out so you don't over-trust the tool.

## Who it's for

Technical Linux users on Fedora 43+ who want a single tool that handles network-joining identity scrubbing without touching their TLS, browser, or DNS stack. NetworkManager and systemd-resolved are required. BlueZ is required for the Bluetooth features. firewalld or nftables is required for the discovery blocks.

Compose Proteus with the tools listed above. Each layer is its own complex world and deserves its own tooling.

## When MAC rotation happens

By default, on a 2h schedule via `proteus-rotate.timer`, and on probe-driven connectivity loss via `proteus-check.timer` at 5m. Probe quorum is at least three of four targets failing, with a 60s cooldown so a flaky link cannot loop you.

Captive portals are the exception. Probe failures classified as portal-caused never trigger MAC rotation — that is how the loop is avoided. Periodic rotation is suppressed while you are authed behind a portal. Known-portal SSIDs get a fresh MAC per visit instead.

Pinning a MAC per interface or per NetworkManager connection is supported (today, phase B); see `proteus wiki concepts` and the `mac-recipes` page.

## How it behaves

There is no daemon. The CLI is the whole product. Two systemd timers (`proteus-rotate.timer` at 2h, `proteus-check.timer` at 5m) and a boot oneshot call back into the binary. Everything else is on-demand.

State lives in `/var/lib/proteus/state.json`. Config lives in `/etc/proteus/config.toml`. The first time Proteus sees a system, it caches the permanent MAC and the original hostname before doing anything; those are sacred and never re-captured.

Anything Proteus writes under `/etc/` carries a "managed by proteus" header plus a SHA of expected content, so `proteus diff` (planned, phase G) can spot manual edits.

All mutating commands need root and exit with a friendly error pointing at sudo when run unprivileged. Read commands work for any user and degrade quietly when the relevant files are not readable.

`proteus revert` works at every release (planned cross-cutting umbrella, phase G — per-component revert paths ship today for bluetooth and hostname). Backing out is a real option from day one.

Logging goes to journald via `tracing-journald`, with a stderr fallback when not under systemd. There is no telemetry, no update check, and no network egress beyond the configured probe targets. Ever.

## Where to go next

- `proteus wiki quickstart` — install, first run, basic recipes.
- `proteus wiki concepts` — mental model: identifiers, rotation, captive portals, managed files, revert.
- `proteus wiki threat-model` — what Proteus does not do, and which tool to reach for instead. Planned for phase F.
- `proteus help` — full CLI reference.
- `proteus status` — what is currently applied on this host, and what was skipped or failed and why.
