Per-SSID profile policies are how Proteus carves out exceptions to the global config on a network-by-network basis. The same laptop that joins a trusted home LAN, a coffee shop, and a conference Wi-Fi over the course of a day rarely wants the same posture for all three. A per-SSID block lets the operator say "on *this* SSID, behave differently" without flipping the global profile or persona every time the laptop moves.

This page covers the schema, the precedence rule, the five fields, the migration story for known-portal SSIDs, and a worked coffee-shop example. For the global slider, see `proteus wiki profiles`. For personas, see `proteus wiki personas`.

## What a per-SSID block looks like

In `/etc/proteus/config.toml`:

```toml
profile = "med"

[per_ssid."coffee-shop"]
persona = "iphone-15"
portal_policy = "fresh-mac-per-visit"

[per_ssid."home-lan"]
pin_mac = "aa:bb:cc:dd:ee:ff"

[per_ssid."conference-wifi"]
aggressiveness_profile = "agr"
rotate_interval = "30m"
```

The key in `[per_ssid."<ssid>"]` is the literal SSID string the access point advertises. It is **case-sensitive** — `"Coffee-Shop"` and `"coffee-shop"` are two different keys.

You manage the blocks through `proteus ssid` rather than hand-editing the file. The CLI has four verbs:

- `proteus ssid list` — every per-SSID entry, one block per line group.
- `proteus ssid show <ssid>` — the resolved policy for one SSID, including the source trace.
- `proteus ssid set <ssid> <key> <value> --yes` — set one field on a block.
- `proteus ssid clear <ssid> --yes` — drop the entire block.

Pass `--json` to `list` and `show` for machine-readable output.

## Precedence chain

When the orchestrator joins an SSID, it walks four layers in decreasing precedence and stops at the first answer for each field:

1. `[per_ssid."<ssid>"]` — the SSID-specific block (highest)
2. `[persona]` — the active persona's defaults
3. `[profile]` baseline — the slider's documented behaviour
4. config defaults — the structural fallback (lowest)

The resolved view comes from `proteus ssid show <ssid>`:

```
ssid:                   coffee-shop
  profile:              med
  persona:              iphone-15
  pin_mac:              (unset)
  rotate_interval:      (global)
  portal_policy:        fresh-mac-per-visit
  source (per_ssid > persona > profile > defaults):
    - per_ssid
    - profile
    - defaults
```

The `source` trace lists every layer that contributed at least one field. Layers a per-SSID override fully covered drop out — in the example above, the per-SSID block supplied `persona` so the persona layer is not in the trace.

## The five fields

Every field is optional. Leave a field unset and the chain falls through to the next layer.

`persona` — persona id to use on this SSID (e.g. `"iphone-15"`, `"galaxy-s24"`, `"randomizer-high"`). Beats the global `[persona] active`. Run `proteus persona list` for the catalogue.

`aggressiveness_profile` — `Profile` slider override. One of `off`, `min`, `low`, `med`, `high`, `agr`. Lifts (or lowers) the global slider for this SSID only. Useful for naming a single hostile SSID `agr` without flipping the rest of your day to that posture.

`pin_mac` — pin a literal MAC address for this SSID (e.g. `"aa:bb:cc:dd:ee:ff"`). The orchestrator pre-empts MAC rotation while a pin is in scope, so the lease stays stable across rejoins. Common shape for a home network where the router has a port-forwarding rule keyed on MAC.

`rotate_interval` — duration override (e.g. `"30m"`, `"4h"`, `"1d"`). Same syntax as `[timers.rotate].interval` and `proteus timer set --interval`. Lets one SSID rotate faster (conference) or slower (home) than the global cadence.

`portal_policy` — captive-portal-style policy override. Currently the only supported value is `"fresh-mac-per-visit"` — every visit to this SSID gets a fresh MAC even if the SSID is not in the legacy `known_portal_ssids` array. Pass-through string; the orchestrator interprets it.

## Migration: known portal SSIDs become per-SSID entries

Releases before Milestone 3 tracked captive-portal SSIDs in `state.known_portal_ssids` (a flat array under `/var/lib/proteus/state.json`). The new per-SSID story subsumes that list: the v1 → v2 state migration mirrors every entry in `known_portal_ssids` into `state.per_ssid_seed[<ssid>].portal_policy = "fresh-mac-per-visit"` so the orchestrator can pick up SSID-scoped policy without consulting the legacy field.

The migration is **idempotent** — running it twice converges. The legacy `known_portal_ssids` array is **kept** for one cycle so older `proteus portal mark / unmark / list` paths keep working; the deprecation is a follow-up.

If you have a v1 state file like:

```json
{ "schema_version": 1, "known_portal_ssids": ["foo", "bar"] }
```

it loads on the next `proteus` invocation as v2 with `state.per_ssid_seed["foo"].portal_policy = "fresh-mac-per-visit"` and the same for `bar`, while the legacy array survives. To move the entries up to the authoritative `[per_ssid]` shape in config, run `proteus ssid set <ssid> portal_policy fresh-mac-per-visit --yes` for each one.

## Worked example: coffee shop with a phone-persona pin

You're a regular at a coffee shop whose Wi-Fi is `coffee-shop` (with the lowercase, hyphenated SSID). It has a captive portal. You want to look like an iPhone 15 every time, and you want a fresh MAC on every visit so the portal can't tie sessions across days. Two `set` commands wire it up:

```sh
sudo proteus ssid set coffee-shop persona iphone-15 --yes
sudo proteus ssid set coffee-shop portal_policy fresh-mac-per-visit --yes
```

Confirm with the resolved view:

```sh
proteus ssid show coffee-shop
```

```
ssid:                   coffee-shop
  profile:              med
  persona:              iphone-15
  pin_mac:              (unset)
  rotate_interval:      (global)
  portal_policy:        fresh-mac-per-visit
  source (per_ssid > persona > profile > defaults):
    - per_ssid
    - profile
    - defaults
```

The orchestrator sees this on the next NM `connection-up` for `coffee-shop` and shapes MAC, hostname, and DHCP options to look like an iPhone 15 — and rotates MAC fresh on each rejoin. Other SSIDs are unaffected.

To roll the change back, drop the block:

```sh
sudo proteus ssid clear coffee-shop --yes
```

## What's not yet wired

This release ships the schema, the resolver, the CLI surface, and the v1 → v2 state migration. The integration with the NM connection-up dispatcher — actually applying the resolved policy on every join — is the follow-up tracked in roadmap Milestone 3. Until that lands, `proteus ssid set / show / list / clear` operates on the config and state files; the orchestrator does not yet consult them on join. The schema is stable, so blocks you write now will take effect on the next release.
