802.1X anonymous outer identity for enterprise Wi-Fi (eduroam, corporate). Opt-in, default off. This page exists because flipping it on can break some networks; read before enabling.

## The leak

Enterprise Wi-Fi authenticates with EAP. The "outer identity" is the EAP-Identity sent in the clear before the TLS tunnel comes up — every passive listener within range sees it, and so does the visited network's RADIUS proxy on the way to your home server. By default, NetworkManager (and most other supplicants) sends your real username as the outer identity. That username is unique to you and persists across networks — a stable cross-site identifier, broadcast unencrypted on join.

The realm part (`@university.edu`) is genuinely needed: roaming RADIUS proxies route on it. The username isn't.

## What anonymous outer identity does

Standard practice is `anonymous@<your-realm>`. The realm stays so RADIUS routing works; the local-part is replaced. Inside the TLS tunnel, the inner identity carries your real username and reaches your home RADIUS, which authenticates you normally. To anyone outside the tunnel, you're indistinguishable from every other user on your realm.

Example:

- Configured identity: `j.smith@university.edu`
- Outer identity (cleartext, on the wire): `anonymous@university.edu`
- Inner identity (inside TLS, only home RADIUS sees it): `j.smith@university.edu`

## Why this is opt-in

- Some auth servers reject mismatched outer/inner identities. Older Microsoft NPS and a number of Cisco ISE policy templates fall in this bucket.
- Unlike MAC rotation, this changes auth-protocol behavior. Failure looks like "won't associate" — which on some networks means a lockout policy ticks.
- The wiki has to list known-bad scenarios so you opt in deliberately, per network.

## How Proteus does it

Per NetworkManager connection profile, over the NM dbus API:

- Reads `802-1x.identity` from the connection.
- Extracts the realm (the part after `@`), or uses `anonymous_realm` from config when strategy is `manual`.
- Writes `802-1x.anonymous-identity = anonymous@<realm>` to that connection.
- Touches nothing else — no certificate changes, no CA bundle swaps, no EAP-method changes, no inner-identity edits.

Reverting clears `802-1x.anonymous-identity` on the connections Proteus modified, leaving every other 802.1X field on the profile untouched.

## Configuration

```toml
[enterprise_wifi]
# Master switch. Opt-in.
anonymous_outer_identity = false

# "auto" extracts the realm from `802-1x.identity` (the part after "@").
# "manual" uses `anonymous_realm` below verbatim.
realm_strip_strategy = "auto"

# Used when `realm_strip_strategy = "manual"`. Empty otherwise.
anonymous_realm = ""

# Per-NM-connection overrides. Map of connection name to bool.
# Use this to deny-list connections known to break, or to enable
# only on networks you have tested.
per_connection_overrides = {}
```

## Use cases

- **eduroam.** Most home institutions support anonymous outer identity. Recommended; eduroam guidance has called for it for years. Test once on your home realm, then forget.
- **Home institution Wi-Fi (non-eduroam).** Depends on the auth server config. Test before committing.
- **Corporate (Microsoft NPS, Cisco ISE).** Many configurations reject. If the policy was written assuming outer == inner, you will fail to associate. Disable for that connection and move on.

## How to test before committing

Pick one connection, enable, try to associate, watch the logs. If it works, leave it; if it doesn't, disable and you're back where you started.

```sh
# Enable for one NM connection profile only
sudo proteus enterprise-wifi enable --connection "MyOrgWiFi"

# Bring it up and watch
nmcli connection up "MyOrgWiFi"
journalctl -u NetworkManager -f

# If association fails (look for EAP-Failure), revert just that connection
sudo proteus enterprise-wifi disable --connection "MyOrgWiFi"
```

`proteus revert` also clears every connection Proteus touched, in one shot.

## Failure mode

Auth servers that reject mismatched identities respond with EAP-Failure on the first message — before the TLS tunnel even starts. In `journalctl -u NetworkManager` you will see the EAP exchange terminate immediately after the EAP-Identity response. `wpa_supplicant` logs the same. The connection will not associate.

Recovery is to disable for that connection (above) or list it in `per_connection_overrides` with `false`.

## Per-connection overrides

When `anonymous_outer_identity = true` globally, use `per_connection_overrides` as a deny-list:

```toml
[enterprise_wifi]
anonymous_outer_identity = true
per_connection_overrides = { "CorpWiFi-NPS" = false, "OldVPN" = false }
```

When the master switch is off, use the same map as an allow-list — only the named connections get the anonymous identity:

```toml
[enterprise_wifi]
anonymous_outer_identity = false
per_connection_overrides = { "eduroam" = true }
```

This is the recommended pattern: explicit per-network opt-in.

## Verification

After associating, confirm the outer identity went out as expected.

```sh
# Look for the EAP-Identity request/response in NM logs
journalctl -u NetworkManager -n 200 | grep -iE 'eap|identity'

# Or capture EAPOL on the wire — outer ID is in cleartext
sudo tcpdump -i wlan0 -nn 'ether proto 0x888e' -vv
```

You want to see `anonymous@<realm>` as the EAP-Identity response, not your real username.

## What Proteus does not touch

- **Inner identity.** Your real auth credential. Never altered. The whole point is to leak nothing about it on the outer.
- **Certificate validation.** Never relaxed. `802-1x.system-ca-certs`, `802-1x.ca-cert`, `802-1x.domain-suffix-match`, `802-1x.subject-match` — all left alone. Disabling cert validation to "fix" an EAP failure opens you to a rogue AP impersonating your home RADIUS and harvesting your inner credentials. Don't.
- **EAP method.** `802-1x.eap` (PEAP, TTLS, TLS, FAST, etc.) is your choice. Proteus has no opinion.
- **Phase-2 method.** `802-1x.phase2-auth` is yours.
- **Passwords or certificates.** Never read, never written, never logged.

## Cross-refs

- `proteus wiki mac-recipes` — MAC rotation interacts with auth. A new MAC may trigger a fresh EAP exchange, and pinned connections may want a stable MAC alongside anonymous outer identity.
- `proteus wiki concepts` — the mental model for what counts as a network identifier and where Proteus draws the line.
- `proteus wiki troubleshooting` — EAP failure recovery, log triage, common dead-ends.
