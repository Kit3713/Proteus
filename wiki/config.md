Reference for every Proteus config knob: location, default, risk. Cross-references the per-feature wiki pages where each knob is discussed in depth.

This page documents the full schema across the v0.1–v0.4 cycles. Unknown keys inside known sections are rejected by `deny_unknown_fields` so a typo surfaces at parse time. Use `proteus config keys` to enumerate every supported key; use `proteus config validate` to parse-check a file.

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
enabled = true
rotation_interval = "2h"       # systemd timer cadence
oui_pool = ["apple", "intel", "samsung", "dell", "random-locally-administered"]
```

Knobs:

- `enabled` — master switch for MAC rotation.
- `rotation_interval` — duration string (`s`, `m`, `h`, `d`). Default `2h`. Driven by `proteus-rotate.timer`. Set to `0` to disable scheduled rotation while leaving probe-driven rotation intact.
- `oui_pool` — which OUI prefixes to draw from. Vendor names map to a curated OUI list compiled into the binary. `random-locally-administered` sets the LAA bit and the unicast bit (locally-administered, valid). Mix freely.

Gateway-MAC and ARP-table collision avoidance are always-on invariants, not config knobs — Proteus refuses to assign a MAC that collides with the live segment.

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

- `enabled` — master switch.
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

Cross-ref `proteus wiki dhcp`.

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

Cross-ref `proteus wiki ipv6`.

### `[discovery]`

```toml
[discovery]
mdns_silence = true            # silence the mDNS responder via systemd-resolved drop-in
llmnr_silence = true           # disable LLMNR via systemd-resolved drop-in
ssdp_block = false             # OPT-IN; breaks KDE Connect
wsd_block = false              # OPT-IN; breaks WSD printers
```

Knobs:

- `mdns_silence` — when true, ships an `MulticastDNS=` drop-in that disables the local mDNS responder and resolver via systemd-resolved (paired with `[resolved]`'s drop-in for the LLMNR pairing).
- `llmnr_silence` — when true, ships an `LLMNR=` drop-in that disables Link-Local Multicast Name Resolution via systemd-resolved.
- `ssdp_block` — OPT-IN. Blocks Simple Service Discovery Protocol via the nftables `proteus` table. Breaks KDE Connect, some smart-home discovery, DLNA. Default off because the breakage is loud and surprising.
- `wsd_block` — OPT-IN. Blocks Web Services Dynamic Discovery. Breaks WSD-only printers (some HP, some Brother models). Default off for the same reason.

NetBIOS, WPAD, and NTP normalization are owned by other sections (`[stack]`, NM per-connection settings, and `[ntp]` respectively). See those sections.

Cross-ref `proteus wiki discovery`.

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

Cross-ref `proteus wiki stack-fingerprint`.

### `[probes]`

```toml
[probes]
quorum_n = 3                   # need this many failures to declare down
quorum_total = 4               # out of this many endpoints
interval = "5m"
cooldown = "60s"
endpoints = ["1.1.1.1:443", "8.8.8.8:443", "9.9.9.9:443", "142.250.190.78:443"]
```

Knobs:

- `quorum_n` / `quorum_total` — declare "down" only when at least `quorum_n` of `quorum_total` probes fail. Defaults `3 of 4` are robust against single-endpoint flakiness without missing real outages. `quorum_n` must be ≤ `quorum_total`.
- `interval` — duration between probe rounds. Default `5m`. Lower = faster reaction, higher beacon load.
- `cooldown` — minimum gap between rotations. Default `60s`. Gives DHCP, RA, and IPv6 DAD time to converge after a rotation.
- `endpoints` — TCP-connect targets. Use IPs (not hostnames) to avoid letting a broken resolver cause rotations. Port `443` is conventional. ICMP echo is the fallback when TCP-connect is blocked.

The probe-driven rotation cadence is owned by `proteus-check.timer`; disable it via `proteus timer disable check` rather than a config knob.

Cross-ref `proteus wiki probes`.

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

Cross-ref `proteus wiki captive-portals`.

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

BR/EDR (classic) BD_ADDR rotation is chipset-specific HCI territory and intentionally not exposed; see `docs/PLAN.md`. Cross-ref `proteus wiki bluetooth`.

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

Cross-ref `proteus wiki enterprise-wifi`.

### `[dns]`

```toml
[dns]
strip_edns_client_subnet = true    # default true; HARD GUARD against detect-and-defer
```

Knobs:

- `strip_edns_client_subnet` — sets a systemd-resolved drop-in disabling EDNS Client Subnet. The hard guard is non-negotiable: if Proteus sees `dnscrypt-proxy`, Pi-hole, AdGuard Home, a custom `/etc/resolv.conf`, or any non-Proteus drop-in under `/etc/systemd/resolved.conf.d/`, it refuses to apply, names the detected tool in `proteus status`, and exits clean. Your DNS setup wins, every time.

This is the only DNS knob Proteus exposes. Anything beyond ECS-strip is somebody else's domain. Cross-ref `proteus wiki dns`.

### `[rf]`

```toml
[rf]
tx_power_reduce = false        # opt-in; reduces capture radius
tx_power_reduction_db = 6
```

Knobs:

- `tx_power_reduce` — OPT-IN. Reduces Wi-Fi TX power so the capture radius for passive listeners is smaller. Default off in `off`/`min`/`low`/`med` profiles; default **on** in `high` and `agr`. Per-knob overrides beat the profile baseline either direction.
- `tx_power_reduction_db` — `u8` count of dB to reduce by, applied as `regulatory_max - (db × 100) mBm`. Default `6` (≈ quarter the radiated power). Hardware caps the effective floor; if `iw reg get` returns no value, Proteus falls back to a conservative 20 dBm ceiling.

The shipped surface is `proteus rf status / apply / revert`. `apply` writes via `iw dev <iface> set txpower fixed <mbm>`; `revert` restores the cached pre-Proteus TX power exactly. L1 analog characteristics (oscillator drift, IQ imbalance, etc.) cannot be erased in software; this knob only narrows the capture radius. Cross-ref `proteus wiki rf-fingerprinting`.

### Rotation triggers

Rotation cadence is governed by `[mac] rotation_interval` for the scheduled timer. Event-driven rotation triggers (NM connection-up, link-flap, regulatory-domain change, captive-portal auth completion) live under `[events]` and are surfaced by the `proteus events run` daemon. Per-SSID overrides go through `[per_ssid."<ssid>"]` (see `proteus wiki per-ssid`).

Cross-ref `proteus wiki rotation`.

### `[timers]`

```toml
[timers.rotate]
interval = "2h"                # systemd cadence for proteus-rotate.timer

