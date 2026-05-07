`wpa_supplicant` is the userspace daemon that drives Wi-Fi association on every Linux distro that uses NetworkManager, IWD-fallback, or systemd-networkd. This page covers the privacy knobs worth flipping, what NetworkManager exposes on top of them, and the security-critical settings Proteus refuses to touch.

For the mental model of what counts as a network identifier, read `proteus wiki concepts` first. For the MAC rotation interaction, `proteus wiki mac-recipes`. For 802.1X enterprise networks, `proteus wiki enterprise-wifi`.

## What wpa_supplicant does

A short summary so the rest of the page makes sense.

- **Association.** Picks a BSS, sends Authentication and Association frames, runs the four-way handshake. Before the handshake, your supplicant emits probe requests carrying the SSIDs it knows and (depending on driver behavior) a source MAC.
- **EAP exchange.** For 802.1X (enterprise) networks, runs the chosen EAP method: PEAP, TTLS, TLS, FAST. The outer identity goes in cleartext; the inner identity rides inside the TLS tunnel. Cross-ref `proteus wiki enterprise-wifi`.
- **Key derivation.** Holds the PMK, derives the PTK and GTK from the four-way handshake, manages rekeying. None of this is privacy-relevant per se but the timing of rekeys can be observed.
- **Roaming.** When you walk between APs of the same ESS, wpa_supplicant decides when to roam and emits a fresh Association on the new BSSID. Each transition is a trace of your physical movement on the air.

The privacy-relevant parts are the source MAC of probe requests, the source MAC of the pre-association frames, and what gets logged on disk. Proteus operates one layer above (via NetworkManager) but the supplicant config is where the rotation actually lands.

## Settings that matter for privacy

`wpa_supplicant.conf` (the global config file, typically `/etc/wpa_supplicant/wpa_supplicant.conf` or `wpa_supplicant-<iface>.conf`) supports three top-level keys that govern the on-air MAC.

- **`mac_addr=2`.** Per-scan random MAC. The supplicant generates a fresh locally-administered address for each scan request. This is the strongest setting — every probe burst has a different source MAC.
- **`preassoc_mac_addr=1`.** Random MAC before association. Distinct from `mac_addr` because pre-association traffic (Authentication frames before the four-way handshake) is its own surface.
- **`rand_addr_lifetime=60`.** How long a random MAC stays in use before being replaced, in seconds. Default is 600 (10 minutes); 60s is more aggressive and matches a busy mobile profile. Set to `0` to regenerate on every scan.

Possible values for `mac_addr`:

- `0` — use the permanent MAC. The default on most distros until you flip it.
- `1` — generate a single random MAC at boot and keep it for the session.
- `2` — generate a fresh random MAC for every scan.
- `3` — use the OUI part of the permanent MAC, randomize the lower 24 bits.

`mac_addr=2` is what you want for a moving laptop on public Wi-Fi. `mac_addr=3` is a compromise that preserves the vendor signal of the permanent OUI while randomizing the device-specific bits — useful if a network somehow keys on the OUI.

Example global config snippet:

```text
ap_scan=1
mac_addr=2
preassoc_mac_addr=1
rand_addr_lifetime=60
```

## Per-network override

The same keys can live inside a `network={ ... }` block to override the global default for one SSID. This matters when one network needs a stable MAC (DHCP reservation, captive portal MAC binding, eduroam lockout policy) while every other network gets the rotating treatment.

```text
network={
    ssid="HomeNetwork"
    psk="..."
    mac_addr=0
    mac_value=aa:bb:cc:dd:ee:ff
}
```

`mac_addr=0` reverts to the permanent MAC for that network; `mac_value` lets you pin a specific MAC instead. Useful for the "always-on home Wi-Fi with a DHCP reservation" case.

## How NetworkManager wraps these

On a NetworkManager system you almost never edit `wpa_supplicant.conf` directly. NM owns the supplicant process and translates its own connection settings into the supplicant config it drives. The keys NM exposes that map to the above:

- **`connection.mac-address-randomization`.** Controls scan-time randomization. Values: `default` (driver-decides), `never`, `always`. The "always" case is what becomes `mac_addr=2` in the supplicant config NM emits.
- **`wifi-sec.mac-address-randomization`.** Same idea, scoped to Wi-Fi security settings — usually you want this set the same way as the connection-level key.
- **`wifi.cloned-mac-address`.** Per-connection cloned MAC. Values: `permanent`, `random` (random at every connect), `stable` (deterministic per-SSID), or an explicit MAC. Proteus drives this key when it rotates per-connection.
- **`802-11-wireless.mac-address`.** The hardware-level MAC, written through the kernel. Proteus avoids this in favor of `wifi.cloned-mac-address` so other NM-aware tools see consistent state.

