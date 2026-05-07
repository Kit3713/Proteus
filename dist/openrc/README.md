# OpenRC service + periodic shims

Init-system artifacts for distros that boot with OpenRC: Alpine, Gentoo,
Artix-OpenRC. These are the same shapes the `init::openrc::Openrc`
builder produces at runtime; we ship them as plain files so packagers
(see `dist/alpine/APKBUILD`, `dist/gentoo/proteus-0.1.0.ebuild`) don't
have to call into the binary at install time.

| File                        | Install path                                     |
| --------------------------- | ------------------------------------------------ |
| `proteus.initd`             | `/etc/init.d/proteus`                            |
| `proteus-rotate.periodic`   | `/etc/periodic/hourly/proteus-rotate`            |
| `proteus-check.periodic`    | `/etc/periodic/15min/proteus-check`              |

## How to test locally

On any OpenRC host:

```sh
sudo install -Dm755 dist/openrc/proteus.initd /etc/init.d/proteus
sudo install -Dm755 dist/openrc/proteus-rotate.periodic /etc/periodic/hourly/proteus-rotate
sudo install -Dm755 dist/openrc/proteus-check.periodic  /etc/periodic/15min/proteus-check
sudo rc-service proteus start
sudo rc-update add proteus default
sudo run-parts --test /etc/periodic/hourly | grep proteus
```

The `run-parts --test` output should list `proteus-rotate`. The actual
rotation cadence still comes from `[rotation] interval` in
`/etc/proteus/config.toml` once Phase D lands; until then the periodic
buckets fire at OpenRC's fixed cadence (15min/hourly/daily/weekly).

## Verification step

After `rc-service proteus start`, run `proteus doctor` and confirm:

- "init system: openrc" appears in the output.
- "backend:" reflects whatever backend is actually present (usually
  NetworkManager on Alpine, or `raw` on a minimal install).

If `proteus doctor` reports `init system: unknown`, the host either
isn't OpenRC or the runtime root (`/run/openrc`) hasn't been initialised
yet — reboot and retry.

## How to help

These artifacts are tested on Alpine 3.20 and Gentoo (musl + glibc).
If you run a different OpenRC distro (Artix-OpenRC, Hyperbola, Devuan
with the OpenRC alternative) please run `proteus doctor` and PR fixes
against this README + `src/init/openrc.rs` if anything looks wrong.

See `wiki/distro-support.md` for the full distro × init × backend matrix.
