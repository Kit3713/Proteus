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

- **`dist/rpm/`** — RPM spec for Fedora / Copr. Arch-neutral as of Milestone 5.
- **`dist/debian/`** — Debian / Ubuntu packaging.
- **`dist/arch/`** — PKGBUILD for Arch / AUR.
- **`dist/nix/`** — Nix derivation.
- **`dist/systemd/`** — Canonical systemd unit shapes (rotate timer + service, boot oneshot, resume hook, check timer + service).
- **`dist/networkmanager/`** — NetworkManager dispatcher script and drop-ins.
- **`dist/polkit/`** — Polkit policy for the privileged actions.
- **`dist/man/`** — `proteus(8)` man page.
- **`dist/completions/`** — bash, zsh, fish completion stubs.

## What's pending

These are tracked in `docs/ROADMAP.md` Milestone 5 and will land in follow-up PRs:

- **Alpine APKBUILD** (musl + OpenRC). The init module already produces the OpenRC artifacts; the APKBUILD itself isn't drafted yet.
- **Void package recipe** (musl + runit). Same story: artifacts exist, packaging recipe doesn't.
- **Gentoo ebuild**. Gentoo can rebuild from the existing tree; an upstream ebuild would smooth the path for users.
- **AUR submission** of the existing PKGBUILD (binary + `-git` flavors).
- **Copr submission** for the RPM spec — the spec itself is ready, the submission step is the gap.
- **Debian unstable submission**.
- **`proteus doctor` distro-compat warnings** for known-quirky setups (Pi-hole, dnscrypt-proxy, openresolv, NetworkManager-l2tp). Some of these already exist in the `Detect-and-defer` section; the rest land alongside the packaging work.

## Caveats

- The init abstraction renders artifacts; it does *not* commit them to disk. That's the install script's job, and the install script lives in `dist/install.sh` (which currently knows only the systemd layout — extending it is a follow-up).
- Resume hooks on OpenRC and runit are best-effort. Neither init has a native suspend-target concept; the artifacts assume an `elogind` sleep.d shim is in place. If your host doesn't ship elogind, the resume hook will not fire.
- Bluetooth features (`proteus bluetooth`) require BlueZ. The init abstraction is orthogonal to BlueZ — same hooks, same scheduling — but the feature itself skips when BlueZ is absent.
- The DNS knob requires systemd-resolved. There is no plan to extend it to other resolvers; the design point is "narrow knob on the one resolver that already exposes the right API". Other distros' default resolvers route through the `Detect-and-defer` section.
