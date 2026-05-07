# DBus surface — Proteus, v0.3.0-alpha

This document enumerates every DBus interface, method, property, and signal the `proteus` binary touches at runtime. The intent is that an external security reviewer can audit Proteus's privileged surface against this artifact rather than against the source — the source is referenced for line-numbers but the contract is documented here.

The audience is anyone reasoning about the trust model: "what does this root-running binary actually call out to, and what does it accept back?" Every method below runs as `uid=0` (the polkit policy scopes Proteus to root for mutating commands; read-only commands run as the invoking user but only consume read-only DBus surface).

Track changes through this file's git history. New DBus calls land in this doc as part of the same PR that introduces them.

## Table of contents

1. NetworkManager (`org.freedesktop.NetworkManager`)
2. systemd-hostnamed (`org.freedesktop.hostname1`)
3. BlueZ (`org.bluez`)
4. systemd1 (`org.freedesktop.systemd1`) — read-only inspection only

## 1. NetworkManager

NM is the primary mutating surface. Every per-feature command in the NM backend reaches into one of the following interfaces.

### 1.1 `org.freedesktop.NetworkManager`

Path: `/org/freedesktop/NetworkManager`. Defined in `src/nm/mod.rs`.

| Direction | Member | Args | Validation |
|---|---|---|---|
| Method | `GetDevices()` | none | none — read-only enumeration |
| Method | `GetDeviceByIpIface(iface)` | `iface: String` | `iface` is the kernel netdev name; we never pass user-controlled strings unvalidated. The single caller is `find_device_by_iface`, which is the resolution helper, not a passthrough |
| Method | `ActivateConnection(conn, dev, opath)` | `conn: ObjectPath, dev: ObjectPath, opath: ObjectPath` | All three args are paths returned by NM in a previous call; we don't construct them from user input |

### 1.2 `org.freedesktop.NetworkManager.Device`

Per-device interface. Defined in `src/nm/mod.rs`.

