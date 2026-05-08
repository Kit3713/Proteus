Hostname rotation patterns. For the mental model, see `proteus wiki concepts`.

Hostnames are network identifiers. mDNS announces them. DHCP option 12 sends them. Your shell prompt prints them. Same string, several layers, several daemons reading from different files. Proteus changes them coherently so they don't disagree.

## The three hostname fields

systemd splits "the hostname" into three. Proteus manages all of them via the `org.freedesktop.hostname1` DBus interface — never by writing files directly.

**Static hostname.** The kernel hostname. Lives in `/etc/hostname`, surfaces as `/proc/sys/kernel/hostname`. Survives reboots. mDNS uses it for `<host>.local` resolution. DHCP option 12 sends it by default. This is the one most people mean when they say "hostname".

**Pretty hostname.** Lives in `/etc/machine-info` as `PRETTY_HOSTNAME`. Display-friendly, allows spaces and Unicode — `Chris's Laptop`. GNOME's About panel and KDE's system info read this. No daemon broadcasts it on its own, but it leaks via screen-sharing UIs and some Bluetooth pairing dialogs.

**Transient hostname.** Lives only in the kernel runtime. Not on disk. Often set over DHCP when a server returns option 12 in its reply. Lives until reboot or until something overwrites it.

If these three disagree, weird things happen. A DHCP server might announce `chris-laptop` to your local DNS while your shell prompt says `linksys-7a3f`. Proteus changes them as a set.

## Modes

The `[hostname] mode` knob in `/etc/proteus/config.toml` selects the strategy.

**`wordlist`** (default). Pick from a curated list of router-flavored words — `linksys`, `tplink`, `asus`, `fios-router`, `bthub-3`, and around 500 similar entries. The output looks like a generic CPE on the local segment. Blends into the background of the kind of devices a network operator already expects to see.

**`generic`**. Pin to a fixed value. Default is `fedora`. For users who want a hostname that doesn't change but also doesn't identify them — a fresh Fedora install presents this exact string, and there are millions of them.

**`pinned`**. Use the exact value the user provided in `[hostname] pinned_value`. For users who have a name they want and are willing to keep it across rotations.

## The wordlist

Compile-time constant. About 500 entries. Embedded in the binary, no network fetch, no on-disk file to tamper with. Curated so every entry is RFC 1123-valid out of the gate — lowercase letters, digits, hyphens, no leading or trailing hyphen, max 63 characters per label.

The list leans on real CPE patterns: vendor names (`netgear`, `dlink`), product family fragments (`archer-c7`, `nighthawk`), ISP-issued patterns (`bthub-3`, `skyhub`, `fios-router`). The aim is plausibility, not uniqueness.

## rotate_with_mac

`[hostname] rotate_with_mac` is `false` by default. Opt-in.

When `true`, every MAC rotation also picks a fresh hostname (subject to the active mode). Stronger correlation defense — an observer can't link your `linksys-7a3f` session to your `tplink-94c1` session by hostname.

The cost is real:

- Your shell prompt changes between rotations
- Per-host bash history splits — if you keep `~/.bash_history.<hostname>`, you'll accumulate stubs
- Per-host config (terminal themes keyed off hostname, font cache paths) may glitch
- Automated tools that key off hostname — backup scripts, monitoring agents, license servers — may break

Default off because the failure modes are silent and annoying. Turn it on if the threat model is worth the friction.

## Original hostname

Captured into `/var/lib/proteus/state.json` on first apply, never re-captured. Sacred. Same rule as the original MAC. See `proteus wiki concepts`.

- `proteus original` prints it
- `proteus revert` restores it
- `proteus reset` clears your config but never touches the cache
- `proteus uninstall --purge` is the only thing that removes it

If you've broken things badly, the original hostname is still there.

## Constraints

Hostnames are RFC 1123-valid: lowercase letters, digits, hyphens. No underscores. No leading or trailing hyphen. Max 63 characters per label. Max 253 characters total.

Proteus generates only RFC-valid names from the wordlist. User-pinned values are validated against the same rules. Invalid `pinned_value` rejects at config load with a clear error naming the rule it broke. No silent normalization — if you typed `My_Laptop`, Proteus tells you underscores aren't allowed rather than quietly turning it into `my-laptop`.

## Recipes

**I want my hostname to look like a default Fedora install.**

```toml
[hostname]
mode = "generic"
pinned_value = "fedora"
```

Static, pretty, and transient all become `fedora`. No rotation. The most common low-effort blend-in.

**I want a different generic hostname every 2h.**

```toml
[hostname]
mode = "wordlist"
rotate_with_mac = true
```

Each MAC rotation picks a fresh router-flavored name. Read the tradeoffs in the `rotate_with_mac` section above before turning this on.

**Pin to my preferred name forever.**

```toml
[hostname]
mode = "pinned"
pinned_value = "trustedlaptop"
```

Stays `trustedlaptop` across reboots and rotations. Useful when you have an SSH config, a DHCP reservation, or a license server that expects a specific name.

## DHCP interaction

Hostname rotation coordinates with DHCP. The default `[dhcp] suppress_hostname = true` means option 12 isn't sent regardless of whatever the static or pretty hostname is set to. Your hostname stays local; the DHCP server learns nothing about it.

If you turn `suppress_hostname` off (some networks won't issue a lease without it), the static hostname goes out as option 12 on every DHCP request. Rotating the hostname then directly affects what the DHCP server sees.

See `proteus wiki dhcp`.

## mDNS interaction

The mDNS responder uses the static hostname for `<host>.local` resolution on the local segment. Anyone on the LAN running `avahi-browse` sees it.

Disabling the mDNS responder (the default — see `proteus wiki discovery`) makes the hostname locally irrelevant. Other devices can't resolve `<your-host>.local` because nothing is answering. The hostname still matters for shell prompts and DHCP, but not for local-network discoverability.

## Application impact

Things that change when the hostname rotates:

- **Shell prompt** — `\h` in `PS1` updates immediately on new shells; existing shells keep the old one until restart
- **bash history** — if you use the per-host `HISTFILE=~/.bash_history.$(hostname)` pattern, your history splits across rotations
- **Per-host themes / fonts** — applications that key cache paths off hostname re-init, may flicker
- **Automated tools** — backup agents, monitoring (Prometheus node exporter labels), license servers, anything that identifies the machine by hostname will see a different machine

`generic` and `pinned` modes don't trigger any of this. Only `wordlist` with `rotate_with_mac = true` does.

## Cross-references

- `proteus wiki dhcp` — option 12 suppression, hostname-DHCP coupling
- `proteus wiki discovery` — mDNS responder, why disabling it makes the hostname locally invisible
- `proteus wiki mac-recipes` — when `rotate_with_mac` is on, MAC and hostname rotate together
- `proteus wiki concepts` — sacred original cache, why state is captured once and never re-captured
