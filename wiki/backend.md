Proteus controls a Linux host's network identifiers through a `NetworkBackend` trait that abstracts over NetworkManager, systemd-networkd, and raw `ip`+`iw` calls. This page is the user-facing view of that abstraction — what each backend covers, how `proteus` picks one at runtime, how to pin a choice, and what to expect when the host doesn't ship NM at all.

The roadmap rationale lives in `docs/ROADMAP.md` Milestone 1; this page is the day-to-day operator's reference.

## What a backend is

A backend is the layer that actually rewrites L2 / L3 / DHCP state on the host. Proteus does not touch `/sys/class/net/<iface>/address` or `/etc/NetworkManager/system-connections/*` directly — every mutation flows through the trait, and the trait's chosen impl decides whether that mutation goes over NM's DBus, into a `systemd-networkd` drop-in, or through a `wpa_supplicant`/`iwd` socket directly.

Three impls ship today:

| Backend | Status | Probe path | What it speaks |
|---|---|---|---|
| `nm` | ✅ fully wired | `/run/NetworkManager` exists | NetworkManager DBus (`org.freedesktop.NetworkManager`) |
| `networkd` | 🚧 partial | `/run/systemd/network` exists | `org.freedesktop.network1` DBus + `/etc/systemd/network/*.network` drop-ins |
| `raw` | 🚧 partial | `/sbin/ip` or `/usr/bin/ip` exists | direct `ip`/`iw`/`wpa_supplicant`/`iwd` calls |

"Partial" means the read paths return sensible defaults and the trait compiles, but the mutating writes (set cloned MAC, set DHCP options, write 802.1X anonymous identity, renew lease) still bail with `"backend::<name>: not yet implemented (Milestone 1 follow-up)"`. Track the full implementations in `docs/ROADMAP.md`.

## Selection at runtime

`proteus doctor` reports the backend matrix and the auto-pick. Example on a Fedora host with NM:

```
Backend
  ✓ nm=yes, networkd=no, raw=yes
  ✓ auto → nm
```

Example on Alpine with iwd-only (no NM, no networkd, raw available):

```
Backend
  ✓ nm=no, networkd=no, raw=yes
  ⚠ auto → raw (write paths still partial; see wiki/backend)
```

Example on a Fedora Server install:

```
Backend
  ✓ nm=no, networkd=yes, raw=yes
  ⚠ auto → networkd (write paths still partial; see wiki/backend)
```

The auto-pick walks `nm → networkd → raw` and chooses the first that's `available()`. To pin a specific backend, set `[backend] driver = "nm"` (or `"networkd"`, `"raw"`, `"auto"`) in `/etc/proteus/config.toml`. Anything outside the four legal values logs a warning and falls back to `"auto"`.

## What the trait covers

Every per-feature command in `proteus` flows through the trait, not directly to NM. The methods you'd hit if you read the source:

- `list_devices` → `[BackendDevice]` for every interface the backend can manage.
- `set_cloned_mac(device, mac)` → write the cloned MAC the next association will use.
- `read_cloned_mac(device)` → what's currently set as the cloned MAC.
- `read_factory_mac(iface)` → the burned-in factory address, via `mac::factory::permanent_address`.
- `rotate_if_needed(iface, cooldown)` → typed `RotateOutcome` (the entry the NM dispatcher hits).
- `list_connections(device)` / `read_connection_id(connection)` / `read_connection_uuid(connection)` → connection-profile metadata.
- `set_dhcp_settings(connection, snapshot)` → push DHCP-option overrides.
- `set_ipv6_settings(connection, settings)` → push IPv6 `addr-gen-mode` / DUID / IAID.
- `renew_lease(device)` → release+renew the DHCP lease (`Reapply` then fall back to `Disconnect`+`ActivateConnection`).
- `write_anonymous_identity(connection, value)` → 802.1X EAP anonymous outer identity.

## Picking a backend on a non-NM host

If you run NetworkManager, you don't need to think about this. The auto-pick will land on `nm` and every feature works. If you don't run NM, the situation is:

- **systemd-networkd-driven host (Fedora Server, NixOS with networkd, custom Debian).** Auto-pick lands on `networkd`. Read paths work; write paths are partial. Pin to `networkd` explicitly if you want the doctor matrix to stop nagging you about the partial state.
- **Alpine + iwd, OpenRC + wpa_supplicant, Void.** Auto-pick lands on `raw`. Same caveat — read paths work; the write paths are stubs that bail loudly. The trait is in place so the actual `ip`/`iw`/`wpa_supplicant` calls drop in cleanly when the follow-up lands.
- **No NM, no networkd, no `ip`.** `proteus doctor` reports `auto: no backend available` and refuses to apply anything. Install one of the three before trying again. (You probably want `iproute2` at minimum.)

## What about the existing `crate::nm::*` modules?

`src/nm/mod.rs`, `src/nm/apply.rs`, `src/nm/dhcp.rs`, `src/ipv6/nm.rs`, and `src/enterprise_wifi/nm.rs` are the NM impl's internals — they're called only from `backend::nm`, not from `commands::*` directly. The migration in `a5cbe8c` (Milestone 1) routed every per-command call site through the trait. The `crate::nm::*` modules stay around as the NM impl's internals; they're not deprecated, they're just no longer the public surface.

## Verification checklist

Before promoting `networkd` or `raw` to ✅ status, the trait acceptance test wants:

```sh
# Full apply/revert/rotate cycle on each backend, in a podman+systemd container.
sudo bash tests/integration/scenarios/nm.sh        # passes today
sudo bash tests/integration/scenarios/networkd.sh  # 🚧 — write paths bail
sudo bash tests/integration/scenarios/raw.sh       # 🚧 — write paths bail
```

The `nm.sh` scenario exercises the full feature set against a podman container running Fedora 43 + NetworkManager. The networkd / raw scenarios are skeletons today; the assertion checklist is committed as commented blocks in those scripts and lights up as the matching backend's writes fill in.

## Limits

- Wireless **probing** (active vs passive scan policy) is reported by `proteus rf scan` per Wi-Fi iface; the actual scan policy NM uses lives in NM's wpa_supplicant config and is not currently part of the trait. Pinning passive scan from inside Proteus is on the roadmap.
- The `read_factory_mac` path always delegates to `mac::factory::permanent_address` — there's no per-backend factory-MAC source. Backends differ in how they *write* the cloned MAC, not in how they read the burned-in one.
- DHCP option 55 (parameter request list) is honoured by the schema and the persona-fingerprint surface, but NM exposes no direct setter for it; the persona's `parameter_request_list` is logged at debug level today and will land on the wire when networkd's native dhclient.conf path goes live (Milestone 1 follow-up).

The roadmap (`docs/ROADMAP.md` Milestone 1) tracks the gaps. New backends are welcomed via the same trait; community contributions are tracked in `CONTRIBUTING.md`.
