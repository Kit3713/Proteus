Throttling and targeted-network-behavior detection is a research direction Proteus has thought about and explicitly deferred to post-v1. This page exists so users know it's an open problem we've considered, not a feature we're hiding.

Not shipped. Not on the v1 roadmap. Read the rest of this page if you want to know why.

## What this page is

A research direction Proteus has thought about but hasn't shipped: detecting when a network is throttling you, fingerprinting you across rotations, or behaving in a way that targets your specific traffic. Documented here for transparency about scope; not a feature.

If you came here looking for a "is this network throttling me?" command, the honest answer is "Proteus does not have one, here's why, and here's what to use instead today."

## What "throttling" and "targeted behavior" mean in this context

These are real adversarial behaviors. They differ from generic flakiness in being correlated with **you** specifically — your MAC, your client identifier, your traffic shape — rather than with the network as a whole.

### Throttling

- **MAC-based bandwidth throttling.** Some captive networks rate-limit by MAC after N MB transferred. Effective if your MAC stays stable, defeated by a fresh MAC. The hotel that gives you 500 MB free and sells you the rest is the canonical example.
- **DHCP rate-limit.** Some networks throttle or NAK DHCP renewals from the same MAC, or from a MAC that requests renewal "too often." A naive rotation tool that triggers DHCP storms gets caught here.
- **Per-MAC connection cap.** Some carrier-grade NATs and access points limit concurrent connections from one MAC, which manifests as page-loads stalling once you cross the cap. Hard to distinguish from generic NAT exhaustion.
- **Time-of-day or session-length throttling.** Some venues throttle clients that have been associated for more than N hours, or during peak periods. Looks like rolling congestion if you only watch throughput.

### Targeted network behavior

- **DNS hijacking targeted at specific MACs.** Rare but documented in some hotel and conference networks. The gateway returns a different answer to your DNS queries than to other clients on the same network.
- **HTTP content rewriting.** Some captive networks (and some ISPs) inject ads, redirect specific pages, or rewrite responses in flight. Mostly defeated by HTTPS-everywhere; not entirely.
- **TCP RSTs from the gateway.** A deauth-style attack at L4. Sometimes used to break VPN connections or long-lived flows that the operator wants you to abandon.
- **Probe responses that differ between sessions.** A captive portal or middlebox tracking you across sessions, returning different content on a re-visit than on a first visit.
- **DHCPNAK after rotating a MAC that was previously seen.** Signals the gateway remembers your old MAC and dislikes the change. Indistinguishable from a normal lease conflict without context.

These are real. Proteus's MAC rotation defeats some of them by accident (rotate, lose the per-MAC throttle bucket); detecting them as a category and reacting deliberately is a different problem.

## Why this isn't in v1

### Detection is hard

Distinguishing "throttled" from "congested" requires:

- A baseline. What's normal throughput here, on this network, at this time of day?
- A measurement window long enough to be statistically meaningful. A 5-second sample in a coffee shop tells you very little.
- Awareness of the current path's expected ceiling. LTE vs Wi-Fi vs Ethernet, distance from AP, contention with other clients, the AP's backhaul.
- Disambiguation from VPN overhead, slow servers, BufferBloat, AQM-induced queuing delay, and the dozens of other reasons a TCP flow can underperform.

A naive "throughput dropped 50% from where it was a minute ago, must be throttled" trips on every congested coffee shop, every busy conference Wi-Fi, every elevator ride that drops the laptop's signal by 10 dB. False positives lead to MAC rotation that breaks working sessions.

### Detection of targeted behavior is harder

You'd need to:

- Compare DNS responses against a known-good resolver. Chicken-and-egg with the DNS layer — Proteus deliberately doesn't own DNS, so the "known good" comparison endpoint is itself a configuration problem.
- Compare HTTP responses against an out-of-band fetch. Requires a second connection (more noise on the network, more identifiers leaked), breaks if you're behind a portal, and only works for unauthenticated GETs.
- Monitor for TCP RSTs from the gateway. Kernel-level packet capture — root only, high CPU, high battery cost on a laptop, and itself a fingerprint (the only laptops in the coffee shop running tcpdump are the suspicious ones).
- Build a per-network behavior baseline. Privacy concern in its own right — Proteus would be tracking your network experience over time, which is the kind of thing this tool exists to prevent other people from doing to you.

### Risk of false positives

A tool that rotates aggressively on suspected throttling will:

- Break working captive-portal sessions. The user authed at 9 AM, the tool decided at 11 AM that throughput "looks throttled," rotated, and now the portal wants the user to re-auth.
- Trigger DHCP renewal storms. Some networks throttle DHCP requests themselves — so a rotation-driven storm gets throttled, the tool reads that as more "throttling," and rotates again. Death spiral.
- Burn through OUI pool entries faster than configured. The pool exists to keep MACs looking realistic across rotations; a panicked tool empties the pool in an hour.
- Interact badly with the captive-portal logic. The portal classifier exists precisely so Proteus doesn't rotate behind a portal; an aggressive throttling-detector that disagrees with the portal classifier creates a contradiction inside the tool.

The cost of a false positive is high. The cost of NOT detecting throttling is "you're slow on this network for the rest of the session" — annoying, but not catastrophic, and the user can rotate manually if they want to.

The asymmetry is the same one the probe quorum encodes (see `proteus wiki probes`): bias toward false negatives, because false positives interrupt working sessions. A throttling detector with the same bias barely fires; a throttling detector without that bias breaks things.

## What we DO ship that helps

Several existing v1 features partially cover the throttling problem without trying to detect it directly.

