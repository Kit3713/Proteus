Reference for every Proteus config knob: location, default, risk. Cross-references the per-feature wiki pages where each knob is discussed in depth.

This page documents the full schema across phases A through F. Phase A ships only `[mac]`, `[hostname]`, `[dns]`, `[discovery]`, `[probes]`; the rest land in their respective phases. Unknown sections and unknown keys are tolerated by `serde(default)` so a config written for a later phase still parses on an older binary, and vice versa.

## Location

- `/etc/proteus/config.toml` — system-wide. Requires root to edit.
- Override with `--config <path>` for testing or per-user setups. The override applies to that invocation only and never gets written back.
- Format: TOML 1.0.
- Defaults: `proteus show-defaults` prints the live built-in defaults. This page documents them; `show-defaults` is the source of truth if they ever drift.
- A missing file means "use defaults"; not an error.
- Every section and every key has a default. The minimum valid config is the empty file.

## Sections

### `[mac]`

```toml
[mac]
enabled = false                # default false in phase A; true in phase B+
rotation_interval = "2h"       # systemd timer cadence
oui_pool = ["apple", "intel", "samsung", "dell", "random-locally-administered"]
exclude_gateways = true        # never assign a MAC matching the gateway
exclude_arp_table = true       # never assign a MAC currently in the ARP table
per_connection = true          # per-NM-connection rather than per-device
```

Knobs:

- `enabled` — master switch for MAC rotation. Default off in phase A because rotation isn't implemented yet; flips to default-on once phase B lands.
- `rotation_interval` — duration string (`s`, `m`, `h`, `d`). Default `2h`. Driven by `proteus-rotate.timer`. Set to `0` to disable scheduled rotation while leaving probe-driven rotation intact.
- `oui_pool` — which OUI prefixes to draw from. Vendor names map to a curated OUI list compiled into the binary. `random-locally-administered` sets the LAA bit and the unicast bit (locally-administered, valid). Mix freely.
- `exclude_gateways` — recommended on. Collisions with the gateway are awkward and trigger ARP storms.
- `exclude_arp_table` — recommended on. Collisions with peers cause connectivity issues for both you and them.
- `per_connection` — recommended on for NM-managed networks. Per-connection composes better with NM's profile model than per-device. Set false to scope rotation to the device regardless of which NM connection is active.

Cross-ref `proteus wiki mac-recipes`.

### `[hostname]`

```toml
[hostname]
enabled = true
mode = "wordlist"              # "wordlist", "generic", "pinned"
pinned_value = ""              # used when mode = "pinned"
generic_value = "fedora"       # used when mode = "generic"
rotate_with_mac = false        # opt-in: hostname rotates each time MAC does
```

Knobs:

- `enabled` — master switch. Default off in phase A because the hostname mutator isn't built yet; flips on with phase D.
- `mode` — `wordlist` draws from the embedded ~500-word router-flavored list; `generic` always sets `generic_value`; `pinned` always sets `pinned_value`. The original hostname is cached on first run and is sacred regardless of mode.
- `pinned_value` — only consulted when `mode = "pinned"`. Must satisfy POSIX hostname rules (RFC 1123: alphanumeric and hyphens, ≤63 chars per label). Validation rejects with exit code 65.
- `generic_value` — only consulted when `mode = "generic"`. Defaults to `fedora` because blending into a Fedora install is the lowest-friction generic.
- `rotate_with_mac` — opt-in. When on, every MAC rotation also rotates the hostname (kernel + pretty + transient). Off by default because it splits per-host bash history and changes the shell prompt.

Cross-ref `proteus wiki hostname-recipes`.

### `[dhcp]`

```toml
[dhcp]
suppress_hostname = true       # option 12 + 81
suppress_vendor_class = true   # option 60
rotate_client_id = true        # option 61 + DUID coupling with MAC
```

Knobs:

- `suppress_hostname` — drop DHCP options 12 (hostname) and 81 (FQDN). Some DHCP servers expect a hostname for lease accounting; if your DHCP shows blank entries and you care, leave on but check with your network admin.
- `suppress_vendor_class` — drop option 60. The default vendor-class string includes the dhcpcd or NM version and is a strong fingerprint.
- `rotate_client_id` — rotate option 61 (client identifier) and couple the DHCPv6 DUID to the active MAC. Without this, DHCPv6 reissues the same DUID across MAC rotations and leaks correlation. On by default.

Lands in phase D. Cross-ref `proteus wiki dhcp`.

### `[ipv6]`

```toml
[ipv6]
enabled = true
use_temp_addresses = true
addr_gen_mode = "stable-privacy"   # or "eui64" (not recommended)
ndp_hardening = true
```

Knobs:

