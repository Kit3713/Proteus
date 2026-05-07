# Void Linux package

An xbps-src `template` for Proteus on Void (and Artix-Runit). Ships
runit service files via `dist/runit/proteus/`.

**Status: untested by the author.** No live Void build has been done.
The template was written against Void's [packaging manual][1] and lints
clean against `xbps-src lint`, but neither glibc nor musl Void chroots
have actually compiled it yet.

[1]: https://github.com/void-linux/void-packages/blob/master/Manual.md

## Files

| File                              | Purpose                                |
| --------------------------------- | -------------------------------------- |
| `template`                        | xbps-src package template.             |

The runit service tree itself lives at `dist/runit/proteus/` so it can
be reused by Artix-Runit and any other runit distro:

| File                              | Install path                              |
| --------------------------------- | ----------------------------------------- |
| `dist/runit/proteus/run`          | `/etc/sv/proteus/run`                     |
| `dist/runit/proteus/log/run`      | `/etc/sv/proteus/log/run`                 |

## Build locally

In a void-packages clone:

```sh
git clone --depth=1 https://github.com/void-linux/void-packages
cd void-packages
./xbps-src binary-bootstrap

# Wire the template into srcpkgs.
mkdir -p srcpkgs/proteus
cp /path/to/Proteus/dist/void/template srcpkgs/proteus/template

# Optional: import the runit service tree into the void-packages
# convention so `vsv proteus` works without the manual install lines.
mkdir -p srcpkgs/proteus/files
cp -a /path/to/Proteus/dist/runit/proteus srcpkgs/proteus/files/proteus

# Build for the host arch.
./xbps-src pkg proteus

# Or musl, or armv7l-musl, etc:
./xbps-src -a aarch64-musl pkg proteus
```

The resulting `.xbps` lands in `hostdir/binpkgs/`.

## Verification step

After installing:

```sh
xbps-install --repository=hostdir/binpkgs proteus
ldd /usr/bin/proteus              # musl on -musl arches, glibc otherwise
proteus doctor                     # should report 'init system: runit'
ln -sf /etc/sv/proteus /var/service/proteus     # enable
sv status proteus                  # 'run:' line within ~5s
```

If `proteus doctor` reports `init system: unknown`, check that
`/etc/runit/runsvdir` exists — it's runit's pointer to the active
service tree and what `init::runit::Runit::detect` keys off.

## How to help

Void maintainers + curious users: please test in a clean
`void-musl-bootstrap` chroot, run `./xbps-src lint srcpkgs/proteus/template`,
and PR fixes. The biggest unknowns are:

- Whether musl + zbus 5 builds clean without any patch (same Q as Alpine).
- Whether the manual `vinstall` for the runit service tree should be
  replaced with `vsv proteus` once the void-packages import happens.
- Whether `dbus` alone is the right runtime dep, or if `elogind` should
  be a `Recommends:` for the resume hook to fire.

See `wiki/distro-support.md` for the full distro × init × backend matrix;
runit is the wired init for Void and Artix-Runit.
