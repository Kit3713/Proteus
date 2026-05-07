DHCP option suppression. Per-NetworkManager-connection settings written over DBus. Pairs with MAC rotation so the client identity moves with the hardware identity.

For the mental model, read `proteus wiki concepts` first. For MAC rotation context, `proteus wiki mac-recipes`. For IPv6 specifics including DUID derivation, `proteus wiki ipv6`. For hostname interactions, `proteus wiki hostname-recipes`.

## What DHCP leaks

Every DHCP DISCOVER and REQUEST is a broadcast. The local infrastructure — APs, switches, DHCP relays, captive-portal vendors, anyone passively listening on the L2 segment — sees the full option payload. Even with a fresh MAC every join, the option payload can correlate sessions back to the same device, the same OS, or the same human.

Rotating a MAC and then announcing `hostname=lenny-thinkpad` in the next DISCOVER is theatre. The hostname is one packet later. The vendor class is in the same packet. The client identifier may be stable across MAC changes by default. The DUID definitely is.

This page covers what Proteus suppresses, what it deliberately leaves alone, and how to verify the result on the wire.

## Options Proteus suppresses (DHCPv4)

**Option 12 (Hostname).** Broadcasts your kernel hostname to every DHCP server. Proteus disables sending. If you need to send something, the hostname rotation feature (`proteus wiki hostname-recipes`) gives you a generic or wordlist-derived name.

**Option 60 (Vendor Class Identifier).** Banner string identifying the DHCP client and often the OS — `MSFT 5.0`, `android-dhcp-13`, `dhcpcd-9.4.1`. Proteus clears it.

**Option 61 (Client Identifier).** Variable: sometimes a DUID, sometimes the MAC, sometimes a hostname. If it's stable across MAC rotations, the rotation is defeated. Proteus configures NetworkManager to derive option 61 from the current MAC, so it rotates as a unit.

**Option 81 (Client FQDN).** The newer DDNS-update mechanism. Leaks hostname plus domain. Proteus clears it.

## DHCPv6

**DUID (DHCP Unique Identifier).** Persistent across MAC rotations by default. Spec-blessed, fingerprint-grade. Proteus rotates the DUID per-interface alongside MAC by setting NetworkManager to a link-layer DUID derived from the current MAC. See `proteus wiki ipv6` and `proteus wiki concepts` for why DUID coupling is mandatory rather than optional.

**Vendor Class.** Same story as v4 option 60. Cleared.

## How Proteus does it

Settings are written per NetworkManager connection over the NM DBus API. No `nmcli` shelling. The keys set on each managed connection:

```
ipv4.dhcp-send-hostname            = no
ipv4.dhcp-fqdn                     = ""
ipv4.dhcp-vendor-class-identifier  = ""
ipv4.dhcp-client-id                = "mac"
ipv6.dhcp-duid                     = "ll"
```

`ipv4.dhcp-client-id = "mac"` couples option 61 to the current MAC. `ipv6.dhcp-duid = "ll"` selects a link-layer DUID derived from the current MAC, so the DUID rotates whenever the MAC does. Both keys are the load-bearing pieces — without them, suppressing 12/60/81 still leaves a stable client identity in 61 and the DUID.

## Per-connection vs system-wide

Proteus writes per-NM-connection because that composes cleanly with NetworkManager workflows: pinning a connection, exporting it, re-importing on another host, all keep the suppression intact. New connections created by the user inherit nothing from old ones, so `proteus apply` is also the way to re-cover newly-added Wi-Fi networks.

System-wide DHCP suppression at the `dhclient` or `dhcpcd` level is out of scope. Proteus is an NM-aware tool. If you are not using NetworkManager on the relevant interface, see "Detect-and-defer" below.

## What we don't touch

**Static DHCP reservations.** Server-side concern. If your network admin has pinned your MAC to an IP, that's their config, not ours. Rotating your MAC will of course break the reservation; that's expected.

**DHCPv4 option 55 (Parameter Request List).** The ordered list of options the client asks the server for. The PRL is itself a fingerprint — projects like FingerBank classify clients by it. Proteus does not touch it. NetworkManager does not expose a knob, and altering the PRL risks breaking real network operation (servers may withhold options the client didn't request). Documented limit.

**DHCPv6 IA_TA (temporary IPv6 addresses).** IPv6 privacy addresses live in the kernel, not the DHCP client. See `proteus wiki ipv6`.

**Server-side anti-fingerprinting.** What the DHCP server logs, retains, or shares is the operator's policy. Out of scope.

## Verification

Watch the actual exchange:

```
sudo tcpdump -n -i wlan0 -vv 'udp port 67 or udp port 68'
```

Trigger a fresh lease (`nmcli connection down <name> && nmcli connection up <name>`) and inspect the DISCOVER and REQUEST. There should be no Option 12, no Option 60, no Option 81. Option 61 should match the current MAC.

Inspect the resulting NM config:

```
nmcli connection show <name> | grep -E 'dhcp|duid'
```

Expected values match the keys in "How Proteus does it" above. Anything else is drift — check `proteus status` for a `failed` line, or re-run `proteus apply`.

Once Phase G ships, `proteus diff` confirms the applied config against NM defaults in a single command.

## Detect-and-defer

If the interface is not managed by NetworkManager — for example a `dhclient` invocation from a custom init script, or `systemd-networkd` with its own DHCP client — Proteus skips the DHCP changes for that interface and surfaces this in `proteus status` as `skipped (not NM-managed)`. No silent skip. See `proteus wiki concepts` for the detect-and-defer pattern.

## Configuration

```toml
[dhcp]
suppress_hostname     = true   # option 12 + 81
suppress_vendor_class = true   # option 60
rotate_client_id      = true   # couple option 61 + DUID to MAC
```

Defaults are all `true`. Turning `rotate_client_id` off is the most common footgun: it leaves a stable DUID across MAC rotations, which silently undoes most of the point of rotating MACs in the first place.

If a specific network needs the original behavior — for example a DHCP reservation keyed on a particular client identifier — pin the connection with `proteus pin <connection>` rather than disabling these knobs globally.

## Cross-refs

- `proteus wiki concepts` — DUID coupling rationale, detect-and-defer pattern
- `proteus wiki ipv6` — IPv6 IID, stable-privacy, kernel-side temp addresses
- `proteus wiki hostname-recipes` — hostname rotation when you need to send a non-empty name
- `proteus wiki mac-recipes` — MAC rotation triggers and pinning
