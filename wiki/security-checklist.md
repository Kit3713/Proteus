A cookbook of routines for users who care about minimizing every locally controllable fingerprint — L2 through L4 network identifiers, network-joining protocol chatter, and the OS-controllable RF surface. Daily, weekly, monthly, pre-trip, post-trip, annual. Copy-paste each block into a terminal and read what comes back.

This page is operational only. The "why" lives in `proteus wiki threat-model`. The "what to do when it breaks" lives in `proteus wiki troubleshooting`. The "what each command does" lives in `proteus wiki cli`.

Proteus runs idle most of the time. These checklists exist so you can confirm that "idle" actually means "working".

## Daily checklist (every morning)

A 30-second routine. Run all four; read the output for anything off.

```sh
proteus doctor                  # are all components healthy?
proteus status                  # is Proteus actively managing things?
proteus original                # is the cached original MAC still here?
proteus current                 # what MAC am I on right now?
```

What to look for:

- `proteus doctor` — `Summary: ... 0 fail`. Any `fail` is hard breakage; `warn` and `skip` are fine. If anything failed, jump to `proteus wiki troubleshooting`.
- `proteus status` — every feature you enabled should read `applied` (not `failed` or `skipped (unexpected reason)`). A `skipped (detected dnscrypt-proxy)` on the DNS feature is fine — that's the detect-and-defer guard doing its job.
- `proteus original` — `captured_at` should be a real timestamp, `original_macs` should be non-empty. If this is suddenly empty, your state file got wiped and you've lost your revert anchor — stop and investigate before running any mutating command.
- `proteus current` — the MAC should be a locally-administered address (second hex digit `2`, `6`, `a`, or `e`), not your cached original.

When to re-run something:

- A `fail` in `doctor` means re-run after the remediation the check suggests.
- An unexpected `skipped` in `status` means read the reason; if it surprises you, file a bug.

## Weekly checklist

Once a week, on a quiet morning.

- Review drift: `proteus diff` (lands phase G). Flags any managed file whose SHA no longer matches the `# expected-sha256:` header. The header is an edit-detection / tamper hint, not an integrity guarantee — anyone with write access can recompute it — so treat drift as a "something edited this file" signal: decide whether to keep the edit or re-apply Proteus's version.
- Skim the log: `journalctl -t proteus -n 200 --no-pager`. Look for repeated errors, DBus glitches, unexpected rotations.
- Verify timers: `proteus timer status`. The `rotate` timer should be active with a sane next-fire time; the `check` timer too.
- Confirm probes still work: `proteus probe`. Should classify `clear` on a normal connection. If it returns `inconclusive` or `down` from your home network, your probe endpoints may need updating — see `proteus wiki probes`.
- Audit your config: `proteus config show`. Re-read it; does it still match what you actually want? Knobs accumulate.

```sh
proteus diff                                    # phase G
journalctl -t proteus -n 200 --no-pager
proteus timer status
proteus probe
proteus config show
```

## Monthly checklist

Once a month, with coffee and ten minutes.

- Re-read `proteus wiki threat-model`. Has the threat landscape shifted? New tracking technique you should know about? New tool to add to your stack?
- Update Proteus itself: `cargo install --git https://github.com/Kit3713/Proteus.git --locked`. Or wait for distro packages once they land.
- Re-verify the rest of your stack: dnscrypt-proxy still resolving; Tor Browser still launches; your VPN still authenticates; your Pi-hole or NextDNS subscription is current.
- Test revert on a safe config: `sudo proteus revert --yes` (phase G) on a non-production config to confirm rollback still works cleanly. Then re-apply.

```sh
proteus wiki threat-model | less
cargo install --git https://github.com/Kit3713/Proteus.git --locked
sudo proteus revert --yes                       # phase G; test on a throwaway config
sudo proteus apply --yes                        # then re-apply
```

## Pre-trip checklist (before traveling to a less-trusted environment)

Hotel Wi-Fi, conference Wi-Fi, airport Wi-Fi, coffee shop, anywhere you don't control. Run before you leave.

```sh
# 1. Verify health
proteus doctor

# 2. Verify your originals are cached (sacred)
proteus original --json | jq .

# 3. Tighten rotation cadence
sudo proteus config set mac.rotation_interval 30m --yes
sudo proteus timer set rotate --interval 30m

# 4. Make sure Bluetooth is locked down (phase B)
sudo proteus bluetooth apply --yes

# 5. Make sure DHCP suppression is on (phase D)
sudo proteus config enable dhcp --yes
sudo proteus dhcp apply --yes

# 6. Make sure DNS hard guard is OK (phase D)
proteus dns status

# 7. Browse outside Proteus's scope: launch Tor Browser or your VPN
# (Proteus does NOT touch browser, VPN, or DNS resolution policy)
```

Cross-ref `proteus wiki hostile-environments` (planned) for per-environment playbooks: hotel, conference, coffee shop, airport, transit hub.

