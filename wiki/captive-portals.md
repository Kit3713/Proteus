Captive portals are first-class. Not a heuristic, not a check tacked onto the probe path. A primary concern with its own detector, classification, policies, and per-SSID memory.

This page is honest about the complexity. Detecting a portal is the easy part. Not breaking your session every five minutes is the hard part.

## Why first-class

Rotating a MAC behind a captive portal is a footgun. The shape of the bug:

1. You join the airport Wi-Fi. Captive portal greets you. You auth as MAC `X`.
2. The portal binds your session token to MAC `X`. Traffic flows.
3. Two hours later, the rotation timer fires. You're now MAC `Y`.
4. The portal sees an unknown client. Intercepts your next request, redirects to the splash page.
5. Probes fail because the portal is in the path. Probe-driven rotation logic concludes "connectivity lost" and rotates again. Now you're MAC `Z`.
6. You re-auth, the portal binds to `Z`, and the loop primes itself for the next timer.

In the worst case you're locked out (SMS-bound portals send a code per MAC; you've now requested four). In the typical case you've correlated four MACs to one identity in the portal operator's logs — the opposite of what Proteus is for.

A naive "detect a portal, then back off" check inside the probe loop misses the periodic-rotation timer entirely. The two paths have to coordinate. That coordination is what this page describes.

## Detection

A dedicated portal detector runs alongside probes. It's not part of the probe quorum — see `proteus wiki probes` for why probes stay narrow.

The detector hits a known endpoint and compares the response to an expected body. Defaults match NetworkManager's connectivity check so corporate environments that already whitelist it work without further config:

```toml
[captive_portal]
detect_url = "http://nmcheck.gnome.org/check_network_status.txt"
expected_response = "NetworkManager is online"
```

If the response body matches, the path is `clear`. If the request is intercepted (HTTP 200 with a different body, 302 to an auth host, TLS hijack), the path is `portal-required` or `portal-authed` depending on follow-up signals. If the request times out or errors in a way that doesn't fit either bucket, the path is `unknown`.

Both addresses families are tried where available. IPv6 sometimes works while IPv4 is intercepted, especially on home routers running carrier NAT — see edge cases below.

## Classifications

Four states. Surfaced in `proteus status` and emitted as a structured event in the JSON output.

