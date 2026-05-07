# Alpine Linux package

An `APKBUILD` for Proteus targeting Alpine 3.20+ (musl + OpenRC).

**Status: untested by the author.** Alpine community / edge build is
pending an Alpine maintainer picking this up. The recipe was lifted
from Alpine's [APKBUILD reference][1] and validated with `apkbuild-lint`
during authoring, but no live build in an Alpine chroot has been done.
Please flag any musl-vs-glibc surprises in `src/init/openrc.rs` or the
zbus/dbus dependency surface.

[1]: https://wiki.alpinelinux.org/wiki/APKBUILD_Reference

## Files

| File                        | Purpose                                          |
| --------------------------- | ------------------------------------------------ |
| `APKBUILD`                  | Alpine package recipe.                           |
| `proteus.post-install`      | Prints `proteus doctor` hint after `apk add`.    |

The OpenRC service + periodic shims live under `dist/openrc/` so they
can be reused by Gentoo and any other OpenRC distro without copying.

## Build locally

In an Alpine chroot or container, with `abuild` configured (see the
[abuild quickstart][2]):

```sh
# Stage the source tree where abuild expects it.
cd /home/build
git clone https://github.com/Kit3713/Proteus
cd Proteus/dist/alpine

# Build without checksum verification (no released tarball yet).
abuild -F -r
```

The resulting `.apk` lands in `~/packages/<repo>/<arch>/proteus-0.1.0-r0.apk`.

[2]: https://wiki.alpinelinux.org/wiki/Creating_an_Alpine_package

## Verification step

After installing the produced `.apk`:

```sh
apk add ./proteus-0.1.0-r0.apk
ldd /usr/bin/proteus               # should show musl + libdbus-1
proteus doctor                      # should report 'init system: openrc' + 'libc: musl'
```

If `proteus doctor` reports `init system: unknown`, check that
`/run/openrc` exists (i.e. that you actually booted with OpenRC, not
sysvinit fallback).

## How to help

Alpine maintainers + curious users: please test in a clean
`alpine:edge` chroot, `apkbuild-lint APKBUILD`, and PR fixes. The
biggest unknowns are:

- Whether musl + zbus 5 builds clean without any patch.
- Whether `dbus` alone covers the runtime — Alpine doesn't pull in
  `dbus-libs` transitively the way glibc distros do.
- Whether the `openrc` subpackage split matches Alpine convention
  (vs shipping the initd in the main package).

See `wiki/distro-support.md` for the full distro × init × backend
matrix; OpenRC is the wired init for Alpine.
