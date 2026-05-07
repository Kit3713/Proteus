# Proteus

A Rust CLI for Linux that erases the network identifiers your laptop hands out every time it joins a network. MAC addresses, DHCP options, IPv6 derivations, hostname, mDNS chatter, TCP fingerprint quirks, Bluetooth name, captive-portal correlation. It rotates MACs on a schedule (default 2h) and on probe-driven connectivity loss (default every 5m). Single binary, embedded wiki, runs on Fedora 43+ with systemd + NetworkManager.

Named after the shapeshifter.

## What it isn't

A privacy suite. It doesn't do TLS fingerprinting (use Tor Browser), DNS resolution policy beyond stripping EDNS Client Subnet (use dnscrypt-proxy or Pi-hole), tracker blocking (Pi-hole), traffic correlation defense (Tor, Mullvad), or browser fingerprint randomization (Tor Browser, librewolf). The wiki page `threat-model` spells this out so people don't over-trust it.

It also won't break Fedora's security hardening. No writes to `/etc/ssl/openssl.cnf` (would fight `update-crypto-policies`), no SSH config tweaking, no `/etc/machine-id` rotation. Anything that could weaken hardening or break a working setup is opt-in, default off, and the wiki page lists the concrete failure modes.

## What gets erased

L2: Wi-Fi MAC, Ethernet MAC, Bluetooth adapter name and discoverability, BLE address where the controller supports privacy mode. 802.1X anonymous outer identity for enterprise Wi-Fi, opt-in.

L3: IPv6 stable-privacy and temp addresses, DUID rotated alongside MAC, ICMPv6/NDP quirks.

L3-L4: TCP timestamps off, ICMP info-replies dropped, optional gratuitous-ARP suppression.

L7-but-network-identity: DHCP options 12/60/61/81 suppressed; hostname (kernel, pretty, transient) rotatable with a router-flavored wordlist or pinned to a generic; mDNS responder and resolver disabled; LLMNR, NetBIOS, SSDP, WSD blocked; WPAD off; NTP client config normalized; EDNS Client Subnet stripped only when systemd-resolved is the active resolver and no other DNS-privacy tool is detected.

Captive portals are first-class. Don't loop-rotate behind one. Fresh MAC per visit to known-portal SSIDs. Suppress periodic rotation while authed.

L1 RF: nothing software can do about your Wi-Fi card's analog characteristics. Proteus offers an opt-in TX power reduction so the capture radius for passive listeners is smaller, and surfaces your chipset in `proteus status` so you know what your hardware exposes. The wiki page `rf-fingerprinting` is honest about the limits.

## How it works

There's no daemon. The CLI is the whole product. Two systemd timers (`proteus-rotate.timer` 2h, `proteus-check.timer` 5m) and a boot oneshot call back into the binary. Everything else is on-demand.

State lives in `/var/lib/proteus/state.json`, config in `/etc/proteus/config.toml`. The first time Proteus sees a system, it caches the permanent MAC and the original hostname before doing anything; those are sacred and never re-captured. Anything Proteus writes to `/etc/` carries a "managed by proteus" header plus a SHA of expected content, so `proteus diff` can spot manual edits.

Networking goes through NetworkManager via zbus (no `nmcli` shelling) and rtnetlink for direct interface ops. Bluetooth via BlueZ over zbus. Anything OS-specific lives behind a `Platform` trait so a future macOS or Windows port — no commitment — would be a backend swap rather than a fork.

All mutating commands need root; non-root invocations exit with a friendly error pointing at sudo. Read commands work for any user when the relevant files are readable, and degrade quietly when not. Logging goes to journald via tracing-journald with a stderr fallback when not under systemd.

The CLI is designed to be wrappable. Read commands take `--json`, destructive ones take `--yes`, exit codes are stable and documented. A friend may build a GUI later; I want them to wrap a clean surface, not scrape a TUI.

## Phases

Seven phases, roughly in order. Stop and look at each before starting the next. Nothing ships without `proteus revert` working — backing out has to be a real option from day one.

**A. Skeleton.** Cargo project tuned for size: `opt-level="z"`, lto, codegen-units=1, panic=abort, strip. Full clap surface, every subcommand parses; the ones that aren't built yet return "not implemented in this phase" pointing at where they will land. Read-only `proteus status`, `current`, `original`, `show-config`, `show-defaults` work via netlink + sysctl reads only, no DBus yet. Wiki ships with three real pages: `intro`, `quickstart`, `concepts`. Logging wired up. Binary under 3 MB stripped, cold release build under 60s on the dev host.

