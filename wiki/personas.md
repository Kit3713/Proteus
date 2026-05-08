Persona mode is the second of Proteus's two stealth strategies. Where the existing entropy-based randomizer disappears you into noise, persona mode shapes every fingerprint Proteus already controls to look like one specific real device — an iPhone 15, a MacBook Air M3, a Samsung TV, an ESP32-class IoT widget. From `nmap -O`, p0f, fpdhcp, Fingerbank, passive Wi-Fi capture, and OS-detection heuristics, the host should look like the chosen target.

This page is the field manual: what persona mode does and does not defeat, the schema, how to author your own, the built-in catalogue, and the verification checklist.

For the design rationale, see `docs/ROADMAP.md` (Milestone 2 of the v0.3 cycle). For the threat-model boundary in plain English, see `proteus wiki threat-model`.

## What persona mode is

A persona is a coherent set of values that the apply and rotate paths consult when shaping every identifier Proteus already controls — MAC OUI choice, MAC byte pattern, hostname template, DHCP option content (vendor-class identifier, FQDN, parameter-request-list ordering), IPv6 SLAAC behaviour, TCP/IP stack knobs (window scale, MSS, sysctl settings), mDNS posture, Bluetooth alias template, RF TX-power band.

Two flavours coexist in the same schema and the same on-disk representation:

- **`kind = "stealth"`** — cover-identity goal. Every marker mimics a specific real device. The user *looks like* that device to passive observers. `iphone-15`, `macbook-air-m3`, `samsung-tv-2024` are stealth personas.
- **`kind = "randomizer"`** — anonymity goal. Same schema but `oui_pool` is broad, `rotate_cadence` is set, and rotation drives the user into noise rather than mimicking a single device. The six built-in `Profile` baselines (`off`/`min`/`low`/`med`/`high`/`agr`) get identical-content randomizer mirrors so they show up alongside any user-authored randomizer recipes in `proteus persona list`.

The user picks one persona at a time via `proteus persona use <id>`. `proteus persona clear` drops back to plain randomizer mode driven by the existing `Profile` slider.

## What persona mode does not defeat

Persona mode shapes only what Proteus already controls. It does not touch any of the following, and assuming otherwise will lead you astray.

- **TLS fingerprinting** (JA3, JA4, ALPN, cipher-suite ordering) — your browser / library owns the ClientHello and Proteus has no business intercepting it. Use Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling.
- **Browser fingerprinting** (Canvas, WebGL, fonts, audio context, screen, plugins, the long tail) — solved from inside the browser process. Same recommendations.
- **Wireshark + payload-content analysis** — looking inside encrypted application flows is application-layer territory; persona mode does not pretend the host emits *no* traffic, only that the L1-L4 envelope looks like a different device.
- **Behavioural / timing analysis** — packet rhythm, traffic shape, idle-time clustering. A persona doesn't lie about *when* you transmit, only *what* you look like at the L1-L4 level. Tor and Mullvad are the right tools for traffic-correlation defence.
- **TLS-layer SNI**, **DNS query patterns**, **app-layer identifiers** (cookies, OAuth tokens, Matrix device IDs, OS-bundled installation IDs) — these belong to the application, not the host stack.
- **RF L1 analog hardware fingerprints** — clock-skew offsets, IQ imbalance, transmit-power ramp shapes. A persona changes the OS-controllable RF surface (TX-power, scan style); analog quirks survive every software-level change. A swappable USB Wi-Fi adapter is the only real answer.
- **Account boundaries** — if you log in to the same Google account from `iphone-15` and `macbook-air-m3` personas, the application correlates you across both. Persona mode is not account-laundering.

If your concern is on this list, persona mode is the wrong tool. The threat-model wiki page (`proteus help threat-model`) walks through which tool owns which problem.

## The schema

Persona files are TOML. Built-ins live under `data/personas/` and ship with the binary; user personas live under `/etc/proteus/personas/` and shadow built-ins on id collision. A user persona file is *exactly* the same shape as a built-in.