[timers.check]
interval = "5m"                # systemd cadence for proteus-check.timer
```

Knobs:

- `timers.rotate.interval` — systemd cadence for `proteus-rotate.timer`. Same syntax as `proteus timer set rotate --interval`: compact durations (`30m`, `1h`), named cadences (`hourly`, `daily`), raw calendar expressions (`*-*-* 06:00:00`), or the sentinel `never` which disables the timer.
- `timers.check.interval` — systemd cadence for `proteus-check.timer`. Same syntax.

Each profile carries a baseline cadence for both timers; `proteus apply` reconciles the configured value against the on-disk drop-in under `/etc/systemd/system/proteus-*.timer.d/`. User overrides win on a per-timer basis and survive profile changes — override-only-if-present, mirroring the bool toggles. The full per-profile table lives in `proteus wiki profiles`. Set from the CLI:

```sh
sudo proteus config set timers.rotate.interval 1h --yes
sudo proteus config set timers.check.interval 30s --yes
```

Cross-ref `proteus wiki timer` for the drop-in mechanics and the full duration grammar.

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
- Unknown keys inside known sections are rejected: `deny_unknown_fields` ensures a typo surfaces at parse time. Newer binaries reading older configs use defaults for fields the old config didn't set; an older binary reading a newer config will reject the unknown sections / keys.

## Choosing a preset

Annotated, ready-to-copy presets live in `examples/` at the repo root. Each is a starting point — read the file, copy the closest one, then tweak. Quick decision guide:

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

Substitute the preset filename you picked. The full index plus per-preset rationale lives in `examples/README.md`.

## Examples

### Minimal: just MAC rotation, leave the rest alone

```toml
[mac]
enabled = true

[discovery]
mdns_silence = false    # leave mDNS alone
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

[events]
enabled = true   # daemonized triggers on connection-up / portal-auth / link-flap
```

## Managing config via CLI

Hand-editing `/etc/proteus/config.toml` works, but `proteus config` is the first-class path. Read commands run as any user; mutating commands need root and explicit `--yes` (since they touch a privileged file).

```sh
# Show current config (alias for `proteus show-config`)
proteus config show
proteus config show --json

# Inspect a single value — prints user value when set, default otherwise
proteus config get mac.enabled
proteus config get mac.rotation_interval --json

# List every supported key with type + default
proteus config keys
proteus config keys --json | jq '.[] | select(.type == "bool")'

# Toggle a feature on or off (shorthand for set <component>.enabled true|false)
sudo proteus config enable bluetooth --yes
sudo proteus config disable dns --reason "using dnscrypt-proxy" --yes

# Set any single value; type is coerced from the default
sudo proteus config set mac.rotation_interval 1h --yes
sudo proteus config set probes.quorum_n 4 --yes

# Open $EDITOR (falls back to vi); validates after save
sudo proteus config edit

# Sanity check a hand-edit
proteus config validate
proteus config validate --json

# Reset back to defaults — section-scoped or whole-file
sudo proteus config reset dns --yes
sudo proteus config reset --yes   # nuclear: rewrites the entire file
```

Notes:

- `proteus config disable <component> --reason <text>` writes a `# Proteus: disabled at <iso8601> - reason: <text>` comment above the section. This is your explicit override path, complementing the automatic detect-and-defer (see `proteus wiki concepts`).
- `proteus config set` round-trips through `toml_edit`, so user comments and formatting in `config.toml` are preserved.
- Unknown keys exit 65 with a pointer to `proteus config keys`. Setting an out-of-range value or a string where a bool is expected exits 65 too.
- The rendered config never contains secrets, so `proteus config show --json` is safe to log or paste into bug reports.

## Cross-refs

- `proteus wiki cli` — full command-line reference, exit codes, JSON schemas.
- `proteus wiki troubleshooting` — what to do when a knob breaks something.
- `proteus wiki concepts` — mental model for identifiers, rotation, captive portals, managed files, revert.
- Per-feature wiki pages (`mac-recipes`, `hostname-recipes`, `dhcp`, `ipv6`, `discovery`, `stack-fingerprint`, `probes`, `captive-portals`, `bluetooth`, `enterprise-wifi`, `dns`, `rf-fingerprinting`, `rotation`) cross-referenced inline above.