**B. L2 identity.** Wi-Fi/Ethernet MAC rotation, exact set, OUI-pool randomization (Apple, Intel, Samsung, Dell, plus locally-administered random), pin/unpin per interface or per NM connection. Avoid assigning a MAC that matches the gateway or anything else in the current ARP table. Plus the easy Bluetooth bits via BlueZ: generic adapter alias, discoverable=off by default, BLE Resolvable Private Address mode where the controller supports it. BR/EDR (classic) BD_ADDR rotation is chipset-specific HCI-command territory and stays deferred — too easy to brick across vendors. Wiki: `mac-recipes`, `bluetooth`. Tested on at least one each of iwlwifi/rtw89 Wi-Fi and Intel/non-Intel Bluetooth.

**C. Probes, timers, captive portals.** Probe quorum (default ≥3 of 4 fail → rotate), 60s cooldown, TCP-connect with ICMP fallback. Two systemd timers and the boot oneshot. Captive portal handling is first-class, not a heuristic: dedicated detector against `nmcheck.gnome.org` or equivalent, classification (`clear` / `portal-required` / `portal-authed` / `unknown`), policy choice (`rotate-before-auth` is the default, `preserve-mac` for SMS-bound portals, `ask` for interactive), suppress periodic rotation while authed, fresh MAC per visit to known-portal SSIDs, browser-helper to launch the portal page. Probe failures classified as portal-caused never trigger MAC rotation — that's how you avoid the loop. Wiki: `probes`, `rotation`, `captive-portals`.

**D. DHCP, IPv6, hostname, 802.1X, the one DNS knob.** NM settings for DHCP option 12/60/61/81 suppression. IPv6 stable-privacy plus DUID rotation alongside MAC. Hostname (kernel/pretty/transient) over hostname1 dbus, with a wordlist of about 500 router-flavored words, a generic-default option (`fedora`), and an optional rotate-with-MAC. 802.1X anonymous outer identity for enterprise Wi-Fi, opt-in and default off because some corporate auth servers reject mismatched outer identities. The single DNS knob: `dns.strip-edns-client-subnet`, on by default but with a hard guard. If Proteus sees dnscrypt-proxy, Pi-hole, AdGuard Home, a custom `/etc/resolv.conf`, or any non-Proteus drop-in under `/etc/systemd/resolved.conf.d/`, it refuses to apply, names the detected tool in `proteus status`, and exits clean. The user's DNS setup wins, every time. Wiki: `dhcp`, `ipv6`, `hostname-recipes`, `enterprise-wifi`, `dns`.

**E. Discovery silencing, stack fingerprint, RF surface.** systemd-resolved drop-in disabling mDNS responder and resolver and LLMNR. firewalld (or nftables-direct fallback) blocking SSDP, WSD, NetBIOS — SSDP and WSD off by default because they break KDE Connect and WS-Discovery printers. Sysctl drop-in for `tcp_timestamps=0` (with documented PAWS edge case for high-bandwidth long-lived flows), ICMP info-reply drops via nft, optional ARP gratuitous suppression. ICMPv6/NDP fingerprint hardening. WPAD off via NM. NTP client config normalized via systemd-timesyncd drop-in, skipped if chrony or ntpd is installed (same detect-and-defer pattern as DNS). Opt-in `wifi.tx-power-reduce` and chipset reporting in status. Wiki: `discovery`, `stack-fingerprint`, `rf-fingerprinting`. No SSH, no TLS, no OpenSSL touching — those are application-protocol fingerprints and out of scope.

**F. Cross-cutting wiki, search, packaging.** Per-feature wiki pages already shipped per phase. This phase fills the connective tissue: `threat-model` (the most important page — what we don't do and which tool to reach for instead), `cli` (full reference plus exit codes plus JSON schemas), `config` (every flag with default and risks), `troubleshooting`, `verifying` (tcpdump, avahi-browse, nmap recipes to confirm we're doing what we claim), `uninstall`, `internals` (state.json schema, JSON output schemas — the page a future GUI author will read), `faq`, `glossary`. Full-text wiki search via a build-time inverted index, target under 200ms cold. Audit pass on every error to make sure each one points at a wiki page or `proteus help <feature>` where applicable. `install.sh` is POSIX shell, no bashisms; copies the binary, writes `semanage fcontext` rules, runs `proteus apply`. `uninstall.sh` is a thin wrapper around `proteus uninstall --purge --yes` so distro packages can reuse the same code path.

