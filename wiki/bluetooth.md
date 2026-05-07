Bluetooth on Linux splits cleanly into two halves. Adapter alias, discoverability, and BLE Resolvable Private Address mode are the three knobs a tool can touch without chipset-specific code; Proteus handles all three. Anything that requires vendor HCI opcodes — including BR/EDR (classic) `BD_ADDR` rotation — is deferred until there is a known-good chipset matrix.

For the mental model behind identifiers and rotation, read `proteus wiki concepts` first.

## What Proteus touches

**Adapter alias.** BlueZ exposes a per-adapter `Alias` property over DBus — the name your phone or laptop shows during pairing and discovery. By default this is the system hostname, which leaks the hostname Proteus is otherwise scrubbing. Proteus replaces the alias with a generic string (`BT Device` by default) so the discoverable name is no longer correlated with the host.

**Discoverable=off by default.** A discoverable adapter answers inquiry scans from any device in range. Most users want pairing on, discoverability off — pair from the laptop side, then leave the adapter quiet. Proteus sets `Discoverable=false` on apply. If you want to pair a new device, flip it on temporarily with `bluetoothctl discoverable on`, finish pairing, and Proteus's next apply will turn it back off.

**BLE Resolvable Private Address (RPA).** Where the controller supports privacy mode (most modern chipsets do), Proteus enables RPA. The controller rotates the on-air BLE address on a schedule — typically every 15 minutes, controlled by the controller, not by Proteus. A passive observer sees a different random-looking address each interval. Devices that hold your Identity Resolving Key (IRK) — the ones you've paired with — can resolve the rotating address back to you. Devices that don't can't. That's the entire point.

## What Proteus does NOT touch

**BR/EDR (classic) `BD_ADDR` rotation.** Each Bluetooth chipset vendor accepts different vendor-specific HCI opcodes for setting the public address. CSR uses one set, Broadcom another, Intel another, Realtek another, Mediatek another. Some accept the change at runtime; some only accept it before firmware load; some require a specific power sequence; some brick the controller if you get it wrong. The reference for how messy this is lives in the BlueZ source tree at `tools/bdaddr.c` — every supported chipset has its own branch.

Until there is a known-good chipset matrix and a way to detect the controller's exact firmware revision, Proteus does not attempt classic BD_ADDR rotation. The bricking risk is real and the failure mode is "your Bluetooth adapter no longer exists until you reflash it".

If you need classic BD_ADDR rotation today, `bdaddr` from `bluez-tools` is the manual route — it carries the vendor-specific code and the warnings to match. Proteus will revisit this when the chipset matrix stabilizes.

**Random LE address in modes the controller doesn't expose.** Some controllers support privacy mode but only via vendor extensions, not the standard HCI privacy commands. Proteus only enables RPA when the controller exposes it through the standard kernel and BlueZ interfaces. If your controller has the capability behind a vendor blob, Proteus skips with a `skipped (no controller privacy support)` line in status rather than reaching for vendor code.

**IRK rotation.** The Identity Resolving Key is what paired devices use to reverse your RPA back to a stable identity. Rotating the IRK would break every existing pairing. Proteus does not rotate IRKs. See the limits section below.

## How to use

Read commands first, before changing anything. Per-feature status is one of `applied / skipped (reason) / failed (reason)` — never a silent skip. See `proteus wiki concepts` for the rule.

```
proteus status
```

Shows the Bluetooth adapter's current alias, discoverable state, RPA support, and whether RPA is enabled. If BlueZ isn't present, the section reads `bluetooth: skipped (no BlueZ)` and Proteus moves on cleanly.

```
sudo proteus apply
```

Writes the configured alias (default `BT Device`), sets `Discoverable=false`, and enables RPA where the controller supports it. Idempotent — running it ten times converges to the same state as running it once.

```
sudo proteus revert
```

(Phase G.) Restores the original alias and discoverable setting from the cache that Proteus snapshotted on first run. Like every other identifier, the original Bluetooth alias is captured once and never re-captured.

## Detection logic

Proteus detects BlueZ by checking for the `org.bluez` service on the system DBus. If the service is missing, every Bluetooth feature skips cleanly with a `skipped (no BlueZ)` line in `proteus status` and no error. Headless servers and BlueZ-less installs are first-class citizens — Proteus just steps out of the way.

