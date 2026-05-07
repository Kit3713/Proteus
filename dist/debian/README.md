# Debian / Ubuntu package

Source-package layout for building Proteus as a `.deb` for Debian and Ubuntu.

Proteus targets Fedora 43+ as its primary platform, but Debian-family distros
share systemd, NetworkManager, BlueZ, and the same dbus surface — the only
real difference is packaging. Ubuntu LTS releases (22.04 jammy, 24.04 noble,
26.04 the next LTS) are the supported targets; Debian stable (bookworm) and
testing (trixie) work with the same source package.

## Build

From the repo root, with the source-package tooling installed:

    sudo apt install build-essential debhelper devscripts dh-cargo \
        cargo rustc pkg-config libdbus-1-dev
    cp -r dist/debian debian
    dpkg-buildpackage -us -uc -b

`-us -uc` skips signing for local test builds. `-b` builds a binary-only
package (no source tarball). The resulting `.deb` lands one directory up
as `../proteus_0.1.0-1_amd64.deb` (or `_arm64.deb` when cross-built).

To build for arm64 from amd64:

    dpkg-buildpackage -us -uc -b -a arm64

This requires the `crossbuild-essential-arm64` package and a Rust toolchain
that can target `aarch64-unknown-linux-gnu`.

## Install

    sudo dpkg -i ../proteus_0.1.0-1_amd64.deb
    sudo apt-get install -f   # pull in any missing runtime deps

## Where files land

| File                  | Path                                                       |
| --------------------- | ---------------------------------------------------------- |
| Binary                | `/usr/bin/proteus`                                         |
| Man page              | `/usr/share/man/man1/proteus.1`                            |
| Bash completion       | `/usr/share/bash-completion/completions/proteus`           |
| Zsh completion        | `/usr/share/zsh/vendor-completions/_proteus`               |
| Fish completion       | `/usr/share/fish/vendor_completions.d/proteus.fish`        |
| systemd units         | `/lib/systemd/system/proteus-*.{service,timer}`            |
| NM dispatcher hook    | `/etc/NetworkManager/dispatcher.d/01-proteus`              |
| polkit policy         | `/usr/share/polkit-1/actions/com.kit3713.proteus.policy`   |
| Config dir            | `/etc/proteus/` (empty, owned by package)                  |
| State dir             | `/var/lib/proteus/` (mode 0700, empty)                     |

Debian convention puts vendor-shipped systemd units under `/lib/systemd/system`,
not `/usr/lib/systemd/system` (Arch / Fedora). `systemctl` searches both, so
the only practical difference is the path the package owns.

## Architectures

`amd64` and `arm64` only. Proteus targets laptops and small servers; 32-bit
ARM and i386 aren't worth the test matrix. ARM64 covers Raspberry Pi 4/5,
Ampere/Graviton servers, and Apple Silicon under Asahi Linux.

## Ubuntu LTS targets

- 22.04 jammy — Rust 1.75 in archive; needs rustup or the rust-all PPA for
  edition 2024 (Rust 1.85+).
- 24.04 noble — Rust 1.75 in archive; same rustup story until backports.
- 26.04 (forthcoming) — should ship Rust 1.85+ in main.

The `Build-Depends` line pins `rustc (>= 1.85)`. On older releases the build
will fail loudly rather than silently producing a binary that crashes on a
2024-edition feature.

## Launchpad PPA

Once `v0.1.0` is tagged:

1. Run `dpkg-buildpackage -S -us -uc` to build a source package.
2. Sign with `debsign ../proteus_0.1.0-1_source.changes`.
3. `dput ppa:kit3713/proteus ../proteus_0.1.0-1_source.changes`.
4. Launchpad builds amd64 + arm64 and publishes to
   `ppa:kit3713/proteus`. Users add the PPA and `apt install proteus`.

The PPA infrastructure is free for open-source projects; the only real
ongoing cost is keeping the changelog accurate and rebuilding for each
new Ubuntu LTS within its support window.

## Debian unstable submission prep

The current directory ships everything Debian's [Debian Cargo packaging
guide][1] expects for a hand-written (non-`debcargo`) source package:

| File                | Purpose                                                |
| ------------------- | ------------------------------------------------------ |
| `control`           | Source + binary package metadata.                      |
| `rules`             | Build/install/test overrides (debhelper compat 13).    |
| `compat`            | Legacy debhelper-compat marker (kept for older tools). |
| `copyright`         | DEP-5 copyright file (GPL-3.0-or-later).               |
| `changelog`         | Debian-format changelog (`unstable; urgency=medium`).  |
| `source/format`     | `3.0 (quilt)`.                                         |

**Status: submission prep, not submitted.** The package builds locally
with `dpkg-buildpackage -us -uc -b`. Outstanding steps owned by the
maintainer (not in scope for this PR):

1. Get a Debian sponsor (mentors.debian.net or a NM-team contact).
2. Sign the upload with `debsign ../proteus_0.1.0-1_source.changes`.
3. `dput mentors ../proteus_0.1.0-1_source.changes`.
4. File an ITP bug against `wnpp` (`reportbug wnpp`) noting GPL-3,
   Rust 1.85+ requirement, and the upstream URL.
5. Once accepted, Debian's auto-builders cover amd64 + arm64 (the
   Architectures we list); other ports are best-effort.

Until then, the `dput ppa:kit3713/proteus ...` Launchpad path (below)
is the supported install route for Debian-derivative users.

[1]: https://wiki.debian.org/Teams/RustPackaging

## How to help

Debian / Ubuntu maintainers: please test in a clean `debian:trixie` or
`ubuntu:noble` chroot with `sbuild`, lint with `lintian`, and PR fixes.
Specifically wanted: a sponsor for the ITP, and confirmation that the
`Rules-Requires-Root: no` claim still holds when we add the
`/var/lib/proteus` 0700 directory (it should — `dh_fixperms` reads
the rules-file install commands).

See `wiki/distro-support.md` for the full distro × init × backend matrix.

## Notes for packagers

- The package does **not** enable timers automatically. That is a
  `proteus apply` decision — same policy as the Arch package.
- `/etc/proteus/config.toml` should be flagged `conffile` once Phase D
  ships a stable config schema; for now there is no shipped default config.
- `cargo build --release --frozen` requires `Cargo.lock` to be present and
  in sync with `Cargo.toml`. Proteus commits `Cargo.lock`.
- The release profile in `Cargo.toml` already does `strip = true`, so
  `dh_strip` has nothing to do. Override or skip it if it complains.
- `debian/compat` is included for older debhelper; modern systems use
  `debhelper-compat (= 13)` from `Build-Depends` and ignore the file.
