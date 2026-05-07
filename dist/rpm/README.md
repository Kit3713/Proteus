# Fedora / RHEL packaging

An RPM spec and Copr configuration for Proteus.

Fedora 43+ is the **primary** platform per `docs/PLAN.md`. RHEL/EPEL is a
secondary target (see "EPEL plans" below). Arch is the other supported
secondary target — see `dist/arch/`.

## Files

| Path                   | Purpose                                             |
| ---------------------- | --------------------------------------------------- |
| `proteus.spec`         | RPM spec following Fedora packaging guidelines.     |
| `.copr/Makefile`       | Copr "custom" source method entry point.            |
| `proteus.rpkg`         | rpkg config for git-checkout SRPM builds via Copr.  |

## Build the RPM locally

You need `rpm-build`, `rpmdevtools`, and `cargo`/`rust` ≥ 1.85.

```sh
# Set up a standard ~/rpmbuild tree (idempotent).
rpmdev-setuptree

# Drop the spec into the SPECS dir.
cp dist/rpm/proteus.spec ~/rpmbuild/SPECS/

# Fetch (or stage) the source tarball. Until v0.1.0 is tagged on GitHub,
# generate one from the current checkout instead:
git archive --format=tar.gz --prefix=Proteus-0.1.0/ -o ~/rpmbuild/SOURCES/proteus-0.1.0.tar.gz HEAD

# Build a binary RPM (-bb) plus its source RPM (-bs); use -ba for both.
rpmbuild -ba ~/rpmbuild/SPECS/proteus.spec
```

The output lands under `~/rpmbuild/RPMS/<arch>/` and `~/rpmbuild/SRPMS/`.

## Install the resulting RPM

```sh
sudo dnf install ~/rpmbuild/RPMS/x86_64/proteus-0.1.0-1.fc43.x86_64.rpm
```

Same caveats as the Arch package: timers are **not** auto-enabled. The
`%post` scriptlet only runs `%systemd_post` (presets, no-op without an
explicit preset file). Enable manually after reviewing your config:

```sh
sudo proteus apply --yes
sudo systemctl enable --now proteus-rotate.timer proteus-check.timer
```

## Mock build (clean chroot, recommended before publishing)

Mock isolates the build in a fresh Fedora chroot — same thing Copr does.

```sh
sudo dnf install mock
sudo usermod -aG mock "$USER" && newgrp mock

# Build an SRPM first.
rpmbuild -bs ~/rpmbuild/SPECS/proteus.spec

# Then build it in a Fedora 43 chroot.
mock -r fedora-43-x86_64 ~/rpmbuild/SRPMS/proteus-0.1.0-1.fc43.src.rpm

# aarch64 cross-build (slower, qemu-user under the hood):
mock -r fedora-43-aarch64 ~/rpmbuild/SRPMS/proteus-0.1.0-1.fc43.src.rpm
```

Built RPMs land in `/var/lib/mock/fedora-43-x86_64/result/`.

## Copr setup

