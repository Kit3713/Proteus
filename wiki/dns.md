Read this first: Proteus is not a DNS-privacy tool. If you want encrypted DNS, ad-blocking, anti-tracking, or resolver choice, stop here and reach for a real DNS tool. Proteus owns one knob and refuses to fight anything else for control of your resolver.

## What Proteus does

One knob: strip EDNS Client Subnet (ECS) on systemd-resolved.

ECS is a DNS extension where the resolver tells the upstream DNS server your /24 (IPv4) or /48 (IPv6) prefix. Authoritative servers use this to give location-tailored CDN responses. The side effect: every DNS query you make carries your approximate location to whoever sees it. Proteus tells systemd-resolved to strip this from outgoing queries.

That is the entire DNS feature. There is no second knob coming. There is no resolver picker. There is no DoH client embedded in Proteus. By design.

## What Proteus does NOT do

- Choose a DNS resolver — that is your job. Use NetworkManager connection settings or `/etc/resolv.conf`.
- Encrypt DNS — use `dnscrypt-proxy`, the NextDNS client, AdGuard Home, or knot-resolver.
- Block trackers via DNS — use Pi-hole, NextDNS, or AdGuard Home.
- DNS-over-HTTPS or DNS-over-TLS — beyond systemd-resolved's built-in DoT support, which you configure yourself. Proteus does not configure your resolver.
- Local DNS caching — beyond what systemd-resolved already does.

This is intentional. DNS is its own complex world; tools dedicated to DNS do it better than a generalist could.

## The hard guard

The ECS-strip knob refuses to apply if Proteus detects another DNS-privacy tool managing systemd-resolved or your resolver path. If any of these are present, Proteus exits clean and surfaces the detection in `proteus status`:

- `dnscrypt-proxy` running — binary at `/usr/bin/dnscrypt-proxy` or `/usr/local/bin/dnscrypt-proxy`, OR systemd unit `dnscrypt-proxy.service` is active.
- `pi-hole` — FTL daemon active.
- `AdGuardHome` binary or service active.
- Custom `/etc/resolv.conf` — a real file, not a symlink to systemd-resolved's stub at `/run/systemd/resolve/stub-resolv.conf`.
- ANY non-Proteus drop-in under `/etc/systemd/resolved.conf.d/*.conf`.
- `knot-resolver`, `unbound`, `bind`, or any other resolver listening on `127.0.0.1:53` or `::1:53`.

The user's DNS setup wins, every time. If you have set up DNS deliberately, Proteus will not undo your work to install one knob.

## How it does it

Writes a single drop-in:

```
/etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf
```

```
# managed by proteus
# sha256:<expected-content-hash>
[Resolve]
DNSOverTLS=no  # do not change; this just preserves user setting
DNSSEC=allow-downgrade  # do not change
EDNSClientSubnet=no  # the only line we own
```

Then `systemctl restart systemd-resolved`. Everything else in `resolved.conf` is left alone. The two preserved lines exist so the drop-in does not silently flip user-visible behavior; only `EDNSClientSubnet=no` is the change Proteus is making.

## Configuration

```toml
[dns]
strip_edns_client_subnet = true  # default true; the only knob
```

Set it to `false` to disable, then run `proteus apply`. There is no per-interface override and there will not be one — the systemd-resolved drop-in is global by nature.

## Detection priority

The detect-and-defer pattern (cross-ref `proteus wiki concepts`):

1. Check for non-Proteus drop-ins under `/etc/systemd/resolved.conf.d/`.
2. Check for processes/services named in the deny-list above.
3. Check `/etc/resolv.conf` for non-stub configuration.
4. If clean, apply the drop-in.
5. If anything is detected, name the tool in `proteus status` and exit clean.

Detect-and-defer is one of two places in Proteus where this pattern runs (the other is NTP). The rule is the same: the more-specialized tool wins, and the decision is surfaced so you know exactly what was skipped and why.

## Verification

- `proteus status --json | jq .dns` — shows what was applied or what we deferred to.
- `dig +short txt o-o.myaddr.l.google.com` — Google's ECS reflector. Should show empty/none after Proteus applies (was previously your /24 prefix). The simplest before/after check.
- `journalctl -u systemd-resolved` — look for the `EDNSClientSubnet=no` config line on restart.
- `cat /etc/systemd/resolved.conf.d/10-proteus-no-ecs.conf` — inspect the drop-in.

If `dig` still reports a subnet after `proteus apply`, check `proteus status` first — Proteus probably deferred and is telling you why.

## Reverting

- `proteus revert` (phase G) — removes the drop-in, restarts systemd-resolved.
- `proteus reset` — same; clears config back to defaults.

The drop-in is the only artifact. Removing it and restarting systemd-resolved returns ECS behavior to the systemd default. Nothing else needs cleaning up.

## Why ECS in particular

Of all the DNS leaks, ECS is unique: it is a knob that systemd-resolved exposes cleanly, it does not break anything when disabled (servers fall back to non-ECS responses), and it meaningfully reduces location leakage to upstream resolver operators and authoritative servers. Lower-hanging fruit than a full DoH/DoT setup, no tradeoffs against the rest of your DNS stack. Everything else DNS-shaped belongs in a real DNS tool. Compose Proteus on top.

## Cross-refs

- `proteus wiki concepts` — detect-and-defer pattern, managed files, idempotency.
- `proteus wiki threat-model` — what this tool is and is not for. Read before trusting Proteus with anything that matters.
- For real DNS privacy:
  - `dnscrypt-proxy` — https://github.com/DNSCrypt/dnscrypt-proxy
  - Pi-hole — https://pi-hole.net/
  - AdGuard Home — https://adguard.com/en/adguard-home/overview.html
