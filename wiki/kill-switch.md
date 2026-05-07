An emergency network shutdown for hostile environments. One command brings every interface down, disables every radio, and powers off Bluetooth — all the network surface the system is exposing, off, immediately. One command later puts it back.

This page is the operator-facing doc for `proteus kill` and `proteus resume`. For the threat-model framing of when to reach for it, read `proteus wiki hostile-environments` first.

## When to use it

The kill switch is for the moment you decide the environment is no longer trustworthy and you want zero packets leaving the host, right now. Concretely:

- **Suspected compromise.** A captive portal demanding more than it should, a hotspot you no longer trust, a coworker pointing at your screen and saying "wait, did your network just glitch?". When in doubt, kill first and figure out what happened later.
- **Border crossings.** The minutes before customs is the time to make sure the host is not silently associating with anything. `sudo proteus kill --yes` makes the whole network surface visibly off.
- **Conference floor incidents.** A rogue AP appears, an evil twin gets noticed, a deauth attack starts. Kill the radios so your stack stops re-trying.
- **Walking out of a hostile network.** Cleaner than waiting for the OS to drop the association. The kill switch is symmetric — `proteus resume --yes` brings everything back when you reach a network you trust again.
- **Pre-flight discipline.** Some operators always kill before takeoff so the laptop is not whining at the airport network during the door-close announcements.

The kill switch is deliberately heavy. It is not the right tool for "I want to change MAC" (`proteus rotate`) or "I want to revert Proteus's changes" (`proteus revert`). Reach for those instead unless you genuinely want all traffic stopped.

## What it does

When you run `sudo proteus kill --yes`:

1. Enumerates real network interfaces from `/sys/class/net`, skipping `lo` and virtual interfaces (`docker*`, `podman*`, `veth*`, `virbr*`, `br-*`, `tun*`, `tap*`, `tailscale*`, `wg*`, `zt*`, `kube*`, `cni*`). VPN tunnel devices in particular (`tun*`, `tap*`, `wg*`, `tailscale*`, `zt*`) are skipped on purpose: bringing them down via `ip link` would tear down the userspace-installed routes that the VPN client cannot reliably re-create on the matching `link up`, so resume would turn into a debugging session instead of a one-command restore. The underlying physical interface — the Wi-Fi card or Ethernet port the tunnel rides on — is already brought down, which kills the tunnel's traffic at the same moment without needing to touch the tunnel device itself.
2. Brings each one down via `ip link set <iface> down`. The kernel marks the link administratively down; no frames leave the radio, no packets leave the wire.
3. Toggles `WirelessEnabled = false` and `WwanEnabled = false` on the NetworkManager DBus service. Equivalent to `nmcli radio all off`.
4. Powers off every BlueZ adapter via DBus (`org.bluez.Adapter1.Powered = false`).
5. Records the snapshot under `state.kill_switch` in `/var/lib/proteus/state.json` so `proteus resume` knows what to bring back up.
6. Prints a per-step summary so you can see what was disabled.

The interface set is captured at activation time. Plugging a new device in after the switch is active does not auto-disable it (and Wi-Fi rfkilled at the NM level will keep it from associating regardless). Run `proteus kill --yes` again to re-snapshot if you need to.

## What it does not do

The kill switch is a network-layer hatch. It does not pretend to be more than that.

- **Does not unmount drives.** Encrypted volumes stay mounted; running tasks keep running. If your threat model includes "RAM is hostile", you want a different tool — luks-suspend, `cryptsetup luksSuspend`, or shutting the lid.
- **Does not kill running processes.** A Firefox tab waiting on TCP keeps waiting; the syscall returns when the kernel notices the link is down. SSH sessions, VPN sessions, Slack, Spotify — they all just see network errors and retry. Nothing crashes; nothing exits gracefully either.
- **Does not drop in-flight TLS sessions on the wire.** Anything mid-handshake when the kill switch fires will time out from the peer's perspective rather than tearing down cleanly. Servers see a flow that just stops responding. There is no "I am leaving now" goodbye.
- **Does not encrypt or hide local data.** Anyone with physical access to your unlocked laptop sees the same files they would have seen before. A network kill switch is not a panic button for the disk.
- **Does not affect cellular modems Proteus does not see.** If the host has a WWAN modem managed by NetworkManager, this disables it. If it has a cellular modem managed by something else (a vendor utility, ModemManager outside NM), Proteus does not touch it. `rfkill block all` is the brute hammer if you need the modem off.
- **Does not survive a reboot.** State is recorded in `state.json`, but the kernel itself comes back up with interfaces enabled on next boot. If you rebooted while killed, run `proteus kill --yes` again or `nmcli radio all off`.

The list of "does not" matters: people occasionally mistake a network kill switch for a panic-room button. It is one specific tool — the network-side off switch — and not a substitute for any other defense.

## How to use it

Three subcommands, all gated on `--yes`:

```sh
# Activate. Asks for --yes; needs root.
sudo proteus kill --yes

# Read-only check. Works without root.
proteus kill status
proteus kill status --json

# Restore. Asks for --yes; needs root.
sudo proteus resume --yes
```

The status output (human-readable):

