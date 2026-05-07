# Proteus NetworkManager dispatcher

Event-driven rotation hook that fires whenever NetworkManager reports a
connection state change. Pairs with `proteus-resume.service` (sleep hook)
to give Proteus near-immediate reaction to disconnects, reconnects, VPN
events, captive-portal connectivity changes, and resume-from-suspend.

There is no daemon. The dispatcher is a short-lived bash script invoked
by NM; the resume hook is a oneshot systemd service. Both honor the
"no daemon" invariant from `docs/PLAN.md`.

## Install

Drop `dispatcher.d/01-proteus` into `/etc/NetworkManager/dispatcher.d/`
(or `/usr/lib/NetworkManager/dispatcher.d/` for a packaged install). Owner
root, mode `0755`. NM will run it on every connection state change.

```
sudo install -m 0755 -o root -g root \
    dist/networkmanager/dispatcher.d/01-proteus \
    /etc/NetworkManager/dispatcher.d/01-proteus
```

`install.sh` does this automatically.

## What it does

- `up` — interface came up: invoke `proteus rotate --iface <name> --yes`,
  unless the configured cooldown (default 60s) is still in effect.
- `connectivity-change` — log only. Don't rotate behind a portal.
- `down`, `pre-up`, `pre-down`, `vpn-*`, `dhcp*`, `hostname` — log only.

All events log a single line via `logger -t proteus-dispatcher`, visible
in the journal:

```
journalctl -t proteus-dispatcher
```

## Cross-references

- `proteus wiki rotation` — the architecture rationale, including the
  "Event-driven triggers" section that describes how the dispatcher and
  the resume hook work together with `proteus-check.timer`.
- `dist/systemd/README.md` — the polling timers and boot oneshot.
- `docs/PLAN.md` — the "no daemon" invariant.
