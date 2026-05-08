Recipes for confirming, on the wire and in `/etc/`, that Proteus is doing what it claims. Each section is a concrete command, an expected output snippet, and the interpretation that says "applied" or "broken". For the mental model, read `proteus wiki concepts` first.

Install once: `sudo dnf install tcpdump nmap bind-utils avahi-tools nftables iproute jq bluez`. Examples assume `wlan0`; substitute as needed.

## MAC rotation

The most-fingerprinted byte sequence on a Linux system. Verify the rotation happened on the wire and the OUI matches the configured pool.

```sh
# Before rotation
ip link show wlan0 | grep ether
# expected: link/ether aa:bb:cc:dd:ee:ff brd ff:ff:ff:ff:ff:ff

sudo proteus rotate --iface wlan0

# After rotation
ip link show wlan0 | grep ether
# expected: link/ether xx:yy:zz:aa:bb:cc — DIFFERENT from before
```

Confirm the OUI came from the configured pool (default `auto` biases toward chipset-realistic):

```sh
proteus current --json | jq -r '.interfaces[] | select(.name=="wlan0") | .mac'
# Take the first three bytes (the OUI). Check it against the IEEE OUI registry
# at https://standards-oui.ieee.org/ or `oui-tool` if installed.
# For the `random` pool the second hex digit of byte 0 should be 2/6/A/E
# (the locally-administered bit set).
```

Cross-ref `proteus wiki mac-recipes` for OUI pools, pinning, and collision avoidance.

## DHCP option suppression

DHCP DISCOVER/REQUEST is broadcast. Anyone on the L2 segment sees the option payload.

```sh
# Capture a fresh exchange (rotate forces DISCOVER + REQUEST)
sudo tcpdump -i wlan0 -nn -vv 'udp port 67 or udp port 68' -c 4
```

In the decoded `Vendor-rfc1048 Extensions` block:

- Option 12 (Hostname) — ABSENT.
- Option 60 (Vendor-Class) — ABSENT or empty.
- Option 61 (Client-ID) — matches the current MAC, not a stable DUID or hostname.
- Option 81 (Client-FQDN) — ABSENT.

NetworkManager's per-connection settings should agree:

```sh
nmcli connection show <profile-name> | grep -E 'dhcp-(send-hostname|fqdn|vendor-class|client-id)'
# expected:
#   ipv4.dhcp-send-hostname:               no
#   ipv4.dhcp-fqdn:                        --
#   ipv4.dhcp-vendor-class-identifier:     --
#   ipv4.dhcp-client-id:                   mac
#   ipv6.dhcp-duid:                        ll
```

If a key disagrees, re-run `sudo proteus apply` and check `proteus status` for a `failed` line.

Cross-ref `proteus wiki dhcp` for the per-option rationale and the load-bearing role of `dhcp-client-id=mac`.

## IPv6 rotation

Rotating MAC without rotating IPv6 leaves the IID as the per-network identifier.

```sh
# Before
ip -6 addr show wlan0
# Note the stable-privacy address and any "temporary" addresses

sudo proteus rotate --iface wlan0

# After (give SLAAC + DAD a moment)
ip -6 addr show wlan0
# stable-privacy address: changed (IID derives from new MAC + stable_secret)
# temporary addresses: regenerated against the same prefix
```

Kernel knobs Proteus sets:

```sh
sysctl net.ipv6.conf.wlan0.addr_gen_mode      # expected: 3 (stable-privacy, RFC 7217)
sysctl net.ipv6.conf.wlan0.use_tempaddr       # expected: 2 (prefer temp for outbound)
sysctl net.ipv6.conf.wlan0.ndisc_evict_nocarrier  # expected: 1 (flush NDP on carrier loss)
```

Cross-ref `proteus wiki ipv6` for the IID derivation, DUID coupling, and the down/up cycle that makes the new address visible immediately.

## mDNS responder silence

From another machine on the same LAN, the host should not appear:

```sh
# From peer
avahi-browse -arpt
# expected: NO entries with your hostname or IP
```

If it does appear, check the systemd-resolved drop-in:

```sh
sudo cat /etc/systemd/resolved.conf.d/10-proteus-discovery.conf
# expected:
#   [Resolve]
#   MulticastDNS=resolve   # resolve-only (do not respond)
#   LLMNR=no
```

If `avahi-daemon` is also installed, `proteus status` will note it. Stop or mask:

```sh
sudo systemctl disable --now avahi-daemon avahi-daemon.socket
```

Cross-ref `proteus wiki discovery` for the responder/resolver split, LLMNR, NetBIOS, SSDP, WSD.

## TCP timestamp suppression

A SYN with TCP timestamps leaks system uptime monotonically.

```sh
sysctl net.ipv4.tcp_timestamps    # expected: 0
```

Capture a handshake and inspect the SYN options:

```sh
sudo tcpdump -i wlan0 -nn -vv 'tcp[tcpflags] & tcp-syn != 0' -c 5 &
curl -s https://example.com >/dev/null
# Options block, typical:  [mss 1460,sackOK,nop,wscale 7]
# expected: NO "TS val" or "timestamp" entry in the options list
```

If you still see `TS val`, the drop-in didn't apply:

```sh
sudo cat /etc/sysctl.d/95-proteus.conf       # expected: net.ipv4.tcp_timestamps = 0
sudo sysctl --system | grep tcp_timestamps
```

Cross-ref `proteus wiki stack-fingerprint` for the PAWS edge case and the rest of the stack tweaks.

## ICMP info-reply drop

ICMP type 13/14 (Timestamp) and type 15/16 (Information) are pre-DHCP-era OS-fingerprinting vectors.

```sh
sudo nft list ruleset | grep -A 20 'table inet proteus'
# expected: input chain dropping
#   icmp type { timestamp-request, info-request, address-mask-request } drop

# From a peer
sudo nmap -sn -PP <your-ip>                    # expected: no Timestamp Reply (type 14)
sudo nping --icmp --icmp-type 15 -c 1 <your-ip>  # expected: no Information Reply, times out
```

If `table inet proteus` is missing, re-run `sudo proteus apply`. Cross-ref `proteus wiki stack-fingerprint`.

## EDNS Client Subnet strip

ECS leaks your /24 to authoritative DNS servers. Verify with Google's reflector:

```sh
dig +short txt o-o.myaddr.l.google.com
# Before: includes "edns0-client-subnet 203.0.113.0/24"
# After:  NO edns0-client-subnet line
```

Drop-in:

```sh
sudo cat /etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf
# expected:
#   [Resolve]
#   EDNSClientSubnet=no
```

If `proteus status` says `dns: skipped (detected dnscrypt-proxy)` (or pi-hole, AdGuard Home, custom resolv.conf), Proteus is deferring. The user's DNS setup wins, every time. Cross-ref `proteus wiki dns`.

## Bluetooth alias and discoverability

The adapter alias defaults to your hostname.

```sh
bluetoothctl show | grep -E '^\s*(Name|Alias|Discoverable):'
# expected:
#   Alias: BT Device       # or your configured generic
#   Discoverable: no
```

From a peer with Bluetooth on:

```sh
bluetoothctl scan le on            # or, classic: hcitool scan
# expected: your adapter should NOT appear (Discoverable: no)
```

RPA (Resolvable Private Address) support, where enabled, rotates the on-air BLE address every ~15 minutes under controller control:

```sh
btmgmt info
# expected: "current settings" includes "privacy"
```

Cross-ref `proteus wiki bluetooth`. BR/EDR BD_ADDR rotation is deferred.

## Hostname rotation

Hostname leaks via mDNS, DHCP option 12, screen-sharing UIs, and shell prompts. The three flavors must agree.

```sh
proteus current --json | jq '.hostname'
# expected: static, pretty, transient all agree

sudo proteus rotate
proteus current --json | jq '.hostname'
# If [hostname] rotate_with_mac = true:  values DIFFER from before
# If rotate_with_mac = false (default):  values UNCHANGED (rotate touches MACs only)
```

Authoritative check via systemd:

```sh
hostnamectl
# expected: Static, Pretty, Transient all the same string
```