**G. Diff, dry-run, reset, uninstall, integration tests.** `proteus diff` shows config vs defaults vs live state and flags drift from our managed files via the SHA in their headers. `proteus dry-run <command>` previews mutations — every mutator goes through a `Plan` enum that can be either previewed or executed. `proteus reset` clears your config back to defaults and re-applies; this is the "I tinkered and broke it" hatch and it deliberately does not touch the cached original-MACs or history. `proteus uninstall [--purge]` is the full-removal hatch — runs revert, removes the binary, optionally clears `/etc/proteus` and `/var/lib/proteus`. Integration tests in a privileged Podman + systemd container with stubbed NM and BlueZ. Image-diff verification that a clean install + uninstall returns the system to baseline. CI on a Fedora-latest container: build with size check ≤3 MB, clippy with `-D warnings`, fmt, unit tests. Tag v1.0.0 with the stripped binary attached and a SHA256.

Rough sizing on clean dev days, no surprises: A 1, B 2.5, C 2, D 1.5, E 1.5, F 1, G 2. About 11.5 days.

## Invariants I'm holding myself to

No network egress beyond the configured probe targets. Ever. No telemetry, no update checks.

`proteus revert` works at every commit. If I can't back out cleanly, I haven't shipped the feature.

`proteus apply` is idempotent. Running it ten times converges to the same state as running it once.

No silent failures. If I can't do what was asked, I say so loudly and point at a wiki page or `proteus help`.

Robustness over breadth. A feature that's flaky across chipsets, distros, or daemon versions is worse than no feature. Every feature handles "not supported here" gracefully — log a single skip line, move on, never crash. Status surfaces per-feature `applied / skipped (reason) / failed (reason)`.

Anything that could weaken Fedora's hardening or break a working setup is opt-in, default off, with a wiki page that lists the concrete failure modes. Crypto-policies stay alone, machine-id stays alone, SSH config stays alone.

Binary stays ≤3.75 MB stripped (release-time hard cap; see `.github/workflows/release.yml`). Any dependency that adds more than 200 KB needs a justification.

## Things I haven't decided

These don't block phase A but I'd like to settle them before they bite.

Hostname wordlist: probably a curated list of about 500 router-flavored words rather than petname-style adj+noun.

`proteus pin` granularity: probably both interface name and NM connection profile, with the connection profile preferred when ambiguous.

Drop-detect on Ethernet: I think rotate-on-probe-fail still applies for parity, even though a wired drop is usually the cable being pulled.

DUID rotation scope: per-interface feels more isolating than system-wide.

CI runner: start with GitHub-hosted Fedora container, switch to self-hosted only if it hurts.

## Out of scope, with what to use instead

I want to be honest about this so people don't over-trust the tool. The `threat-model` wiki page expands on each.

Browser fingerprints (Canvas, WebGL, fonts, screen, language) — Tor Browser, librewolf, or Brave's built-in randomization. Browsers solve this from inside the process.

TLS ClientHello (JA3/JA4) — same story. Can't normalize across NSS, BoringSSL, rustls, OpenSSL, GnuTLS from outside, and apps override anyway.

SSH client fingerprint (HASSH) — your `ssh_config` is yours.

DNS resolution policy beyond ECS strip — dnscrypt-proxy, NextDNS client, AdGuard Home, knot-resolver, Pi-hole. DNS is its own complex world and deserves its own tooling.

Tracker IDs in app traffic — Pi-hole, NextDNS, uBlock Origin.

Traffic correlation — Tor or Mullvad VPN.

RF L1 fingerprinting (analog transmitter characteristics) — software can't fix this. A swappable USB Wi-Fi adapter is the real answer; Proteus only narrows the capture radius via opt-in TX power reduction.

Bluetooth BR/EDR (classic) BD_ADDR rotation — chipset-specific, deferred until I have a known-good chipset matrix.

`/etc/machine-id` rotation — TPM, journald, dbus all reference it. Real breakage risk.

Per-SSID profiles — config schema reserves the namespace, deferred to v2.

macOS / Windows ports — possible but a significant rewrite of the platform layer. CLI, config, and wiki layers stay portable for free thanks to the `Platform` trait. No commitment, no v1 work.

A GUI — Proteus is CLI-first. The CLI is wrappable so someone can build a GUI later without forking.

Telemetry, update checks, analytics — never.

## What's next

Once this reads right, I commit it as the first commit on `main` and push. Then phase A: Cargo project, src skeleton, three wiki pages, README, LICENSE files.