```toml
id = "iphone-15"               # kebab-case, must match the file stem
display_name = "iPhone 15 (iOS 17)"
kind = "stealth"               # "stealth" | "randomizer"
category = "phone"             # phone|laptop|tablet|tv|iot|router|console|printer|generic
oui_pool = ["apple"]           # vendor tokens or literal "aa:bb:cc"
mac_byte_pattern = ""          # optional shape for trailing 3 bytes
hostname_template = "{owner}s-iPhone"
mdns_advertise = true
bt_name_template = "{owner}'s iPhone"
rotate_cadence = ""            # only meaningful for kind = "randomizer"
notes = "iOS 17 baseline."

[dhcp_fingerprint]
vendor_class_identifier = "iPhone"      # DHCP option 60
fqdn = ""                                # DHCP option 81
parameter_request_list = [1, 3, 6, 15, 119, 252]  # DHCP option 55
host_name = ""                           # DHCP option 12

[tcp_stack]
window_scale = 6
mss = 1460
tcp_timestamps = true
tcp_sack = true
default_ttl = 64

[ipv6_traits]
use_temp_addresses = true
addr_gen_mode = "stable-privacy"  # eui64|stable-privacy|random
send_rs = true

[rf_traits]
tx_power_dbm = 0           # 0 = leave at regulatory max
scan_style = "passive"     # passive|active
power_save = "auto"        # on|off|auto
```

### Field reference

- **`id`** — kebab-case identifier; must match the file stem (`iphone-15.toml` ⇒ `id = "iphone-15"`). The schema check rejects any other shape with a wiki-linked error.
- **`display_name`** — what `proteus persona list` prints. Free-form, but try to keep it under 40 chars.
- **`kind`** — `stealth` (cover-identity) or `randomizer` (anonymity). Randomizer personas must set `rotate_cadence`; the schema check rejects randomizers with no cadence.
- **`category`** — informational filter; `proteus persona list --category phone` selects on this. Stealth personas pick a real category; randomizers usually set `generic`.
- **`oui_pool`** — vendor tokens (`apple`, `intel`, `samsung`, `dell`, `random-locally-administered`) or literal six-hex-digit prefixes (`aa:bb:cc`). The MAC generator picks one prefix per rotation. The integration follow-up adds Google, Microsoft, LG, TPLink, Asus, Roku, Amazon, generic-IoT to the vendor token table.
- **`mac_byte_pattern`** — optional shape for the trailing three bytes. Free-form for now; the apply path will define the wildcard syntax in the integration follow-up.
- **`hostname_template`** — string with `{n}` (digit), `{owner}` (first-name pool), `{wordlist}` (the existing 534-word router-flavoured list), and any persona-specific tokens. Rendered against `data/hostname-wordlist.txt` plus persona-specific pools at apply time.
- **`dhcp_fingerprint`** — DHCP option content. The integration follow-up routes these through the existing DHCP suppression path so the option path *sets* values from a persona instead of only suppressing.
- **`tcp_stack`** — abstract TCP/IP knobs the apply path translates to concrete `/proc/sys/net/...` writes. Window scale, MSS, timestamps, SACK, default TTL.
- **`ipv6_traits`** — SLAAC and ND. `addr_gen_mode` mirrors NM's `ipv6.addr-gen-mode` setting (`eui64` / `stable-privacy` / `random`).
- **`mdns_advertise`** — whether the persona advertises mDNS at all. Stealth personas for chatty devices (Apple, printers, TVs) leave this on; quiet personas (laptops in stealth mode) turn it off.
- **`bt_name_template`** — Bluetooth alias template; same token set as `hostname_template`.
- **`rf_traits`** — `tx_power_dbm` (0 means "regulatory max"), `scan_style`, `power_save`.
- **`rotate_cadence`** — only meaningful for `kind = "randomizer"`. Strings like `"30m"`, `"2h"`, `"never"`. The six built-in randomizer mirrors set this to match the existing `Profile` slider's cadences.
- **`notes`** — free-form. Author guidance, known limitations, references to source devices for audit trails.

## Authoring a custom persona

The schema is the same one the built-ins use. Cloning the closest built-in is almost always the fastest path:

```sh
sudo proteus persona new my-iphone --from iphone-15 --yes
sudo proteus persona edit my-iphone        # opens $EDITOR on /etc/proteus/personas/my-iphone.toml
proteus persona validate /etc/proteus/personas/my-iphone.toml
sudo proteus persona use my-iphone --yes
```

`proteus persona validate <path>` works on any file, so you can sanity-check before installing. The validator emits wiki-linked errors so a typo (`kind = "stealh"`) lands you on the right page rather than a stack trace.

To share a custom persona between machines:

```sh
proteus persona export my-iphone /tmp/my-iphone.toml   # works without root
sudo proteus persona import /tmp/my-iphone.toml --yes   # on the destination host
```

Permission warnings: `import` runs schema validation before installing; `export` warns when you're exporting a built-in (whose contents are already public — but you may not want a custom-tuned variant land on a world-readable path).

## Built-in catalogue

