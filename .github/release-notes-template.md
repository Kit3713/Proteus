# Proteus VERSION

Pre-release. See [CHANGELOG.md](../CHANGELOG.md) for the full list of
changes in this version. See [docs/ROADMAP.md](../docs/ROADMAP.md) for what
has landed and what is on the bench.

## Highlights

- (one-line summary of the most user-visible change)
- (one-line summary of the second most user-visible change)
- (one-line summary of any breaking change, if applicable)

## Artifacts

The release workflow attaches a stripped binary plus a SHA256 sum for each
supported architecture. See [`dist/release.md`](../dist/release.md) for the
supported-architecture matrix.

- `proteus-x86_64-unknown-linux-gnu` (+ `.sha256`)
- `proteus-aarch64-unknown-linux-gnu` (+ `.sha256`)

Verify a download:

```sh
sha256sum -c proteus-x86_64-unknown-linux-gnu.sha256
```

## Install

Until distribution packages are published, the supported install path is:

```sh
git clone --branch vVERSION https://github.com/Kit3713/Proteus.git
cd Proteus
sudo ./install.sh
```

Distribution packaging skeletons live under `dist/` (Arch, RPM, Debian,
Nix). Per-distro publishing is a Phase F follow-up.

## Verify

```sh
proteus doctor
proteus status --json
```

## What is NOT in this release

This release is a pre-release. See the "Known gaps" section in
[CHANGELOG.md](../CHANGELOG.md) for features that are intentionally not yet
implemented. In particular: probe- and timer-driven rotation callbacks,
captive-portal handling, DHCP option suppression, the DNS ECS-strip knob,
the sysctl drop-in, and the `proteus uninstall` / `proteus diff` / `proteus
dry-run` implementations.

## Reporting issues

Use the GitHub issue templates. For security vulnerabilities, follow
[SECURITY.md](../SECURITY.md) and open a private advisory.