When BlueZ is present, Proteus talks to it directly over the BlueZ DBus API. No shelling out to `bluetoothctl`. The `bluetoothctl` CLI is itself a thin DBus wrapper; Proteus does the same calls without spawning a subprocess. This is the same pattern Proteus uses for NetworkManager — DBus directly via zbus, never shell-out.

Per-adapter detection: Proteus enumerates `org.bluez.Adapter1` objects and operates on each. Most laptops have one adapter; multi-adapter setups (USB Bluetooth dongle plus internal) are handled by applying the same policy to every adapter unless you scope the config.

## RPA behavior

When privacy mode is enabled, the controller generates a Resolvable Private Address by encrypting your IRK with a random number. The result is a 48-bit address with a specific bit pattern that marks it as resolvable.

The controller rotates the RPA on a schedule. The interval is controller-controlled — typical defaults are 15 minutes, but you'll see anywhere from 1 minute to several hours depending on the chipset and BlueZ version. Proteus does not override this; the controller's default is fine for tracker-defeat.

A passive observer (anything sniffing BLE without your IRK) sees a different random-looking address each interval and cannot link them. A paired device that holds your IRK runs a constant-time check against incoming addresses and resolves them back to your stable identity. Pairings keep working transparently.

This is exactly how iPhones, AirPods, and modern fitness trackers have worked for years. Proteus opts your laptop into the same pattern.

## Limits

**Paired devices need your IRK.** That's how pairing works. If you rotate IRKs, every existing pairing breaks and you have to re-pair from both sides. Proteus does not rotate IRKs because the breakage cost is high and the gain is narrow — most BLE tracking is from passive observers who never had your IRK in the first place.

**Active scanners with a target list still see you when you initiate a connection.** RPA defeats passive tracking. An active scanner that you connect to (a hostile beacon you tap, a malicious peripheral you pair with) sees the address you used for that connection. Don't pair with hardware you don't trust.

**Classic Bluetooth is not protected.** BR/EDR (the older, non-LE protocol used by audio sinks like most Bluetooth speakers and some headsets) uses your fixed `BD_ADDR`. Until the chipset matrix lands, this address does not rotate. If your threat model includes someone in physical proximity logging classic Bluetooth addresses, Proteus does not help here yet. Use BLE-only audio (most modern AirPods, Sony, Bose) or turn the adapter off when you don't need it.

**Bonded devices still see your identity address during pairing.** RPA hides you from new observers, not from devices you're actively bonding with. The bonding handshake exchanges the IRK so the bonded device can resolve future RPAs. This is the design.

## Common questions

**Will this break my AirPods / Bose / Sony headphones / fitness tracker?**

No. Paired devices share your IRK and continue resolving your rotating BLE address transparently. The user-visible behavior is identical. RPA is what your iPhone has been doing for years.

If a paired device does break after `proteus apply`, that's a bug — file it with the chipset model and BlueZ version. The most likely cause is a controller that misreports privacy support.

**Can I disable Bluetooth entirely?**

Use the existing tools. Proteus is for "Bluetooth on, fingerprint reduced", not "Bluetooth off". For full off:

```
bluetoothctl power off
```

Or block at the rfkill layer for a hard off across reboots:

```
rfkill block bluetooth
```

If you never use Bluetooth, mask the service:

```
sudo systemctl mask bluetooth.service
```

Proteus respects all of the above. If the adapter is powered off or the service is masked, the Bluetooth section in `proteus status` reads `skipped (adapter off)` or `skipped (no BlueZ)`.

**What about iBeacon, Eddystone, and other BLE tracker beacons?**

Out of scope. iBeacon and Eddystone are application-layer protocols carried over BLE advertisements; the privacy concern is an app on your phone broadcasting your location to a beacon network, not the BLE radio itself. Proteus operates on the radio layer. For application-layer beacon defenses, the answer is "don't install the apps that scan for them", which is a phone-OS-level concern, not a Linux-network-fingerprint concern.

For the broader "what Proteus is not" picture, see `proteus wiki threat-model` (planned for phase F).

## Where to go next

- `proteus wiki concepts` — the mental model: identifiers, rotation, managed files, detect-and-defer.
- `proteus wiki mac-recipes` — Wi-Fi and Ethernet MAC patterns. Same phase as this page.
- `proteus wiki threat-model` — what Proteus does not do and which tool to reach for instead. Phase F.
- `proteus help bluetooth` — full CLI reference for the Bluetooth subcommands.
