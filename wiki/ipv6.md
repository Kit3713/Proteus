IPv6 has its own identifier surface, separate from MAC. Rotating the MAC without thinking about IPv6 leaves stable per-network identifiers in plain sight. This page covers what leaks, what Proteus sets, and what stays the user's call.

> **Status (audit 2026-05):** IPv6 stable-privacy IID coupling, DUID rotation, and NDP hardening are **planned and not yet implemented**. There is no PR open for them yet. The `[ipv6]` config section described below does not exist in `src/config.rs`. Today, `proteus apply` does not write any `net.ipv6.conf.<iface>.*` sysctls and does not touch the DHCPv6 DUID — the page describes the planned design once IPv6 lands (likely co-shipped with PR #69's sysctl drop-in or alongside DHCP work in PR #73).

## The leak surface

**Interface Identifier (IID).** The lower 64 bits of an IPv6 address. Under EUI-64 (RFC 4291 Appendix A) the IID is derived from the MAC: split the 48-bit MAC, inject `FF:FE` in the middle, flip the universal/local bit. The MAC is recoverable from any global address. Rotating the MAC doesn't help if even one address on the interface still uses EUI-64 — that one address keeps leaking.

**Stable-privacy addresses (RFC 7217).** Linux can generate IIDs that are stable but not MAC-derived: `IID = F(prefix, iface, network_id, dad_counter, secret)`. Same network → same IID across reboots. That stability is the leak: a hotel Wi-Fi sees the same IID every visit, even with a rotated MAC, because the kernel's stable secret didn't change.

**Temporary addresses (RFC 4941, RFC 8981).** Short-lived random IIDs used as the source for outbound connections. Default-on in modern Linux but configurable. Lifetime is hours by default; the kernel regenerates them on a timer. These cover outbound traffic but not the address advertised in NDP for inbound.

**DHCPv6 DUID.** The client identifier sent in DHCPv6 requests. Persistent by default — written to disk and reused across reboots and interfaces. A rotated MAC with a stable DUID still correlates. See `proteus wiki dhcp` for the DUID details and rotation strategy.

**NDP / Router Solicitation.** The first Neighbor Solicitation or Router Solicitation frame after the link comes up carries the source MAC in the link-layer-address option. This happens before any IPv6 privacy machinery kicks in. A passive listener on the segment sees the MAC regardless of what addresses you eventually settle on.

## What Proteus does

Per managed interface, Proteus writes the following sysctls:

- `net.ipv6.conf.<iface>.use_tempaddr = 2` — prefer temporary addresses (RFC 8981) for outbound. This is the default on modern Linux, but Proteus sets it explicitly so a distro or admin override doesn't quietly disable it.
- `net.ipv6.conf.<iface>.addr_gen_mode = 3` — stable-privacy IID generation (RFC 7217). Combined with rotated MAC, the stable-privacy IID rotates too, because the kernel mixes the MAC into the IID derivation alongside its stable secret.
- `net.ipv6.conf.<iface>.ndisc_evict_nocarrier = 1` — flush NDP cache on carrier loss so a re-association doesn't replay neighbor entries that tie the new identity to the old one. Part of the NDP-fingerprint hardening pass.

DUID rotation is coupled with MAC rotation per-interface. Mechanism is in `proteus wiki dhcp`.

## What Proteus does not touch

- **Privacy extensions globally.** Proteus sets `use_tempaddr` per-interface for managed interfaces only. The `net.ipv6.conf.all.*` and `default.*` knobs are the user's choice.
- **Router Advertisement processing.** The kernel handles RAs. Proteus does not override `accept_ra`, `accept_ra_pinfo`, `accept_ra_rtr_pref`, or related knobs — overriding them breaks SLAAC on real networks.
- **Disabling IPv6.** Never. `disable_ipv6 = 1` is a privacy regression in many cases — it forces fallback to v4, which has its own correlation surface (sticky DHCPv4 client-id, no temp addresses, no privacy extensions). Proteus rotates v6 identifiers; it does not eliminate v6.
- **The kernel's stable secret.** `/proc/sys/net/ipv6/conf/<iface>/stable_secret` is set once by the kernel or systemd-networkd and stays. Proteus does not rewrite it; the MAC rotation is enough to drive IID rotation.

## Coupling with MAC rotation

When `proteus rotate <iface>` runs (phase B):

1. Set the new MAC. Mechanism in `proteus wiki mac-recipes`.
2. Bring the interface down, then up — forces IPv6 re-derivation. Without the down/up cycle the kernel may keep stale addresses for their full preferred lifetime.
3. Stable-privacy IID derives from the new MAC plus the existing stable secret — new IID.
4. New DUID derives from the new MAC for the next DHCPv6 exchange — see `proteus wiki dhcp`.
5. Kernel regenerates temporary addresses against the current prefix — new outbound IIDs.

The down/up step is what makes step 3 visible immediately rather than at the end of the previous IID's preferred lifetime.

## Verification

Look at the live address set:

```
ip -6 addr show wlan0
```

Expect to see at least one global-scope address with the `temporary` flag (the temp address used for outbound), plus a stable-privacy address used as the interface's primary global v6. After `proteus rotate wlan0`, both should change.

Watch v6 traffic and confirm the new IID appears on the wire:

```
tcpdump -n -i wlan0 -vv ip6
```

Confirm no EUI-64 is in use. Quick check — the IID (last 64 bits) should not contain `ff:fe` in the middle two bytes. Anything matching `*:*ff:fe*:*` in the IID was MAC-derived.

Check `addr_gen_mode` actually took:

```
cat /proc/sys/net/ipv6/conf/wlan0/addr_gen_mode
```

`3` is stable-privacy. `0` is EUI-64 (the leak). `1` is "no IID without stable_secret".

## Configuration

```toml
[ipv6]
enabled = true
use_temp_addresses = true
addr_gen_mode = "stable-privacy"   # or "eui64" (not recommended)
ndp_hardening = true
```

Defaults match the sysctls described above. `addr_gen_mode = "eui64"` is exposed for users on legacy networks that filter on EUI-64-shaped IIDs, but it negates the point of the page; Proteus warns loudly when set.

When `enabled = false`, Proteus leaves all `net.ipv6.conf.<iface>.*` knobs alone. Use this on systems where another tool (systemd-networkd with explicit `IPv6Token=`, NetworkManager `ipv6.addr-gen-mode`) is already managing the IID. Detect-and-defer applies the same way it does for DNS and NTP: if NM has an explicit non-default `ipv6.addr-gen-mode` on the connection profile, Proteus skips the sysctl write and surfaces the skip in `proteus status`.

## Edge cases

**Link-local addresses (`fe80::/10`).** Always present, derived per `addr_gen_mode`. With `eui64` they leak the MAC inside any link-local frame even when no global v6 prefix is on the link. `stable-privacy` covers link-local too — that's why the mode is set globally per-interface, not per-prefix.

**SLAAC plus DHCPv6.** When the router advertises both, the client takes a SLAAC address (subject to `addr_gen_mode`) and a DHCPv6-assigned address. Proteus rotates the SLAAC IID via MAC + stable-privacy and the DHCPv6 identity via DUID rotation; both paths need to be covered or the unrotated one keeps correlating.

**Address lifetimes.** Even after re-derivation, old temp and stable-privacy addresses linger until their preferred and valid lifetimes expire. The down/up cycle in step 2 of the rotation flow flushes them. Without it, you end up with both old and new addresses on the interface for hours.

**RFC 7217 secret rotation.** Proteus does not rotate `stable_secret` because the kernel stores it per-interface and rewriting it during a live session breaks any inbound connections that resolved to the old IID. MAC rotation is sufficient for the RFC 7217 derivation to produce a fresh IID; rotating the secret on top is overkill for the threat model and breaks more than it fixes.

**IPv6-only networks.** No DHCPv4 fallback path. The DUID still gets rotated on MAC change and the SLAAC + temp address paths still work. Nothing in the IPv6 handling assumes a v4 dual-stack — Proteus is correct on `464XLAT`, `NAT64`, and pure IPv6 segments.

**Containers and netns.** Sysctls under `net.ipv6.conf.<iface>.*` are namespace-scoped on Linux. Proteus only writes them in the host namespace for managed interfaces; container interfaces inside their own netns are not touched. Run Proteus inside the namespace if you need it there.

## Cross-refs

- `proteus wiki mac-recipes` — MAC rotation drives IID rotation; without it, stable-privacy stays stable.
- `proteus wiki dhcp` — DUID specifics, options 12/60/61/81 suppression, DHCPv6 client behavior.
- `proteus wiki concepts` — the identifier mental model, where IPv6 fits in the layered picture.
