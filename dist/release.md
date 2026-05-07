# Proteus release artifacts

This document describes which architectures Proteus ships prebuilt binaries
for, why, and how to handle the architectures we don't ship.

## Supported architectures

### x86_64-unknown-linux-gnu (primary)

The main desktop and laptop target. Built natively inside the Fedora 43
container on every CI run and on every `v*` tag. This is the architecture the
project is developed against; PLAN.md and the wiki assume x86_64 unless
otherwise noted.

### aarch64-unknown-linux-gnu (secondary)

Covers 64-bit ARM systems running glibc-based Linux. The intended audience:

- ARM laptops (e.g. Lenovo ThinkPad X13s, Asahi Linux on Apple Silicon
  running a glibc-based distro).
- Single-board computers from Raspberry Pi 4 onward, when running a 64-bit OS
  (Raspberry Pi OS 64-bit, Fedora aarch64, Ubuntu aarch64, Debian arm64).
- AWS Graviton, Ampere, and other ARM64 cloud or server instances.

Built by cross-compiling from `ubuntu-latest` using the
`gcc-aarch64-linux-gnu` toolchain. The release workflow strips the binary
with `aarch64-linux-gnu-strip` and enforces the same 3 MB cap as x86_64.

The aarch64 artifact is not exercised on a real ARM runner in CI — there is
no GitHub-hosted ARM runner available to the project today. The cross-build
catches dependency churn and link-level breakage. Functional verification
on aarch64 still relies on the user reporting issues; the wiki page
`troubleshooting` will note this once Phase F lands.

## Unsupported architectures

### 32-bit ARM (armv7l, armhf)

Not shipped. Proteus assumes:

- 64-bit pointer arithmetic in a few state-tracking paths.
- glibc and tokio behaviors that match the 64-bit profile.
- A memory budget that comfortably exceeds what the smallest armv7 boards
  ship with.

A user with a 32-bit ARM device is welcome to build from source
(`cargo build --release`); the code itself should compile cleanly because the
dependency tree is portable Rust, but the project does not gate CI on armv7
and will not investigate armv7-only regressions. The 3 MB binary cap is not
guaranteed on armv7.

If 32-bit ARM demand becomes loud enough to justify a fourth CI lane, the
plan is to add an `armv7-unknown-linux-gnueabihf` cross-build to ci.yml first
(catch breakage early) and only add release artifacts after one full release
cycle without armv7-specific failures.

### Other architectures (riscv64, ppc64le, s390x, mips, ...)

Out of scope for v1. Same story as armv7: the code may build, but there is
no CI lane and no artifact. Users on these architectures should build from
source and report issues without expecting binary releases.

### musl targets (`*-linux-musl`)

Not shipped today. The release artifacts link against the glibc that ships
with Fedora 43 (x86_64) and Ubuntu's aarch64 sysroot. If a user needs a
fully static binary for Alpine or a minimal container, building from source
with `--target x86_64-unknown-linux-musl` is the path. A musl release lane
may be added later if there is demand.

## Dependency cross-compile notes

The dependency tree is mostly pure Rust and cross-compiles cleanly:

- `zbus` — pure Rust, async D-Bus client.
- `tokio` — pure Rust runtime.
- `rand` / `getrandom` — pure Rust; uses kernel syscalls on Linux.
- `serde`, `serde_json`, `toml`, `toml_edit` — pure Rust.
- `clap` — pure Rust (with the `default-features = false` set used here).
- `tracing`, `tracing-subscriber`, `tracing-journald` — pure Rust;
  `tracing-journald` writes to journald via the local socket and does not
  link against libsystemd.
- `include_dir` — build-time only; the binary itself has no extra runtime
  dependency.
- `anyhow`, `thiserror` — pure Rust.

If any future dependency pulls in a `cc`-driven native build (a `build.rs`
that compiles C), the cross-compile job will need the matching cross-gcc
package. The aarch64 lane already installs `gcc-aarch64-linux-gnu`; further
arches would need their own `gcc-<arch>-linux-gnu` package alongside.

If a dependency ever fails to cross-compile, the immediate options are:

1. Pin to an older version that does cross-compile.
2. Disable a feature flag that triggers the C build.
3. Drop the dependency.

The 3 MB binary cap and the no-network-egress invariant mean we rarely add
heavyweight deps anyway, so this is unlikely to bite often.

## Distro packages produced on tag

Tagging `v*` triggers `.github/workflows/release.yml`, which builds both raw
binaries above plus installable packages from the recipes under `dist/`:

| Format          | Recipe                  | Container             | Artifact pattern             |
| --------------- | ----------------------- | --------------------- | ---------------------------- |
| Fedora/RHEL RPM | `dist/rpm/proteus.spec` | `fedora:43`           | `proteus-*.rpm`              |
| Debian/Ubuntu   | `dist/debian/`          | `ubuntu:24.04`        | `proteus_*.deb`              |
| Arch Linux      | `dist/arch/PKGBUILD`    | `archlinux:base-devel`| `proteus-*.pkg.tar.zst`      |

Each artifact ships alongside a `.sha256` companion. The package jobs are
non-blocking (`continue-on-error: true`): if a single recipe drifts, the
release still ships every other format and the raw binaries.

### Install commands

```sh
# Fedora / RHEL / openSUSE
sudo dnf install ./proteus-*.rpm

# Debian / Ubuntu
sudo dpkg -i ./proteus_*.deb
sudo apt-get install -f   # pull any missing runtime deps

# Arch Linux
sudo pacman -U ./proteus-*.pkg.tar.zst
```

### Nix

Nix users do not get a release tarball: install directly from the flake
under `dist/nix/`:

```sh
nix profile install github:Kit3713/Proteus?dir=dist/nix
# or, for a one-off invocation:
nix run github:Kit3713/Proteus?dir=dist/nix
```

This way every `nix build` follows whatever commit the user pins, instead
of a separately published artifact. See `dist/nix/README.md` for the
NixOS module.

## How a release is cut

1. Land all changes on `main` and confirm CI is green, including the
   `cross-build aarch64` lane.
2. Tag the commit: `git tag -s v0.1.0 -m "Proteus 0.1.0"` (signed tags
   preferred; unsigned tags also trigger the workflow).
3. Push the tag: `git push origin v0.1.0`.
4. The `Release` workflow builds both architectures, builds RPM + .deb +
   Arch packages, computes SHA256 sums, and creates a draft GitHub Release
   with everything attached.
5. Review the draft release notes, confirm each expected artifact is
   present (raw binaries are required; package artifacts are best-effort
   and may be missing if a recipe drifted), edit if needed, and publish.

The workflow leaves the release as a draft on purpose so a human can
double-check the binary sizes, the package contents, and the
auto-generated changelog before the artifact is announced.