## Post-trip checklist (after returning from a less-trusted environment)

When you're back on a trusted network. Drop the trip's session state, re-apply your home config, check nothing got corrupted.

```sh
# 1. Drop session state — fresh MACs, undo any per-trip pins
sudo proteus revert --yes                       # phase G

# 2. Re-apply your home config
sudo proteus apply --yes

# 3. Confirm originals are intact
proteus original --json | jq .

# 4. Browser cleanup (in your browser): clear cookies, site data, and cache
#    for any sites you visited. Proteus does not touch browser state.
```

If you tightened rotation cadence in the pre-trip checklist, reset it now:

```sh
sudo proteus config set mac.rotation_interval 2h --yes
sudo proteus timer reset rotate
```

## Annual checklist (or when threat model changes)

Once a year, or whenever your situation changes meaningfully (new job, new country, new device, new co-traveler).

- Re-read `proteus wiki threat-model` end-to-end.
- Re-evaluate your browser, VPN, and DNS choices. Has your VPN been bought? Has your DNS provider changed policy? Is your browser still maintained?
- Verify your dnscrypt-proxy / Pi-hole / NextDNS / AdGuard subscription is current and the upstream resolver still reflects your privacy preferences.
- Re-verify Tor Browser is updated — `Help` -> `About Tor Browser`. Old Tor Browser is dangerous Tor Browser.
- Re-verify the system firmware and OS are current — `sudo dnf upgrade --refresh` on Fedora.

## Triggers — when to run something

Event-driven, not scheduled. Run these only when the trigger fires.

- **Joined an unknown network**: `sudo proteus rotate --yes` (phase B) if Proteus didn't auto-rotate at join. Check `proteus current` to confirm.
- **Captive portal in path**: do not rotate manually — let Proteus's captive-portal logic handle it. Cross-ref `proteus wiki captive-portals`.
- **Battery low / on the move**: nothing. Proteus is event-driven; the NM dispatcher and the systemd timers handle it. There is no battery-related action.
- **Daemon misbehaving**: `proteus doctor`, then `journalctl -u proteus-rotate -n 50` for the rotate timer or `journalctl -u proteus-check -n 50` for the probe timer. Cross-ref `proteus wiki troubleshooting`.
- **Want to verify nothing was missed**: `proteus diff` (phase G) to see config-vs-defaults-vs-live drift; `proteus status --json | jq .features` to see per-feature state.
- **About to give a public talk or demo**: `proteus current --json` to confirm what MAC will appear in any screenshots; consider rotating beforehand if the MAC will be visible.

## Out-of-scope reminders (don't expect Proteus to do these)

These are real fingerprinting surfaces, but they belong to other tools. Proteus will not address them and never will.

- **Browser fingerprint** (Canvas, WebGL, fonts, JS quirks) — use Tor Browser, Mullvad Browser, LibreWolf, or Brave with farbling on.
- **DNS encryption** (DoT, DoH, DNSCrypt) — use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole.
- **Tracker blocking** (ads, analytics beacons, telemetry) — use Pi-hole or AdGuard Home network-wide; uBlock Origin in the browser.
- **Traffic correlation** (timing, packet sizes, flow analysis) — use Tor for low-volume; Mullvad or another reputable VPN for higher-volume.
- **Application logins** — separate accounts per identity. No network tool can paper over reusing the same Google account across two MACs you wanted to be uncorrelated.
- **TLS ClientHello** (JA3, JA4) — use a browser or HTTP client whose ClientHello you trust. Cannot be normalized from outside the application.
- **SSH client fingerprint** (HASSH) — edit your `~/.ssh/config` with explicit `KexAlgorithms`, `Ciphers`, `MACs`, `HostKeyAlgorithms`. Proteus refuses to touch SSH config.
- **`/etc/machine-id`** — Proteus refuses to rotate it; rotation breaks too much. If your threat model requires a fresh machine-id, you want a fresh install.
- **Hardware-level RF fingerprint** (analog quirks, IQ imbalance, clock skew) — swap the radio (USB Wi-Fi adapter). Proteus only offers opt-in TX power reduction as a small defense-in-depth.

Cross-ref `proteus wiki threat-model` for the full discussion of each.

## Cross-refs

- `proteus wiki threat-model` — what to defend against, what's out of scope, why.
- `proteus wiki hostile-environments` — per-environment playbooks (planned).
- `proteus wiki getting-started` — first-time setup (see `proteus wiki quickstart` until this lands).
- `proteus wiki troubleshooting` — symptom-based recovery recipes.
- `proteus wiki cli` — full command reference, exit codes, JSON schemas.
- `proteus wiki doctor` — per-check meaning of every `doctor` line.
- `proteus wiki captive-portals` — portal detection, classification, and the no-rotation-loop rule.
- `proteus wiki rotation` — schedule and probe-driven rotation in detail.
- `proteus wiki dns` — the one DNS knob and its detect-and-defer guard.