| Direction | Member | Notes |
|---|---|---|
| Property (read) | `Interface` | netdev name, used to label per-iface output |
| Property (read) | `DeviceType` | NM enum, mapped to `BackendKind { Wifi, Ethernet, Other }` |
| Property (read) | `HwAddress` | live netdev address — **never** cached as factory MAC (issue #208 closed) |
| Property (read) | `Managed` | bool, gates whether Proteus rotates this device |
| Property (read) | `AvailableConnections` | array of `ObjectPath` |
| Property (read) | `ActiveConnection` | `ObjectPath` or `/` for "no active connection" |
| Method | `Reapply(connection_dict, version_id, flags)` | called with `(empty, 0, 0)` to renew DHCP without disturbing L2; documented at `src/nm/dhcp.rs::renew_lease` |
| Method | `Disconnect()` | fallback path for `Reapply` non-support |

### 1.3 `org.freedesktop.NetworkManager.Settings`

Path: `/org/freedesktop/NetworkManager/Settings`. Defined in `src/nm/mod.rs`.

| Direction | Member | Args | Notes |
|---|---|---|---|
| Method | `ListConnections()` | none | read-only |
| Method | `GetConnectionByUuid(uuid)` | `uuid: String` | uuid validated as 36-char `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` shape via `is_uuid_shape` (`src/state.rs`) before being passed to NM |

### 1.4 `org.freedesktop.NetworkManager.Settings.Connection`

Per-connection settings interface. The mutating surface — every secret-merge invariant lives here.

| Direction | Member | Args | Validation |
|---|---|---|---|
| Method | `GetSettings()` | none | read-only; result is the connection-settings dict |
| Method | `GetSecrets(setting_name)` | `setting_name: String` | called only with constants from `nm::SECRET_SECTIONS`: `802-11-wireless-security`, `802-1x`, `vpn`, `wireguard`, `gsm`, `cdma`, `pppoe`, `macsec`. **No user input flows into this arg.** |
| Method | `Update(settings)` | `settings: ConnectionSettings` | Always called via `nm::update_with_secrets(conn, path, settings)` (`src/nm/mod.rs`). The helper merges every relevant `GetSecrets` section back into `settings` before pushing — this is the issue #207 invariant; absent the merge, NM interprets the missing keys as "user cleared their password" and wipes its secrets store |

#### Settings dict shape we write

We touch a small subset of the keys NM accepts. Values are chosen by the per-feature command and validated on the way in:

- `802-11-wireless.cloned-mac-address` (`ay`, byte array): MAC parsed via `Mac::from_str` so junk strings are rejected before the wire. Issue #100 / #207.
- `802-11-wireless.assigned-mac-address` (`s`, string): same MAC rendered as `aa:bb:cc:dd:ee:ff` for legacy NM ≤1.36.
- `802-11-wireless.scan-rand-mac-address` = `"random"`. Milestone 4b.
- `802-11-wireless.mac-address-randomization` = `2_i32` (always randomize). Milestone 4b.
- `802-3-ethernet.cloned-mac-address` / `assigned-mac-address`: same as Wi-Fi, ethernet variant.
- `802-1x.anonymous-identity` (`s`): persona-derived value or empty string for clear; redacted via `enterprise_wifi::redact_identity` before display.
- `ipv6.addr-gen-mode` (`i32`): mapped from string token via `addr_gen_mode_to_int`; rejects unknown tokens before the wire (`src/nm/mod.rs`).
- `ipv6.dhcp-duid`, `ipv6.dhcp-iaid` (`s`): `"ll"` / `"mac"` constants, never user input.
- `ipv4.dhcp-send-hostname` (`b`), `dhcp-hostname`, `dhcp-fqdn`, `dhcp-vendor-class-identifier`, `dhcp-client-id` (`s`): persona-derived or empty.
- `connection.user-data` (`a{ss}`): Proteus tag (`proteus.managed=true` etc.); third-party entries preserved via the read-modify-write helper at `src/nm/dhcp.rs::tag_user_data`.

### 1.5 NM signals subscribed to

- `org.freedesktop.NetworkManager.Device::StateChanged` — connection-up event source (Milestone 4c). Filter: `new_state == NM_DEVICE_STATE_ACTIVATED (100)`. Subscribed only when the long-running `proteus events` daemon is opted in.

## 2. systemd-hostnamed

Path: `/org/freedesktop/hostname1`. Defined in `src/hostname/dbus.rs`.

| Direction | Member | Args | Notes |
|---|---|---|---|
| Method | `SetHostname(name, user_interaction)` | `name: String, user_interaction: bool` | name is rendered via `hostname::render_template` or pulled from the wordlist; never raw user input. We pass `user_interaction = false` so the call doesn't trigger an auth prompt under polkit |
| Method | `SetPrettyHostname(name, ...)` | same | same constraints |
| Method | `SetStaticHostname(name, ...)` | same | same constraints |
| Property (read) | `Hostname`, `StaticHostname`, `PrettyHostname` | for `proteus hostname status` |

The hostname is RFC 1123 lowercased + length-clamped before the wire.

## 3. BlueZ

Path: `/org/bluez`. Defined in `src/bluetooth/mod.rs`.

| Interface | Member | Args | Notes |
|---|---|---|---|
| `org.freedesktop.DBus.ObjectManager` | `GetManagedObjects()` | none | read-only adapter enumeration |
| `org.bluez.Adapter1` | `Alias` (write property) | `String` | persona-derived or wordlist-rendered |
| `org.bluez.Adapter1` | `Powered`, `Discoverable` (write properties) | `bool` | constants — `Powered=true` for normal operation, `Discoverable=false` per Proteus policy |
| `org.bluez.Adapter1` | `Address`, `Powered`, `Discoverable`, `Alias` (read properties) | for status |

We never set `Discoverable=true` from Proteus; the kill-switch path sets `Powered=false` to bring radios down.

## 4. systemd1 (read-only)

Path: `/org/freedesktop/systemd1`. Used only for the doctor health-check + the NTP detect-and-defer guard.

| Member | Why |
|---|---|
| `LoadUnit(unit)` | locate `chronyd.service` / `ntpd.service` / `systemd-resolved.service` for the `unit_is_active` probe in `src/dns/`, `src/ntp/`. Read-only — never `StartUnit` / `StopUnit` |
| `Unit.ActiveState` (property read) | the actual probe |

We do not call any mutating systemd1 method from Proteus. Periodic-rotation systemd units are installed by `dist/systemd/*.{service,timer}` files (read by systemd at unit-load time, not via Proteus DBus calls).

## What Proteus does NOT call

This list is deliberate — auditors flagging "should Proteus also touch X?" should land here:

- **`org.freedesktop.NetworkManager.Settings.AddConnection` / `DeleteConnection`** — Proteus never creates or deletes NM connection profiles; it only mutates existing ones. The user owns the profile lifecycle.
- **`org.bluez.Device1.Pair` / `Connect` / `Disconnect`** — pairing flow is the user's. Proteus only sets adapter-level identity.
- **`org.freedesktop.systemd1.Manager.StartUnit` / `StopUnit` / `EnableUnitFiles` / `DisableUnitFiles`** — Proteus does not start, stop, enable, or disable systemd units at runtime. The install scripts do that at install time.
- **`org.freedesktop.NetworkManager.WiFi.RequestScan`** — Proteus reads scan results indirectly via `iw dev <iface> scan dump` (read-only); it does not trigger active scans through NM.
- **`org.freedesktop.login1`** — Proteus does not interact with seat / session management.

## Validation guarantees

- Every `String` arg passed to a mutating method is either:
  - a constant defined in Proteus source (e.g. `"ll"` for `dhcp-duid`, `"random"` for `scan-rand-mac-address`)
  - a value parsed and validated through a Proteus type (`Mac::from_str`, `is_uuid_shape`, `addr_gen_mode_to_int`, `enterprise_wifi::extract_realm`) before reaching the wire
  - a wordlist-derived value rendered through `hostname::render_template` or `persona::template::render_template`
- We never pass an unvalidated CLI-derived string into an NM `Update` dict. The CLI-derived `connection` arg of `proteus enterprise-wifi enable <connection>` is resolved to an `ObjectPath` via `find_connection_by_id` before any settings-dict write happens.
- `bail!` / `anyhow!` is the standard error path; DBus errors carry their NM-side context via `context()` so the operator sees "calling Settings.Connection.Update: <NM message>" rather than a bare "DBus error".

## Reproducible audit

To regenerate the list of `proxy(...)` interfaces in the source:

```sh
grep -rn 'interface = "' src/ | grep -v '#\['
```

To list every `Update` / `SetX` / `GetSecrets` call site:

```sh
grep -rn '\.update(\|\.set_alias(\|\.set_hostname(\|\.set_powered(\|\.set_discoverable(\|\.get_secrets(\|\.reapply(\|\.disconnect(' src/
```

Both commands are stable across `cargo fmt` and intentional commits land in this file alongside the source change.

## Reporting

Audit findings against this document go in GitHub Issues with the `security` label. The threat model lives at `wiki/threat-model.md`; this file is the implementation-side artifact that supports it.
