Probes answer one question: do we still have Internet? They run on a timer and feed the rotation logic. The whole point is to be conservative — one DNS hiccup or one dropped packet shouldn't trigger a MAC rotation, and a captive portal in the path should never trigger one.

Probes ship in phase C.

## What probes do

A probe round contacts a small set of known endpoints and asks each one a yes/no question: did the connection succeed? The answers feed a quorum vote. Quorum says "Internet is up" (do nothing), "Internet is down" (rotate), or "ambiguous" (do nothing and try again next round).

The single job is reachability — not health-check, not latency, not throughput. Details of how that's done live in `Probe protocol` below.

If you're tracing why a rotation happened, `proteus status` shows the last probe round's result and per-endpoint outcome. Per-round results land in journald under `proteus-check.service`.

## Quorum

Default: contact 4 endpoints in parallel, declare "down" only if 3 or more fail. The thresholds are configurable as `probes.quorum_n` (failures required) and `probes.quorum_total` (endpoints contacted).

The reason is asymmetric cost. A false positive (rotate when the link was actually fine) interrupts whatever you were doing and burns a fresh MAC for nothing. A false negative (don't rotate when the link is actually down) just means you wait one more probe interval before noticing. Quorum biases toward false negatives.

A 3-of-4 vote means a single endpoint going dark — Cloudflare hiccup, route flap to one provider, transient packet loss along one path — does not move the needle. You need a coordinated outage, which usually means your link, not the endpoints.

## Default endpoints

Four stable, public, popular IPs. Hard-coded as IPs, not hostnames, so DNS resolution problems on the host can't influence the probe outcome.

- `1.1.1.1:443` — Cloudflare resolver
- `8.8.8.8:443` — Google resolver
- `9.9.9.9:443` — Quad9 resolver
- `142.250.190.78:443` — a public-facing Google IP

These are all anycast, geographically distributed, and run by organizations that handle global outage budgets in seconds rather than minutes. If three of these four are unreachable from your laptop, your link is the problem, not their backbones.

You can replace the list in `/etc/proteus/config.toml`. Reasons you might:

- You don't want your traffic to include those four IPs (they're public DNS resolvers — popular but identifiable as a probe pattern if someone is looking).
- You're on a network that blocks one or more of them (some corporate filters block public DNS).
- You want to bias toward IPs your other traffic already touches, so the probes blend in.

If you change the list, prefer raw IPs over hostnames for the same DNS-independence reason.

## Probe protocol

TCP-connect to port 443 is the default. A successful 3-way handshake counts as "up"; a connection refused, reset, timeout, or unreachable counts as "down". This is cheap, supported everywhere, and indistinguishable from any other HTTPS-bound socket on your machine.

ICMP echo (ping) is the fallback for endpoints that block TCP on a given network. Some hotel and airport Wi-Fi networks drop outbound TCP to public DNS servers but allow ICMP, or vice versa. The probe code tries TCP first, falls back to ICMP only if every endpoint fails TCP, and surfaces the choice in `proteus status` so you know what's happening.

Neither method does any application-layer work. No TLS handshake, no HTTP request, no DNS query. The probe is reachability and nothing else.

## Cooldown

After a rotation, probes pause for 60 seconds before the next round. The freshly-rotated stack needs time to come up — DHCP lease, IPv6 router advertisement, IPv6 duplicate address detection, captive-portal interception detection — all of which take real seconds.

Without a cooldown you get a "rotate → probe → fail because the new MAC isn't ready yet → rotate again" loop. The cooldown breaks the loop.

Tunable as `probes.cooldown`. Don't drop it below 30 seconds unless you've measured your stack's bring-up time and know it's safe.

## Schedule

Probes run every 5 minutes by default, via `proteus-check.timer`. The timer fires the binary, which runs one probe round and exits. There is no daemon.

Tunable as `probes.interval`. Common adjustments:

- Lower to 1-2 minutes on a flaky LTE link if you want faster detection of an outage. Costs more probe traffic.
- Raise to 15-30 minutes on always-on Ethernet where rotations on connectivity loss don't really matter (a wired drop is usually the cable being pulled, and you'll know).

Disable probe-driven rotation entirely with `probes.enabled = false`. Scheduled rotation (`proteus-rotate.timer`) keeps running on its own clock; only the connectivity-driven path goes away.

## Failure categories

Each probe round produces one of four classifications. Exit codes and JSON output match these names exactly so a wrapper or future GUI can branch on them.

- `clear` — quorum says Internet is up. No action.
- `down` — quorum says Internet is down. Trigger rotation, subject to cooldown and pin-state.
- `portal-suspected` — the failure pattern looks like a captive portal injected a response (TCP succeeds to a host that shouldn't be answering, signature ICMP replies, redirected sessions). Do not rotate. See `proteus wiki captive-portals` for the dedicated portal flow that takes over from here.
- `inconclusive` — split result, e.g. 2 of 4 fail. Below the quorum threshold. No action; the next round decides.

The `portal-suspected` exit is the load-bearing one. Probe failures classified as portal-caused never trigger MAC rotation. That's how the "rotate behind a portal forever" loop is avoided. The portal classifier is intentionally separate from the probe quorum so the two can disagree usefully.

## Privacy note

The probe targets become part of your outbound traffic. By default that's four well-known public IPs, hit every 5 minutes from your machine. They're popular enough that they don't single you out — almost every laptop on a network is talking to at least one of these — but they are a pattern.

If your threat model treats probe traffic as a leak, you have three options:

- Replace `probes.endpoints` with IPs your machine already talks to regularly, so the probes blend in.
- Lengthen `probes.interval` so the cadence is less distinctive.
- Set `probes.enabled = false` and rely on scheduled rotation only.

Probes never go anywhere except the configured endpoint list. The "no telemetry, no update checks" invariant from the README applies here too: those four IPs are the entire outbound footprint.

## Configuration

```toml
[probes]
enabled = true
quorum_n = 3
quorum_total = 4
interval = "5m"
cooldown = "60s"
endpoints = [
    "1.1.1.1:443",
    "8.8.8.8:443",
    "9.9.9.9:443",
    "142.250.190.78:443",
]
```

All fields are optional; omitted ones use the defaults shown. Run `proteus show-defaults` to print the canonical defaults.

## Tuning

- **Flaky LTE.** Lower `quorum_n` to 2, or raise `quorum_total` to 5 by adding a fifth endpoint. Either way you make the quorum harder to trip on a single bad probe.
- **Always-on Ethernet.** Set `probes.enabled = false`. A wired drop is usually the cable being pulled and isn't a fingerprint-correlation event you need to react to.
- **Locked-down corporate Wi-Fi blocks public DNS.** Replace the endpoints with IPs your network actually allows. Or set `probes.enabled = false` and rely on scheduled rotation.
- **You're behind a captive portal regularly.** Don't tune probes. The portal classifier handles this; see `proteus wiki captive-portals`.

## Cross-references

- `proteus wiki rotation` — what happens when probes return `down`. Pin state, cooldown enforcement, OUI selection, collision avoidance.
- `proteus wiki captive-portals` — the portal classifier that intercepts `portal-suspected` outcomes and runs its own flow.
- `proteus wiki concepts` — the rotation-and-probes mental model in one page.
- `proteus wiki config` — full config schema. Lands in phase F.