- `enabled` — master switch for IPv6 identity hygiene. Off disables Proteus's IPv6 work entirely; the kernel keeps doing whatever it was doing before.
- `use_temp_addresses` — RFC 4941 / 8981 temporary addresses for outbound traffic. Recommended on; rotates the IID on a kernel timer independent of MAC.
- `addr_gen_mode` — `stable-privacy` (RFC 7217) is the only recommended setting. `eui64` derives the IID directly from the MAC and leaks it; included as a knob for diagnostic purposes only. Do not set `eui64` unless you know exactly why.
- `ndp_hardening` — sysctl drop-in for ICMPv6/NDP fingerprint reduction. Tunes hop limit defaults, suppresses extraneous router solicitation behavior. Safe; on by default.

Lands in phase D. Cross-ref `proteus wiki ipv6`.

### `[discovery]`

```toml
[discovery]
mdns_responder = false         # disable mDNS announcements (default true → off in active config)
mdns_resolve = true            # keep mDNS resolution working
llmnr = false                  # disable LLMNR
netbios = false                # disable NetBIOS (samba nmbd)
ssdp_block = false             # OPT-IN; breaks KDE Connect
wsd_block = false              # OPT-IN; breaks WSD printers
wpad = false                   # disable WPAD via NM per-connection
ntp_normalize = true           # normalize systemd-timesyncd; skip if chrony/ntpd
```

Knobs:

- `mdns_responder` — when false, Proteus disables outbound mDNS announcements (`_workstation._tcp` and friends). Asymmetric with resolution: you can still discover other hosts but you stop announcing yourself.
- `mdns_resolve` — when true, `.local` resolution keeps working. Recommended on; flipping off breaks AirPrint and many home setups.
- `llmnr` — when false, Link-Local Multicast Name Resolution is disabled. Microsoft-era; safe to disable on Linux.
- `netbios` — when false, NetBIOS over TCP/IP is disabled (kills samba `nmbd`). Safe unless you actually browse Windows shares by NetBIOS name.
- `ssdp_block` — OPT-IN. Blocks Simple Service Discovery Protocol via firewalld/nftables. Breaks KDE Connect, some smart-home discovery, DLNA. Default off because the breakage is loud and surprising.
- `wsd_block` — OPT-IN. Blocks Web Services Dynamic Discovery. Breaks WSD-only printers (some HP, some Brother models). Default off for the same reason.
- `wpad` — when false, WPAD is disabled via per-connection NM settings. Recommended; WPAD is an exfil channel.
- `ntp_normalize` — when true, normalizes systemd-timesyncd's request signature via a drop-in. Detect-and-defer: skipped automatically if `chrony` or `ntpd` is installed; surfaced in `proteus status`.

Lands in phase E. Cross-ref `proteus wiki discovery`.

### `[stack]`

```toml
[stack]
tcp_timestamps_off = true
icmp_info_replies_drop = true
icmpv6_hardening = true
suppress_gratuitous_arp = false   # opt-in
```

Knobs:

- `tcp_timestamps_off` — sets `net.ipv4.tcp_timestamps = 0`. RFC 7323 §7.1 leaks system uptime; turning timestamps off plugs that. Edge case: PAWS protection on high-bandwidth long-lived flows. Documented in `proteus wiki stack-fingerprint`.
- `icmp_info_replies_drop` — drops ICMP type 15/16 (info request/reply) and address-mask requests via nft rule. Old OS-fingerprinting vector; safe to drop.
- `icmpv6_hardening` — sysctl drop-in for ICMPv6 fingerprint normalization. Hop limit defaults, RA acceptance behavior. Safe; on by default.
- `suppress_gratuitous_arp` — OPT-IN. Suppresses gratuitous ARP on link up. Off by default because it slows failover detection on some networks.

Lands in phase E. Cross-ref `proteus wiki stack-fingerprint`.

### `[probes]`

```toml
[probes]
enabled = true
quorum_n = 3                   # need this many failures to declare down
quorum_total = 4               # out of this many endpoints
interval = "5m"
cooldown = "60s"
endpoints = ["1.1.1.1:443", "8.8.8.8:443", "9.9.9.9:443", "142.250.190.78:443"]
```

Knobs:

- `enabled` — master switch for probe-driven rotation. Off disables `proteus-check.timer`; scheduled rotation via `proteus-rotate.timer` is unaffected.
- `quorum_n` / `quorum_total` — declare "down" only when at least `quorum_n` of `quorum_total` probes fail. Defaults `3 of 4` are robust against single-endpoint flakiness without missing real outages.
- `interval` — duration between probe rounds. Default `5m`. Lower = faster reaction, higher beacon load.
- `cooldown` — minimum gap between rotations. Default `60s`. Gives DHCP, RA, and IPv6 DAD time to converge after a rotation.
- `endpoints` — TCP-connect targets. Use IPs (not hostnames) to avoid letting a broken resolver cause rotations. Port `443` is conventional. ICMP echo is the fallback when TCP-connect is blocked.