[Copr](https://copr.fedorainfracloud.org/) is Fedora's community build
service. It builds RPMs from a spec on every commit and serves them via a
generated yum repo.

1. Sign in at https://copr.fedorainfracloud.org/ with a FAS account.
2. Create a new project, e.g. `kit3713/proteus`.
3. Choose chroots: at least `fedora-43-x86_64`, `fedora-43-aarch64`,
   `fedora-rawhide-x86_64`. Add `epel-9-x86_64` once we commit to EPEL
   (see below).
4. Set "Build options" → "Custom" source method:
   - Script: `make -f dist/rpm/.copr/Makefile srpm outdir=$outdir`
   - Builddeps: `rpkg`
   - Resultdir: `dist/rpm`
5. Trigger the first build with the "New build → Custom" button or via
   the Webhook integration on `git push`.

Users then install with:

```sh
sudo dnf copr enable kit3713/proteus
sudo dnf install proteus
```

## ExclusiveArch — why x86_64 + aarch64

Proteus is a laptop-focused tool, but `aarch64` covers two real cases:

- ARM SBCs (Raspberry Pi 4/5, Pine64, etc.) running Fedora as travel /
  hotspot devices — same MAC-rotation and discovery-silencing wants.
- Apple Silicon Macs running Fedora in a VM (Asahi or UTM) — increasingly
  common dev setup.

Other arches (`i686`, `ppc64le`, `s390x`, `riscv64`) aren't tested. Rather
than ship silently broken packages, `ExclusiveArch` makes Copr/mock skip
them with a clear "this arch isn't supported" error. Add an arch when
someone reports a working build there.

## EPEL plans

EPEL 9 / 10 (RHEL 9 / 10) is in scope for a future minor release once:

- Phase A's `cargo build --release --frozen` is verified against the EPEL
  Rust toolchain (currently lags Fedora — confirm `rust >= 1.85`).
- The `systemd-rpm-macros` BR works on EL (it does as of EPEL 9).
- The `Recommends:` weak deps are reviewed — older `dnf` honors them, but
  some EL admins disable weak deps and we should make sure the package is
  still useful without `bluez` / `nftables` / `polkit`.

For now, leaving EL chroots out of Copr keeps the failure surface narrow.

## Validation

Without rpm tooling installed, this is a no-op. Otherwise:

```sh
# Lint (style + common bugs).
rpmlint dist/rpm/proteus.spec

# Parse-only smoke test (catches macro typos and basic syntax errors).
rpmspec --parse dist/rpm/proteus.spec > /dev/null

# Full build, dependency resolution, and chroot install in one shot.
mock -r fedora-43-x86_64 ~/rpmbuild/SRPMS/proteus-0.1.0-1.fc43.src.rpm
```

## Notes for packagers

- `Version:` mirrors `Cargo.toml`'s `version`. Bump in lockstep with
  `proteus.rpkg` and `dist/arch/PKGBUILD`.
- Source0 points at `v$version` on GitHub. Until v0.1.0 is tagged, use
  `git archive` (see "Build the RPM locally") or rely on the Copr custom
  method (which builds from the current git checkout via `rpkg`).
- `%cargo_build` honors Fedora's vendored-crate rules. If the Rust SIG
  ever objects to network access during `%build`, switch to
  `cargo build --release --offline` plus a vendored tarball.
- `Cargo.lock` is committed, so `--frozen` builds are deterministic.
- The release profile in `Cargo.toml` already does `strip = true`; no
  explicit `%{__strip}` call is needed.
- `%check` runs `cargo test --release --lib` (Milestone 5). Integration
  tests need a privileged systemd container (Phase G) and aren't lib
  tests; if Copr hits a flake we haven't reproduced locally, rebuild
  the SRPM with `rpmbuild --without check ...` to skip.
- The NM dispatcher hook is intentionally **not** marked
  `%config(noreplace)`: it's a script that ships with the package, not a
  user config. If the dispatcher logic changes in a new release, RPM
  should overwrite the old version — not silently preserve a stale copy.
  User config lives in `/etc/proteus/config.toml` (created on first run).

## How to help

Fedora / Copr maintainers: please test the spec in `mock -r
fedora-43-x86_64` and `fedora-43-aarch64`, run `rpmlint
dist/rpm/proteus.spec`, and PR fixes. The Copr submission step itself
is the maintainer's call — the spec is ready, the actual upload to
`copr.fedorainfracloud.org/coprs/kit3713/proteus/` lives outside this
repo.

See `wiki/distro-support.md` for the full distro × init × backend
matrix.

## Cross-references

- `dist/arch/PKGBUILD` — Arch package, mirrors install paths.
- `dist/systemd/README.md` — what the timers and services actually do.
- `dist/networkmanager/README.md` — dispatcher hook architecture.
- `dist/polkit/README.md` — polkit policy purpose.
- `docs/PLAN.md` — phase F notes the packaging stubs.