- **Probe-driven rotation** (Phase C). If connectivity actually fails — quorum of probes can't reach the public anycast targets — Proteus rotates. Throttling that doesn't break connectivity is invisible to this. Throttling that does break connectivity is handled the same way any outage is handled.
- **Captive-portal aware rotation** (Phase C). Proteus does not rotate behind a portal, so we don't make throttling worse by triggering re-auth loops on networks that bind throttling to authenticated sessions.
- **Manual `proteus rotate`** (Phase B). If you suspect throttling, you can rotate explicitly. This is the today-answer. Cross-ref `proteus wiki rotation`.
- **`proteus status` per-feature visibility** (Phase A). You can see what Proteus has done; if your throughput is bad, you can diagnose whether Proteus is the cause without guessing.
- **Scheduled rotation cadence** (Phase C). The default 2h interval means even unnoticed throttling gets a fresh MAC twice a work session. Not a detector, but a passive defense against per-MAC throttle buckets that accumulate over hours.

The pattern: Proteus reacts to clear signals (no Internet, captive portal detected, user said rotate), and refuses to react to ambiguous signals (slow, weird, vibes-bad). That ambiguous space is exactly where throttling detection lives.

## Research directions for post-v1

If we ever ship throttling detection, plausible approaches:

- **Adaptive baseline.** Collect per-network throughput stats, detect divergence from the per-network baseline (NOT from a global baseline). The "this network is slow today vs how this same network was last Tuesday" framing is more honest than the global "your throughput dropped" framing. Privacy cost: Proteus retains per-network data over time, which is the kind of state this tool would otherwise refuse to keep.
- **DNS comparison.** Compare resolver responses against a single known-good resolver (1.1.1.1 over DoH) for a small set of sentinel records, flag drift. Requires user opt-in (a small DNS cost, a fingerprint of its own, and a chicken-and-egg problem with what counts as "known good"). Wouldn't catch hijacking that targets non-sentinel records.
- **Heuristic flags, not auto-rotation.** A `proteus suspicion` command that prints "your throughput on this network looks 30% below your baseline" and lets the user decide whether to rotate. Trades automation for honesty about uncertainty. This is the most likely shape of any future feature — read-only, advisory, opt-in.
- **Out-of-band probe.** Fetch a known-content URL via a different transport (LTE if Wi-Fi is connected, or vice versa) and compare against the in-band fetch. Detects rewriting. Expensive in battery, bandwidth, and configuration; doesn't work without a second transport.

None of these are simple. Each has its own false-positive story, its own privacy cost, and its own way of breaking when the network is weird in some way the heuristic didn't anticipate.

The general shape: any throttling-detection feature in Proteus would need to be opt-in, default-off, and biased hard toward telling the user rather than acting automatically. Anything that auto-rotates on suspected throttling violates the "no silent failures, no surprises" invariant the rest of the tool tries to hold.

## What to use instead today

- **Tor** for traffic correlation defense and to bypass throttling that targets your specific identifiers. The exit relay is what the network throttles, and you don't share an identifier with that exit across sessions.
- **Mullvad VPN** or another reputable VPN — same story, less anonymous than Tor but faster and friendlier for high-bandwidth use. The VPN endpoint is what the network throttles.
- **Manual rotation when you suspect bullshit.** `sudo proteus rotate` (Phase B once landed). Costs you nothing if you're wrong, gets you a fresh MAC if you're right.
- **Network observation tools.** `mtr` for path diagnostics and where packet loss appears, `iperf3` for throughput baselines against a server you control, `tcpdump` for packet-level observation, `dig @1.1.1.1 example.com` cross-referenced against your local resolver to spot DNS divergence. None of these are Proteus's job, all of them tell you more about whether you're actually throttled than a heuristic in Proteus could.
- **A second transport.** If you have LTE plus Wi-Fi, fetch the same resource over both and compare. Manual, but definitive for content-rewriting questions.

Throttling detection is a diagnostic problem. Proteus is an erasure tool. The two are related but not the same problem, and conflating them would make Proteus worse at its actual job.

## Your role

If you have ideas for sound throttling-detection heuristics — particularly ones with low false-positive rates against real adversarial networks (not just lab simulations) — open a discussion. This is genuinely an open problem and contributions are welcome.

Don't open a PR for a heuristic that hasn't been validated against real-world adversarial networks. The bar is "doesn't regress existing users on existing networks," not "looks plausible on a whiteboard." A throttling detector that fires on every congested coffee shop is worse than no throttling detector, because it makes Proteus untrustworthy in exactly the situations users need it to be trustworthy.

If you've collected packet captures of throttling or content rewriting in the wild and would be willing to share them (anonymized), that's also useful. Real-world data is the bottleneck on this problem, not implementation effort.

## Cross-refs

- `proteus wiki threat-model` — Proteus's overall scope, what's in and what's out.
- `proteus wiki probes` — what we DO detect (reachability), and the quorum logic that biases toward false negatives.
- `proteus wiki rotation` — manual rotation as the today-answer for suspected throttling.
- `proteus wiki captive-portals` — why naive rotation makes targeted-portal behavior worse, not better.

## A note on honesty

This page exists because the maintainer asked for throttling detection and the right answer was "no, here's why." Documenting "things we considered and rejected" is more useful than letting users wonder if a feature is missing or just not yet built.

The voice of this page is the voice of the rest of the wiki: terse, opinionated, and willing to argue against features that look reasonable on a whiteboard but break in practice. If you're reading this and disagree — particularly if you have a working approach we missed — open a discussion. The deferral isn't permanent, it's just honest about the current state of the art.