Cross-ref `proteus wiki hostname-recipes` for wordlist/generic modes and rotate-with-mac tradeoffs.

## Network identity from another machine

The most honest verification: have a peer probe you.

```sh
# From a peer on the same LAN
sudo nmap -sn -PR <subnet>                               # ARP discovery; sees your MAC
sudo nmap -O <your-ip>                                   # OS detection
sudo nmap -p 137,138,139,1900,3702,5353,5355 <your-ip>   # NetBIOS/SSDP/WSD/mDNS/LLMNR
avahi-browse -arpt                                       # full Bonjour browse

# If you control the LAN gateway:
sudo tcpdump -i <gateway-iface> -nn -vv host <your-ip> -c 200
```

Anything beyond ARP, DHCP-with-suppressed-options, and your own application traffic is a leak — file it.

## State file inspection

```sh
sudo cat /var/lib/proteus/state.json | jq .
# Top-level keys:
#   .version              — schema version
#   .captured_at          — ISO timestamp of first run (sacred, never re-captured)
#   .original.hostname    — pre-Proteus hostname
#   .original.interfaces  — array of {name, permanent_mac}
#   .managed.interfaces   — current Proteus-assigned MACs and pin state
#   .managed.hostname     — current rotated hostname
```

The `.original.*` block is sacred — captured once, never overwritten. Only `proteus uninstall --purge` resets it. Cross-ref `proteus wiki internals`.

## Drift detection

Files Proteus writes under `/etc/` carry a managed-file header with a SHA of the expected content.

```sh
proteus diff
# expected: empty (no drift) OR one line per drifted file
```

`proteus diff` lands in phase G. Until then, manual check:

```sh
head -2 /etc/sysctl.d/95-proteus.conf
# expected:
#   # managed by proteus — do not edit
#   # expected-sha256: <64 hex>

tail -n +3 /etc/sysctl.d/95-proteus.conf | sha256sum
# Compare against the expected-sha256 above.
```

Mismatch means someone edited the file. Re-apply or revert.

The header is an edit-detection / tamper-hint primitive, not an integrity guarantee — header and body share the same root-owned file, so anything with write access can recompute the SHA after editing. The check is for catching honest manual edits and other-tool stomps, not for defending against an attacker who already has root. For real attestation, use the published binary's external `.sha256` (see `proteus wiki reproducible-builds`).

## End-to-end smoke test

```sh
sudo proteus apply
sleep 5

# Any feature not in {applied, skipped}
proteus status --json | jq '.features | to_entries[] | select(.value.state == "failed")'
# expected: empty output

# External verifications, in any order:
sudo tcpdump -i wlan0 -nn -vv 'udp port 67 or udp port 68' -c 4    # DHCP
avahi-browse -arpt                                                  # mDNS (from peer)
sudo nmap -sn -PR <subnet>                                          # MAC discovery
dig +short txt o-o.myaddr.l.google.com                              # ECS
bluetoothctl show | grep -E '(Alias|Discoverable):'                 # Bluetooth
```

All pass: Proteus is doing what it claims. Any fail: open the per-feature page from the cross-refs and walk its checks.

## Cross-refs

- `proteus wiki concepts` — managed files, header format, detect-and-defer, idempotency
- `proteus wiki mac-recipes` — MAC rotation, OUI pools, pinning
- `proteus wiki dhcp` — option 12/60/61/81 suppression, DUID coupling
- `proteus wiki ipv6` — IID, stable-privacy, DUID, NDP
- `proteus wiki discovery` — mDNS, LLMNR, NetBIOS, SSDP, WSD
- `proteus wiki stack-fingerprint` — TCP timestamps, ICMP drops, NDP hardening
- `proteus wiki dns` — ECS strip, the hard guard, detect-and-defer list
- `proteus wiki bluetooth` — adapter alias, discoverable, RPA
- `proteus wiki hostname-recipes` — three hostname flavors, rotate-with-mac
- `proteus wiki cli` — full command reference and exit codes
- `proteus wiki internals` — state.json schema, JSON output schemas
- `proteus wiki troubleshooting` — recovery paths when verification fails