Lands in phase C. Cross-ref `proteus wiki probes`.

### `[captive_portal]`

```toml
[captive_portal]
enabled = true
detect_url = "http://nmcheck.gnome.org/check_network_status.txt"
expected_response = "NetworkManager is online"
policy = "rotate-before-auth"  # or "preserve-mac", "ask"
fresh_mac_per_visit = true
```

Knobs:

- `enabled` — master switch for the captive-portal detector. Off and Proteus treats every probe failure as a real outage; loop risk behind portals.
- `detect_url` — URL to hit for portal classification. Default uses the same target NetworkManager uses. Keep `http://`, not `https://`; portals intercept TLS in opaque ways.
- `expected_response` — exact body string that means "no portal in the path". Anything else means intercept.
- `policy` — `rotate-before-auth` (default) gets a fresh MAC, then prompts auth; `preserve-mac` keeps the current MAC because some SMS-bound portals tie the auth ticket to it; `ask` is interactive.
- `fresh_mac_per_visit` — when true, known-portal SSIDs get a fresh MAC every visit regardless of the rotation schedule.

Lands in phase C. Cross-ref `proteus wiki captive-portals`.

### `[bluetooth]`

```toml
[bluetooth]
enabled = true
generic_alias = true           # set adapter alias to generic
discoverable = false           # default off
ble_rpa = true                 # enable BLE Resolvable Private Address mode where supported
```

Knobs:

- `enabled` — master switch for Bluetooth identity hygiene. Off and Proteus does not touch BlueZ.
- `generic_alias` — sets the adapter alias to a generic string instead of `Cory's MacBook Pro`. The alias is what shows up on every nearby phone.
- `discoverable` — default false. Discoverable mode advertises constantly; only useful while pairing.
- `ble_rpa` — enables BLE Resolvable Private Address mode where the controller supports it. Skipped silently (with a `skipped (controller does not expose privacy mode)` line in status) on chipsets that don't.

BR/EDR (classic) BD_ADDR rotation is chipset-specific HCI territory and intentionally not exposed; see `docs/PLAN.md`. Lands in phase B. Cross-ref `proteus wiki bluetooth`.

### `[enterprise_wifi]`

```toml
[enterprise_wifi]
anonymous_outer_identity = false   # OPT-IN; some auth servers reject
realm_strip_strategy = "auto"      # "auto" or "manual"
anonymous_realm = ""               # used when "manual"
per_connection_overrides = {}
```

Knobs:

- `anonymous_outer_identity` — OPT-IN. Replaces the 802.1X outer identity with `anonymous@<realm>`. Default off because some corporate auth servers reject mismatched outer identities.
- `realm_strip_strategy` — `auto` derives the realm from the inner identity; `manual` uses `anonymous_realm`.
- `anonymous_realm` — only consulted when strategy is `manual`. Format `example.com`. No leading `@`.
- `per_connection_overrides` — TOML inline table of per-NM-connection overrides. Empty by default; populated by `proteus pin --connection <name> ...` (lands later).

Lands in phase D. Cross-ref `proteus wiki enterprise-wifi`.

### `[dns]`

```toml
[dns]
strip_edns_client_subnet = true    # default true; HARD GUARD against detect-and-defer
```

Knobs:

- `strip_edns_client_subnet` — sets a systemd-resolved drop-in disabling EDNS Client Subnet. The hard guard is non-negotiable: if Proteus sees `dnscrypt-proxy`, Pi-hole, AdGuard Home, a custom `/etc/resolv.conf`, or any non-Proteus drop-in under `/etc/systemd/resolved.conf.d/`, it refuses to apply, names the detected tool in `proteus status`, and exits clean. Your DNS setup wins, every time.

This is the only DNS knob Proteus exposes. Anything beyond ECS-strip is somebody else's domain. Lands in phase D. Cross-ref `proteus wiki dns`.

### `[rf]`

```toml
[rf]
tx_power_reduce = false        # opt-in; reduces capture radius
tx_power_reduction_db = 6
```

Knobs:

- `tx_power_reduce` — OPT-IN. Reduces Wi-Fi TX power so the capture radius for passive listeners is smaller. Off by default because reduced range degrades range from your APs.
- `tx_power_reduction_db` — number of dB to reduce by. Default `6` (≈ quarter the radiated power). Range `0..=20`. Hardware caps the effective floor.

