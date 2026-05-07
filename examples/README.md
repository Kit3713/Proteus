# Proteus example configs

These are starting points, not the only valid configs. Read `proteus wiki
config` for the full schema and the risk table for every knob. Copy the
closest preset to `/etc/proteus/config.toml`, then tweak. After editing,
validate with `proteus show-config --json` (or `proteus show-config` for
human output) so you catch typos before `proteus apply`.

Every preset has the same header block:

- filename
- one-line purpose
- install command
- pointer at `proteus wiki config`

Every section in every preset has a one-line comment explaining what it
does. Every non-default value has a `# WHY:` (rationale) or
`# WARNING:` (what it might break) comment.

## Index

- [`minimal.toml`](minimal.toml) — only MAC rotation; everything else stays at OS defaults.
- [`standard.toml`](standard.toml) — balanced privacy + compatibility. Recommended starting point.
- [`aggressive.toml`](aggressive.toml) — stronger privacy at the cost of some breakage (KDE Connect, WSD printers, corp Wi-Fi possibly).
- [`captive-portal-heavy.toml`](captive-portal-heavy.toml) — tuned for café / conference / hotel / airport routines.
- [`paranoid.toml`](paranoid.toml) — maximum privacy, accept significant breakage. Read the warning header before using.
- [`disabled.toml`](disabled.toml) — every section off. Equivalent to not running `proteus apply`. For people who run another tool stack and just want the read-only commands.
- [`development.toml`](development.toml) — for Proteus contributors. Fast cycles, every feature on, verbose. Not a real-world preset.

## Choosing a preset

If you are not sure, start with `standard.toml`. It enables the rotations
and silences the discovery chatter that almost everyone wants while
leaving the breakage-prone knobs (SSDP, WSD, anonymous outer identity,
TX power reduction) alone. From there:

- If you live on hotel / airport / café Wi-Fi, layer `captive-portal-heavy.toml`'s
  changes on top.
- If you are willing to lose KDE Connect and WSD-only printer discovery
  for a stronger silence profile, move to `aggressive.toml`.
- If you have your own privacy stack and only want `proteus status` to
  read your interfaces, use `disabled.toml`.

## Install

```sh
sudo cp examples/standard.toml /etc/proteus/config.toml
sudo proteus apply
```

Substitute the preset filename you picked. Apply lands in phase B+; on
phase A builds you can still validate with `proteus show-config --json
--config examples/standard.toml`.
