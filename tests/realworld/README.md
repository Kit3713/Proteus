# Real-world testing harness

The roadmap (Milestone 6) calls for a manual checklist for verifying
Proteus on the kinds of networks it's designed for: coffee-shop,
hotel, conference, airport. None of this is automatable — the whole
point is to exercise Proteus against captive portals, real DHCP
servers, and operator-grade Wi-Fi infrastructure that doesn't exist
inside a container.

The harness here is **read-only**. It runs every Proteus probe / status
/ diagnostic against the network you're attached to and writes the
output to a single tarball you can attach to a GitHub Issue without
manually copy-pasting twelve commands. It does NOT mutate state; you
run `proteus apply` separately and re-run the harness.

## Running

```sh
sudo bash tests/realworld/probe.sh > /tmp/proteus-probe-$(date +%s).txt
```

Or capture a tarball with the structured outputs:

```sh
sudo bash tests/realworld/probe.sh --tarball /tmp/proteus-probe.tar.gz
```

The script is intentionally a single shell file with no external
dependencies beyond what `proteus` itself uses. It's safe to run on a
network you don't own — every command is read-only.

## What gets captured

- `proteus doctor --json` — full system + backend matrix
- `proteus status --json` — per-feature applied/idle/skipped state
- `proteus current --json` — current MACs per iface
- `proteus session --json` — current network session snapshot
- `proteus probe --json` — connectivity probe quorum
- `proteus portal status --json` — captive-portal classification
- `iw dev <iface> link` for every Wi-Fi iface — current AP, signal, channel
- `nmcli -t connection show` (when NM is available) — connection list
- `ss -tnp 'state established'` — current TCP sessions (anonymised)
- `ip -j addr` — current addresses
- `ip -j route` — routing table
- `dig +short @1.1.1.1 example.com` — sanity round-trip
- `cat /etc/resolv.conf` — DNS resolver config
- `journalctl -u NetworkManager -n 50` (when present) — recent NM events

## Privacy

The harness anonymises:

- Public-IP-shaped strings → `203.0.113.X` (RFC 5737 docs prefix)
- IPv6 prefix → `2001:db8::X` (RFC 3849 docs prefix)
- `ssid=` lines → `ssid=<REDACTED>`
- `passkey=` / `psk=` lines → omitted entirely

Inspect the tarball before attaching to a public issue. The tarball is
mode `0o600` and lives only where you point it.

## Reporting

Failures on real-world networks are **the highest-value contribution
right now** per the roadmap "How to help" section. File an issue with
the `realworld` label and attach the tarball. Include:

- the network type (`coffee shop`, `hotel`, `conference`, `airport`,
  `home`, `enterprise`)
- the persona / profile in use, if any
- whether `proteus apply` was run before the probe
- what symptom you observed (page didn't load, captive portal kept
  redirecting, NM repeatedly disconnected, etc.)

## Why a separate harness

`proteus doctor` is the one-shot "is the host healthy" check; this
harness is the per-network "what does the world look like from here"
capture. Different audience: doctor is for the operator running
Proteus, the harness is for whoever's helping the operator debug from
afar.
