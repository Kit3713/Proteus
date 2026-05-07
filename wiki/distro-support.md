Proteus targets Fedora 43+ as its primary platform but works on any modern Linux that pairs a supported init system with a supported network backend. This page is the matrix of what's actually supported today, what's wired but not packaged, and what's still pending.

For the day-to-day backend story (NetworkManager vs systemd-networkd vs raw `ip`), see `proteus wiki internals`. For the rotation cadence, see `proteus wiki rotation`. For "is this distro a good fit", read on.

## Init systems

Proteus carries an `init` abstraction (Roadmap Milestone 5) that emits scheduling, boot, and resume-from-suspend hooks in whatever shape your host's init speaks. Run `proteus doctor` to see which init it detected and which ones are even probeable on your machine.

- **systemd** — fully wired, primary target. Schedules become `proteus-<name>.timer` + `.service` pairs under `/etc/systemd/system/`. Resume hooks ride `WantedBy=sleep.target`. Boot hooks ride `WantedBy=multi-user.target`. The shipped units under `dist/systemd/` are the canonical shape.
- **OpenRC** — wired (Alpine, Gentoo, Artix-OpenRC). Schedules become `/etc/periodic/<bucket>/proteus-<name>` shell scripts; the bucket (`15min`/`hourly`/`daily`/`weekly`/`monthly`) is the largest one whose tick still satisfies the requested cadence. Resume + boot hooks land under `/etc/local.d/`.
- **runit** — wired (Void, Artix-Runit). Schedules become a supervised service directory at `/etc/sv/proteus-<name>/run` whose body loops with `sleep`. Resume + boot hooks land under `/etc/runit/core-services/`.
- **SysV-init** — wired (Devuan, Slackware, antiX). Schedules become `/etc/cron.d/proteus-<name>` entries. Resume hooks become pm-utils scripts under `/etc/pm/sleep.d/`. Boot hooks become LSB-headered `/etc/init.d/proteus-<name>` scripts an installer wires into runlevels with `update-rc.d` or `chkconfig`.

`proteus init` itself isn't a CLI surface yet — the abstraction exists so the future install scripts and packaging recipes can call into it without re-implementing per-init scheduling. Today the only consumer is `proteus doctor`'s init matrix.

## Backends

The init layer says *when* to do work; the backend layer says *how* to do it. They're independent — pairing OpenRC with NetworkManager is fine if you've enabled both.

- **NetworkManager** — fully wired, primary target. Default on Fedora, RHEL, openSUSE, Arch, Debian-with-NM, Ubuntu desktop.
- **systemd-networkd** — scaffolded; per-method migration pending. The right pick on minimal systemd installs and most server distros.
- **raw** (`ip` + `iw`) — scaffolded; last-resort fallback for when neither of the above is present.

See `proteus doctor` for which backend it picked, and `proteus wiki internals` for the trait shape.

## Architectures

Roadmap Milestone 5 dropped the RPM-spec `ExclusiveArch` gate. Today's CI cross-compile matrix covers:

- `x86_64-unknown-linux-gnu` — laptops, desktops, x86 servers.
- `aarch64-unknown-linux-gnu` — Pi 4/5, Apple Silicon Linux VMs, ARM servers.
- `armv7-unknown-linux-gnueabihf` — Pi 2/3, ARM Chromebooks.

Other arches (i686, ppc64le, riscv64, s390x) are best-effort: the source is portable Rust with a small libc surface, but the project doesn't gate releases on them.

## Package layouts

The `dist/<distro>/` tree carries the layouts the project ships today.

- **`dist/rpm/`** — RPM spec for Fedora / Copr. Arch-neutral as of Milestone 5; `%check` runs `cargo test --lib`.
- **`dist/debian/`** — Debian / Ubuntu packaging (submission prep, not yet uploaded).
- **`dist/arch/`** — three PKGBUILDs for Arch / AUR: source, `-bin`, and `-git`.
- **`dist/alpine/`** — Alpine APKBUILD (musl + OpenRC). Untested by author.
- **`dist/void/`** — xbps-src template (runit + musl/glibc). Untested by author.
- **`dist/gentoo/`** — EAPI 8 ebuild + metadata.xml.
- **`dist/openrc/`** — OpenRC `initd` + periodic shims, shared by Alpine + Gentoo.
- **`dist/runit/proteus/`** — runit service tree, shared by Void + Artix-Runit.
- **`dist/nix/`** — Nix derivation.
- **`dist/systemd/`** — Canonical systemd unit shapes (rotate timer + service, boot oneshot, resume hook, check timer + service).
- **`dist/networkmanager/`** — NetworkManager dispatcher script and drop-ins.
- **`dist/polkit/`** — Polkit policy for the privileged actions.
- **`dist/man/`** — `proteus(8)` man page.
- **`dist/completions/`** — bash, zsh, fish completion stubs.

## What's landed (recipes)

Roadmap Milestone 5 packaging recipes are now drafted under `dist/`:

- **`dist/alpine/APKBUILD`** + `proteus.post-install` (musl + OpenRC). Untested by author.
- **`dist/openrc/`** — shared OpenRC `initd` + periodic shims used by Alpine + Gentoo.
- **`dist/void/template`** (xbps-src) + **`dist/runit/proteus/`** (runit service tree). Untested by author.
- **`dist/gentoo/proteus-0.1.0.ebuild`** + `metadata.xml` (EAPI 8). Locally validated, not GURU-merged.
- **`dist/arch/PKGBUILD`** (source), **`PKGBUILD-bin`** (release tarball), **`PKGBUILD-git`** (origin/main). AUR submission ready.
- **`dist/rpm/proteus.spec`** — polished with explicit cargo/rust BRs and a `%check` running `cargo test --lib`. Copr submission ready.
- **`dist/debian/`** — `control`, `rules`, `compat`, `copyright`, `changelog`, `source/format`. Submission prep; ITP + sponsor handoff outstanding.

## What's still pending

- **Submission uploads**: the actual `dput`/`copr-cli build`/AUR push for each recipe is the maintainer's call, not blocked on this repo.
- **Distro-test containers**: Alpine + Void recipes have *not* been built in their target chroots by the author. Flagged in each recipe's README.
- **`proteus doctor` distro-compat warnings** for known-quirky setups (Pi-hole, dnscrypt-proxy, openresolv, NetworkManager-l2tp). Some of these already exist in the `Detect-and-defer` section; the rest land alongside the packaging work.
- **`proteus doctor` package-format reporter** (currently reports init/libc/distro/backend, not yet the package manager).

## Caveats

- The init abstraction renders artifacts; it does *not* commit them to disk. That's the install script's job, and the install script lives in `dist/install.sh` (which currently knows only the systemd layout — extending it is a follow-up).
- Resume hooks on OpenRC and runit are best-effort. Neither init has a native suspend-target concept; the artifacts assume an `elogind` sleep.d shim is in place. If your host doesn't ship elogind, the resume hook will not fire.
- Bluetooth features (`proteus bluetooth`) require BlueZ. The init abstraction is orthogonal to BlueZ — same hooks, same scheduling — but the feature itself skips when BlueZ is absent.
- The DNS knob requires systemd-resolved. There is no plan to extend it to other resolvers; the design point is "narrow knob on the one resolver that already exposes the right API". Other distros' default resolvers route through the `Detect-and-defer` section.