The current built-in set is 31 personas — 25 stealth covers and 6 randomizer mirrors of the existing aggressiveness slider.

| id | kind | category | display name |
|---|---|---|---|
| `iphone-15` | stealth | phone | iPhone 15 (iOS 17) |
| `iphone-13` | stealth | phone | iPhone 13 (iOS 16) |
| `pixel-8` | stealth | phone | Pixel 8 (Android 14) |
| `galaxy-s24` | stealth | phone | Galaxy S24 (One UI 6) |
| `macbook-air-m3` | stealth | laptop | MacBook Air M3 (Sonoma) |
| `thinkpad-x1-carbon` | stealth | laptop | ThinkPad X1 Carbon (Linux) |
| `samsung-tv-2024` | stealth | tv | Samsung Smart TV 2024 (Tizen 8) |
| `chromecast` | stealth | tv | Chromecast (Google TV) |
| `nintendo-switch` | stealth | console | Nintendo Switch |
| `playstation-5` | stealth | console | PlayStation 5 |
| `router-tplink` | stealth | router | TP-Link Router (Archer AX) |
| `iot-generic` | stealth | iot | Generic IoT Device |
| `printer-generic-hp` | stealth | printer | HP Printer (Generic) |
| `randomizer-off` | randomizer | generic | Randomizer — Off |
| `randomizer-min` | randomizer | generic | Randomizer — Min |
| `randomizer-low` | randomizer | generic | Randomizer — Low |
| `randomizer-med` | randomizer | generic | Randomizer — Med (default) |
| `randomizer-high` | randomizer | generic | Randomizer — High |
| `randomizer-agr` | randomizer | generic | Randomizer — Aggressive |

`proteus persona list --json` is the machine-readable form for wrappers. `proteus persona show <id>` prints the full schema for a single persona.

## CLI reference

- `proteus persona list [--kind stealth|randomizer] [--category phone|laptop|...] [--json]` — enumerate everything, filterable.
- `proteus persona show <id>` — full schema dump.
- `proteus persona use <id> [--apply] --yes` — set `[persona] active = <id>` in config; `--apply` runs `proteus apply` afterwards once the integration lands.
- `proteus persona clear --yes` — set `[persona] active = None`; back to plain randomizer mode.
- `proteus persona current [--json]` — active persona id and which fields would be persona-shaped.
- `proteus persona random [--kind ...] [--category ...] [--json]` — pick a random persona id; useful for scripted rotation between several covers.
- `proteus persona new <id> --from <existing-id> --yes` — clone an existing persona to `/etc/proteus/personas/<id>.toml`.
- `proteus persona edit <id>` — open in `$EDITOR`.
- `proteus persona validate <path>` — schema check; exit 0 / 1.
- `proteus persona import <path> --yes` — copy `<path>` into `/etc/proteus/personas/`.
- `proteus persona export <id> <path>` — copy persona `<id>` to `<path>`.

Mutating commands (`use`, `clear`, `new`, `edit`, `import`) require root and `--yes`. Read commands (`list`, `show`, `current`, `random`, `validate`, `export`) work for any user.

## Verification checklist

Persona mode is only useful if you can verify the cover. From a second host on the same LAN, before and after `proteus persona use <id>`:

```sh
# OS-detection comparison
nmap -O <your-ip>

# DHCP-trace from the segment
sudo tcpdump -i any -nn 'port 67 or port 68'

# mDNS advertisement comparison
avahi-browse -ar

# IPv6 SLAAC traits (look at the IID derivation)
ip -6 addr show

# Passive Wi-Fi capture (requires monitor mode)
sudo tcpdump -i wlan0mon -nn 'type mgt subtype probe-req'
```

Compare the before/after captures. The OS detection should flip from "Linux 5.x" (or whatever the host actually runs) to the persona's target — iPhone, Samsung TV, etc. The DHCP option-60 should match the persona's `vendor_class_identifier`. The hostname pattern should match the persona's `hostname_template`. mDNS should advertise (or not) per the persona's `mdns_advertise` flag.

If any of those don't match, that is a bug. File it.

## Limits worth restating

A persona is a *cover* at L1-L4 plus the network-joining protocols. It is not invisibility, and it is not a guarantee — a determined adversary with active-probe capability, a session of traffic, or a single TLS handshake can still distinguish you from the real device the persona targets. The point is to make routine, automated correlation fail; persona mode shifts you from "that one Linux box" to "another iPhone on the segment", and that is enough to defeat the analytics platforms most public Wi-Fi networks run.

For everything else: `proteus help threat-model`.