L1 RF analog characteristics cannot be erased in software; this knob only narrows the capture radius. Lands in phase E. Cross-ref `proteus wiki rf-fingerprinting`.

### `[rotation]`

```toml
[rotation]
interval = "2h"                # scheduled MAC rotation cadence
on_probe_fail = true           # rotate on probe quorum failure
on_link_change = true          # rotate when link comes up
on_ssid_change = true          # rotate on Wi-Fi SSID change
```

Knobs:

- `interval` — scheduled rotation cadence. Mirrors `mac.rotation_interval`; the `[rotation]` section is the policy surface, `[mac]` is the identity surface. If they disagree, `[rotation]` wins.
- `on_probe_fail` — rotate on probe quorum failure (subject to portal classification).
- `on_link_change` — rotate when the link comes up. Common for hopping between wired and wireless.
- `on_ssid_change` — rotate on Wi-Fi SSID change. Recommended on; otherwise the same MAC follows you across networks.

Lands in phase C. Cross-ref `proteus wiki rotation`.

## Risks at a glance

| Knob | Risk if enabled |
|------|----------------|
| `discovery.ssdp_block` | KDE Connect stops working |
| `discovery.wsd_block` | WSD-only printers (some HP, Brother) stop being discovered |
| `enterprise_wifi.anonymous_outer_identity` | Some corporate 802.1X servers reject |
| `mac.rotation_interval < 30m` | Some networks throttle DHCP renewal |
| `stack.suppress_gratuitous_arp` | Failover detection on some networks may slow |
| `hostname.rotate_with_mac` | Shell prompt changes; per-host bash history splits |
| `rf.tx_power_reduce` | Reduced range from APs |
| `ipv6.addr_gen_mode = "eui64"` | IID leaks the MAC; do not set unless diagnosing |
| `dhcp.rotate_client_id = false` | DUID stays sticky across MAC rotations; correlation hole |

## Validation

- `proteus show-config --json | jq .` — verify your config parses and see the merged values.
- Invalid TOML: exit code 65; the error names the offending line.
- Invalid value (bad hostname, bad duration string, bad enum variant, bad endpoint): exit code 65; the error names the field and the wiki page that explains valid values.
- Unknown keys are accepted silently. Older binaries reading newer configs will ignore fields they don't understand. Newer binaries reading older configs use defaults for fields the old config didn't set.

## Choosing a preset

Annotated, ready-to-copy presets live in [`examples/`](../examples/) at the repo root. Each is a starting point — read the file, copy the closest one, then tweak. Quick decision guide:

- Just want MAC rotation? `examples/minimal.toml`.
- Not sure where to start? `examples/standard.toml`. Recommended.
- Live on public Wi-Fi? `examples/captive-portal-heavy.toml`.
- Willing to lose KDE Connect / WSD printers for stronger silence? `examples/aggressive.toml`.
- Maximum privacy, accept significant breakage? `examples/paranoid.toml`.
- Have your own privacy stack and just want Proteus's read commands? `examples/disabled.toml`.
- Hacking on Proteus itself? `examples/development.toml`.

Install with:

```sh
sudo cp examples/standard.toml /etc/proteus/config.toml
sudo proteus apply
```

Substitute the preset filename you picked. The full index plus per-preset rationale lives in [`examples/README.md`](../examples/README.md).

## Examples

### Minimal: just MAC rotation, leave the rest alone

```toml
[mac]
enabled = true

[discovery]
mdns_responder = true   # leave mDNS alone
```

### Aggressive: rotate everything, opt-in to discovery blocks

```toml
[mac]
enabled = true
rotation_interval = "1h"

[hostname]
rotate_with_mac = true

[discovery]
ssdp_block = true       # accept KDE Connect breaking
wsd_block = true        # accept WSD printer discovery breaking
```

### Pinned: known network where you want stable identity

```toml
[mac]
enabled = false         # don't rotate at all

[hostname]
mode = "pinned"
pinned_value = "trustedlaptop"
```

### Captive-portal heavy: hotels, conferences, coffee shops

```toml
[captive_portal]
enabled = true
policy = "rotate-before-auth"
fresh_mac_per_visit = true

[rotation]
on_ssid_change = true
```

## Cross-refs

- `proteus wiki cli` — full command-line reference, exit codes, JSON schemas.
- `proteus wiki troubleshooting` — what to do when a knob breaks something.
- `proteus wiki concepts` — mental model for identifiers, rotation, captive portals, managed files, revert.
- Per-feature wiki pages (`mac-recipes`, `hostname-recipes`, `dhcp`, `ipv6`, `discovery`, `stack-fingerprint`, `probes`, `captive-portals`, `bluetooth`, `enterprise-wifi`, `dns`, `rf-fingerprinting`, `rotation`) cross-referenced inline above.