`nmcli` cheatsheet:

```sh
# Inspect what's currently set on a connection
nmcli connection show <name> | grep -i mac

# Force scan-time randomization on an SSID
sudo nmcli connection modify <name> connection.mac-address-randomization always

# Use a fresh random MAC on every connect for one SSID
sudo nmcli connection modify <name> wifi.cloned-mac-address random
```

Proteus drives these via the NM DBus API directly — no `nmcli` shelling — but the key names and semantics are identical. Cross-ref `proteus wiki mac-recipes` for the higher-level rotation behavior.

## Validating

After you set a rotation policy, confirm it took effect. Three angles.

```sh
# What does wpa_supplicant think it's using right now?
sudo wpa_cli -i wlan0 status | grep address
sudo wpa_cli -i wlan0 get_network 0 mac_addr

# What does the kernel see on the interface?
ip -br link show wlan0

# What does the wire actually carry?
sudo tcpdump -i wlan0 -nn -e -s0 'type mgt subtype probe-req' -c 20
```

`wpa_cli get_network 0 mac_addr` returns the per-network override, or empty if the global default applies. `wpa_cli status` includes the live MAC after association. The tcpdump line captures probe requests and shows the source MAC each one carries — if `mac_addr=2` is working, you should see a different MAC on each row.

If you set scan randomization but tcpdump shows your permanent MAC on probes, the most common causes are: the driver does not advertise the capability (some older Realtek and Mediatek drivers do not), the kernel `cfg80211` `randomize_mac` feature is not built in, or NetworkManager is overriding the supplicant config back to the default. `iw phy0 info | grep -i randomize` shows the driver-side capability set.

## What NOT to touch

Proteus refuses to weaken or override these. They are security-critical and the wrong setting opens you to active attacks far worse than any fingerprinting concern.

- **`ca_cert` and `ca_path` (EAP-TLS, EAP-PEAP, EAP-TTLS).** The certificate Proteus validates the RADIUS server against. Removing this opens you to a rogue AP impersonating your home RADIUS and harvesting your inner credentials. Never disable. Never set to a wildcard. If you are tempted because "the network won't work otherwise", the right answer is to find the correct CA bundle for your institution, not to drop validation.
- **`domain_match` and `domain_suffix_match`.** Pin the RADIUS server's certificate to a specific FQDN or suffix. eduroam networks often require this for safe roaming. Removing turns server validation into "any cert chained to a trusted CA", which any cheap commercial CA can issue. Leave alone.
- **`phase2`.** Inner authentication method. Set by the network operator — `MSCHAPV2`, `GTC`, `PAP`, etc. Proteus has no opinion. Do not change without understanding the consequence; mismatch means EAP-Failure on association.
- **`eap` method choice.** PEAP, TTLS, TLS, FAST, etc. Network-operator-determined.
- **`identity` for the inner exchange.** Your real auth credential. Cross-ref `proteus wiki enterprise-wifi` for the outer-vs-inner story; the anonymous-outer-identity feature only touches the outer.
- **PSK and passphrase fields.** Never read, never logged.

The pattern: Proteus touches the on-air MAC and the outer identity. Inner authentication, certificate validation, and the EAP method choice belong to the network operator and to the user. Weakening them to "fix" a connectivity problem is the most common way users compromise themselves.

## Logging and on-disk traces

`wpa_supplicant` logs to journald via NetworkManager (or to a file if run standalone). The default verbosity (`level_str=INFO`) leaks SSIDs, BSSIDs, probe details, and EAP exchange metadata into the journal. Cross-ref `proteus wiki journald-network-logs` for the broader log-leak surface and how to bound it.

```text
# in /etc/wpa_supplicant/wpa_supplicant.conf
level_str=WARN
```

Reduces journal verbosity to warnings and errors. The tradeoff is that diagnosing an association failure becomes harder; bump back to `INFO` when you are debugging, then back down when you are done.

## Cross-refs

- `proteus wiki mac-recipes` — MAC rotation triggers, OUI pools, pinning per interface or connection
- `proteus wiki enterprise-wifi` — 802.1X anonymous outer identity, what Proteus touches and what it refuses to
- `proteus wiki journald-network-logs` — the rest of the log-surface story, including NetworkManager and systemd-resolved
- `proteus wiki network-fingerprint-checklist` — the per-layer leak inventory; `mac_addr=2` is the row that addresses scan-time MAC tracking
- `proteus wiki concepts` — what counts as a network identifier and where Proteus draws the line

External:

- wpa_supplicant.conf(5) — the canonical reference for every key on this page
- iw(8) and `iw phy0 info` — driver-side randomization capability discovery