- `clear` — Internet works, no portal in path. Normal operation.
- `portal-required` — Traffic is intercepted, you have not authed yet. Periodic rotation is suppressed. Probe failures are tagged portal-caused and do not trigger MAC rotation.
- `portal-authed` — You authed, but portal infrastructure remains in the path. Visible as DNS oddities (operator's resolver, occasional NXDOMAIN), occasional re-redirects, or probe response anomalies. Periodic rotation stays suppressed because rotating mid-session usually kicks you out.
- `unknown` — Detector inconclusive. Treated like `portal-required` for safety: rotation suppressed, probes don't trigger rotation. Surfaced in status so you know the detector is uncertain.

## Policies

Configurable via `captive_portal.policy`. Three options:

- `rotate-before-auth` (default) — On joining a network classified as `portal-required`, rotate to a fresh MAC before the user opens the portal page. Auth happens against the new MAC. Portal operator can't correlate this visit to any past visit.
- `preserve-mac` — Keep whatever MAC is currently assigned. The right choice for SMS-bound portals: the auth ticket is tied to your MAC across sessions, and rotating means receiving (and waiting for, and paying for) another SMS code. Also the right choice for some hotel networks that bind your room number to your MAC at check-in.
- `ask` — Interactive. The CLI prints the question to stderr and waits on stdin. Graphical wrappers can intercept this and present a dialog. Useful when you don't yet know which kind of portal you're behind.

The default is `rotate-before-auth` because that's the right choice for the most common case (one-time terms-of-service portals at airports, cafes, conferences). The other two exist because the wrong choice is annoying enough that "just rotate, always" would be a worse default.

## Suppression rules

Two invariants. Both protect against the loop described at the top of this page.

**Periodic rotation is suppressed while `portal-authed`.** Don't rotate behind a session you just paid for, waited for an SMS code for, or accepted terms of service for. The rotation timer logs a single skip line citing the classification and moves on.

**Probe failures classified as portal-caused never trigger MAC rotation.** This is the loop-prevention invariant. If the portal detector says the path looks intercepted, probe failures are tagged portal-caused and the rotation pipeline ignores them. Probes still run, status still surfaces the failures, but the trigger is suppressed.

These rules apply regardless of policy. `preserve-mac` and `rotate-before-auth` differ in what happens at network join, not in how rotation behaves once you're authed.

## Known-portal SSIDs

Some networks you'll join again. Your usual cafe, the conference venue, the airport you fly through monthly. Each visit should look uncorrelated to the operator.

State lives in `/var/lib/proteus/state.json` under `known_portal_ssids`. A small list, manually curated.

```sh
proteus portal mark <ssid>      # add an SSID to the known-portal list
proteus portal list             # print the list
proteus portal unmark <ssid>    # remove an SSID
```

When you join an SSID on the list, you get a fresh MAC for that visit, regardless of where the schedule timer is. Inside the visit, the suppression rules above apply normally — fresh MAC at join, then no further rotation until the next disconnect.

The `proteus portal mark` / `list` / `unmark` commands are phase C surface and ship as stubs in earlier phases. They print "not implemented in this phase" and point at this page.

## Browser helper

```sh
proteus portal open
```

Launches the portal page in your default browser. Tries `$BROWSER` first, then falls back to `xdg-open`. The detector identifies the redirect target so you don't have to type a URL.

Phase C. Earlier phases stub it out.

## Detection edge cases

The honest list. None of these are showstoppers but each is worth knowing.

**HTTPS-only portals.** Rare but they exist. The detector still works — even with TLS intercept, the response body verification catches the mismatch. The detector intentionally does not validate the certificate chain on the detection request alone, otherwise a portal MITM would look like a network failure rather than a portal.

**Walled gardens.** Some org Wi-Fi (university networks, larger conferences) routes you through a transparent proxy that looks indistinguishable from an authed portal forever. Classified as `portal-authed`. With `policy = "preserve-mac"` this is fine — you keep your MAC and rotation stays suppressed. With the default policy you'll rotate once at join and then sit, which is also fine.

**Dual-stack interception.** IPv6 sometimes works while IPv4 is intercepted, especially behind carrier-grade NAT. The detector tries both families and reports the worse of the two. If IPv4 is `portal-required` and IPv6 is `clear`, the path is `portal-required`: you can't trust traffic that might fall through to v4.

**Detector endpoint blocked.** Some networks block `nmcheck.gnome.org` specifically. The detector reports `unknown`, which the suppression rules treat conservatively. Configure `detect_url` to a different endpoint in this case (Apple's `captive.apple.com` and Microsoft's `www.msftconnecttest.com` are common alternates with documented response bodies).

**Slow portals.** A real portal can take 30+ seconds to redirect. The detector's per-attempt timeout is generous and it retries before falling through to `unknown`. If you're hitting `unknown` frequently on a known-good network, increase the timeout in config rather than reaching for `enabled = false`.

## Configuration

```toml
[captive_portal]
enabled = true
detect_url = "http://nmcheck.gnome.org/check_network_status.txt"
expected_response = "NetworkManager is online"
policy = "rotate-before-auth"  # or "preserve-mac", "ask"
fresh_mac_per_visit = true     # for known-portal SSIDs
```

Every field has a sensible default. The two you'll most often tune are `policy` (set to `preserve-mac` if you're regularly on SMS-bound portals) and `detect_url` (if `nmcheck.gnome.org` is blocked on your usual networks).

## Disabling

```toml
[captive_portal]
enabled = false
```

Disables detection and the rotation suppression rules along with it. Probes still run. If you're behind a portal with detection off, you may get the rotation loop described at the top of this page. The setting exists for two cases: you have your own portal handling outside Proteus, or you've established that you never join captive networks (rare for laptops, common for desktops).

There's no in-between. You can't keep suppression while disabling detection — suppression depends on the classification the detector produces. Either trust the detector or accept the loop risk.

## Cross-refs

- `proteus wiki probes` — probe quorum mechanics, why portal-caused failures don't trigger rotation
- `proteus wiki rotation` — what triggers rotation, how the timers and probes interact with portal classification
- `proteus wiki concepts` — captive portal mental model in context with rotation and pinning
