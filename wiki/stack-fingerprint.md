Your kernel's TCP and ICMP behavior is a fingerprint. Two laptops on the same Wi-Fi negotiating identical-looking ClientHellos can still be told apart by the SYN they sent first. Proteus narrows that signal to whatever Linux defaults look like with a few decade-old vestiges turned off.

## What "stack fingerprint" means

A passive observer — your ISP, the AP at the cafe, a traffic-correlation database — does not need to decrypt anything to identify the OS that opened a connection. The choices a kernel makes when it builds a SYN packet are stack-specific: which TCP options it sets, in which order, what window scale it advertises, what initial TTL the IP header carries, whether it answers ICMP type 13 or 15. Combine those with the kernel's ICMP echo behavior, MTU, and timestamp clock origin, and the result is a fingerprint that survives MAC rotation, DHCP scrubbing, and most VPNs.

The two reference tools:

- `nmap -O` — active probe; sends crafted packets and reads the response shape. Identifies kernel families and often kernel versions. See nmap(1) and the OS-detection chapter of the nmap reference.
- `p0f` — passive; reads SYN, SYN-ACK, and a few other shapes off the wire. Catalogs them against signatures for Linux 2.x through 6.x, Windows, BSD, Android, etc. See p0f(1).

There is also a "JA3 for TCP" line of work (TCP fingerprint hashing) used by some CDNs and DDoS-protection products. Same idea, different presentation.

Proteus addresses the easy wins. The hard wins — TCP option ordering, initial window, congestion algorithm — require kernel patches and are documented limits.

## What Proteus changes

A sysctl drop-in at `/etc/sysctl.d/95-proteus.conf` plus a small set of nft rules in the `proteus` table. Both managed files carry the standard `# managed by proteus` header (see `proteus wiki concepts`). See sysctl(8), sysctl.d(5), and nft(8).

### TCP timestamps (`net.ipv4.tcp_timestamps = 0`)

TCP timestamps (RFC 7323) carry a 32-bit value derived from a per-boot monotonic clock. The clock origin leaks system uptime to anyone who can compare two SYNs from the same host, and the timestamp itself is unique per host on the segment. Off by default after `proteus apply`.

Edge case: PAWS (Protection Against Wrapped Sequence numbers, RFC 7323 §5) needs timestamps on long-lived high-bandwidth flows where the sequence number can wrap inside a single window. If you are moving terabytes over one connection, consider keeping them. Documented limit.

Tradeoff: TCP loses one signal it uses for round-trip-time measurement on out-of-order packets. Mostly invisible to interactive users.

### ICMP info-replies and timestamps (nft drop)

ICMP type 15/16 (Information Request/Reply, RFC 792) and type 13/14 (Timestamp Request/Reply) are pre-DHCP-era discovery vectors. Reply 16 leaks the subnet mask. Reply 14 leaks the system clock. Nothing in modern userspace asks for these; many kernels still answer when probed.

Proteus drops them inbound on managed interfaces via the `proteus` nft table. Outbound is left alone — nothing in modern userspace generates these.

### Gratuitous ARP suppression (`net.ipv4.arp_announce = 2`)

Optional. Some networks need gratuitous ARP for failover detection (VRRP, keepalived neighborhoods). Setting `arp_announce = 2` makes the kernel pick a source IP for ARP requests that is on the same subnet as the target, which reduces the leak on link-up.

Off by default. Opt-in via `[stack] suppress_gratuitous_arp = true`.

### ICMPv6 and NDP hardening

Per-interface sysctls in the `net.ipv6.conf.<iface>.*` namespace, applied for every managed interface (see sysctl.d(5) for the per-interface key syntax):

- `ndisc_evict_nocarrier = 1` — flush NDP neighbor entries when the interface loses carrier. Forces fresh discovery on link-up so a stale neighbor cache does not bridge networks.
- `accept_redirects = 0` — drop ICMPv6 redirects. Mitigates spoofed-router attacks where a station on the LAN tries to route your traffic through itself.
- `accept_source_route = 0` — drop source-routed packets. Off by default in modern kernels but pinned here in case a distro changed it.

See `proteus wiki ipv6` for the rest of the IPv6 story.

### TCP initial window

Not changed. Linux's default TCP initial window (10 MSS since RFC 6928 / kernel 2.6.39) is widely used, and changing it would make you stand out more, not less.

### TCP options ordering (MSS, WSCALE, SACK_PERMITTED, TIMESTAMPS, NOP)

Cannot be changed without kernel patches. Documented limit. MSS, window scale, and SACK behavior are similar enough across modern Linux kernels that nmap and p0f mainly use them to distinguish Linux from non-Linux, not to pinpoint a Linux version.

## Detection prevention

Run these from another host on the same network — or from anywhere with reachability — to confirm the change took.

`nmap -O <your-ip>`:
- Before apply: identifies "Linux 5.x" or "Linux 6.x" with a confidence score, often the exact kernel range.
- After apply: should show "OS detection failed" or a much weaker signal across the candidate set.

`p0f -i any -p` (run on a third host that sees the traffic):
- Before apply: identifies the kernel family within "Linux 4.x-5.x" or "Linux 6.x".
- After apply: ambiguous, often falls back to a generic "unknown Linux" entry.

Neither tool is fooled completely. The remaining signal comes from the unpatchable parts (option ordering, initial window). Proteus is honest about that.

## What Proteus does not touch

- **TCP congestion algorithm** — kernel default (CUBIC, BBR on newer setups). Changing it can affect performance and is itself a fingerprint signal in the other direction.
- **MTU** — defaults. Changing breaks PMTUD edge cases and stands out on the wire.
- **TCP keepalive intervals** — defaults. Changing affects long-lived connections in unpredictable ways.
- **DSCP / TOS marking** — application-controlled; not a kernel knob.
- **SSH, TLS, OpenSSL** — out of scope. Application-protocol fingerprints. See `proteus wiki concepts` and the planned `threat-model` page.

## Configuration

```toml
[stack]
tcp_timestamps_off = true            # default true
icmp_info_replies_drop = true        # default true
icmpv6_hardening = true              # default true
suppress_gratuitous_arp = false      # opt-in
```

Every knob is independently toggleable. `proteus show-defaults` prints the full schema; `proteus show-config` prints the resolved values plus where each came from.

## Reverting

`sudo proteus revert` removes the sysctl drop-in and the `proteus` nft table. nft rules clear immediately on revert; a reboot also clears them since the table is not persisted independently. Sysctl values applied at runtime stay until the next reboot, when the kernel reads only the remaining drop-ins. To reset them in the same session without rebooting, run `sysctl --system` after revert.

The revert path is part of the standard invariant: it must work at every commit. See `proteus wiki concepts`.

## Verification

```
sysctl net.ipv4.tcp_timestamps          # 0 after apply
sysctl net.ipv6.conf.wlan0.accept_redirects  # 0 after apply
nft list ruleset | grep -A20 'table .* proteus'   # see our chains and rules
```

From another machine:

```
nmap -O <your-ip>
sudo p0f -i any -p
```

`proteus status` reports each `[stack]` knob as `applied / skipped (reason) / failed (reason)` like every other feature. No silent skips.

## Cross-refs

- `proteus wiki ipv6` — IPv6 stable-privacy, temp addresses, DUID, the rest of the NDP story
- `proteus wiki discovery` — firewall rules that block mDNS responder, LLMNR, SSDP, WSD, NetBIOS
- `proteus wiki rf-fingerprinting` — the L1 limit; software cannot fix analog transmitter characteristics
- `proteus wiki concepts` — managed-file headers, idempotency, the revert invariant