```text
kill switch: ACTIVE
  activated at:           2026-05-06T12:34:56Z
  interfaces:             enp0s3, wlo1
  nm wireless disabled:   true
  nm wwan disabled:       false
  bluetooth disabled:     true

run `sudo proteus resume --yes` to restore.
```

The status output (JSON, schema-stable for wrapper authors):

```json
{
  "active": true,
  "activated_at": "2026-05-06T12:34:56Z",
  "interfaces": ["enp0s3", "wlo1"],
  "nm_wireless_disabled": true,
  "nm_wwan_disabled": false,
  "bluetooth_disabled": true
}
```

When the switch is inactive, `active` is `false` and the per-component flags read `false` too.

### Idempotent

Both commands are idempotent. Running `proteus kill --yes` while the switch is already active prints "kill switch already active" and exits `0`; running `proteus resume --yes` while the switch is not active prints "kill switch not active" and exits `0`. You can wire either one into a key combination or a hot-corner without worrying about double-fire.

### Exit codes

Standard Proteus exit codes apply:

- `0` — success (including the "already active" / "not active" idempotent paths).
- `1` — generic error (e.g. every `ip link set down` failed, no interfaces could be brought down).
- `64` — needs `--yes`. Same convention as `revert`, `reset`, `uninstall`.
- `66` — needs root.

## Recovery

The expected recovery path is `sudo proteus resume --yes`. It reads the recorded snapshot from `state.json` and reverses each step: brings the interfaces back up, re-enables NM radios, powers Bluetooth back on. Idempotent — re-running on an already-resumed system does nothing.

If `proteus resume` fails for any reason (binary broken, DBus broken, you really do not have time), the manual recovery is:

```sh
# Bring interfaces up by hand. Substitute the names from `proteus kill status`.
sudo ip link set wlo1 up
sudo ip link set enp0s3 up

# Re-enable NM radios.
nmcli radio all on

# Re-enable Bluetooth.
bluetoothctl power on

# Clear Proteus's recorded snapshot so subsequent `kill status` reports inactive.
sudo rm -f /var/lib/proteus/state.json   # or hand-edit the kill_switch object out
```

If you used the manual recovery path because `proteus resume` was broken, please file a bug — `proteus resume` is the supported flow and it should work even when other Proteus features have failed.

## Caveats

A few things that bite if you do not know about them ahead of time.

### SSH'd into a remote machine

If you SSH into a remote box and run `sudo proteus kill --yes` there, you will disconnect yourself. The kill switch does not distinguish "the network you are using right now" from "every other network surface" — it cuts everything. Without console access or a serial port, recovery requires somebody with physical access to the machine to log in and run `sudo proteus resume --yes` (or the manual recovery above).

For remote hosts, the safer hatch is `nmcli connection down <iface>` to drop a single connection while leaving SSH on its own interface alive. The kill switch is designed for the laptop in front of you — the one you can also pick up and walk away with.

### Keyboard-shortcut wrappers

A common pattern is to wire `sudo proteus kill --yes` to a keyboard shortcut. Two notes:

- `--yes` is a deliberate gate against fat-fingering. Make sure the shortcut is one you cannot hit by accident; no Ctrl+K bound to "compose new message" in some other tool.
- The shortcut needs to invoke `sudo` somehow (PolicyKit rule, sudoers NOPASSWD entry, or running the desktop session as a user with passwordless sudo for this one binary). Configuring that is a system-administration task; Proteus does not do it for you.

### Bluetooth re-pairing

Powering a Bluetooth adapter off and back on does not clear pairings — the device pairings live in `/var/lib/bluetooth/<address>/`. But some devices do not auto-reconnect across a Powered=false / Powered=true cycle and need a manual `bluetoothctl connect <device-address>` afterwards. Headphones especially are inconsistent here.

### NM-managed vs. unmanaged interfaces

The kill switch brings every interface in `/sys/class/net` (minus the skip list) down via `ip link`, regardless of whether NetworkManager is managing it. If you have an interface that NM is configured to ignore (e.g. `unmanaged-devices` in `NetworkManager.conf`), the kill switch will still bring it down. That is by design — the point of a kill switch is "off the network", not "off NM".

### Probe requests

Bringing the Wi-Fi interface administratively down silences probe requests for that interface. If your saved-networks list is the leak you are worried about (cross-ref `proteus wiki hostile-environments` § "Saved networks and probe requests"), the kill switch handles that too.

## Cross-refs

- `proteus wiki hostile-environments` — the threat-model framing for when this command is the right one to reach for.
- `proteus wiki threat-model` — what the network-layer floor does and does not cover. Read this if you are not sure whether the kill switch is the right tool for your situation.
- `proteus wiki recipes` — common operational scenarios; the kill switch shows up in the "leaving a hostile network" recipe.
- `proteus wiki cli` — full command reference, exit codes, JSON schemas.
- `proteus wiki troubleshooting` — symptom-based recovery, including "I ran kill and now nothing works" cases.
- `proteus wiki uninstall` — `proteus uninstall` does not run resume for you. If you uninstall while the kill switch is active, run `resume` first or use the manual recovery above.
