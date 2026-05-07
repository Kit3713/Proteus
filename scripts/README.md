# scripts/

Developer scripts for working on Proteus. None of these are shipped to users — see [`install.sh`](../install.sh) for the user-facing installer and [`dist/`](../dist/) for packaged artifacts.

## check.sh

Local pre-push checker. Runs the same checks as [`.github/workflows/ci.yml`](../.github/workflows/ci.yml), in CI order, fail-fast:

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `cargo build --release`
5. `strip target/release/proteus` (Linux only)
6. Binary size check (≤ 4,000,000 bytes — matches the release.yml hard cap)
7. `bash -n` on `install.sh` and `uninstall.sh` (if present)
8. `groff -ww -man` lint on `dist/man/proteus.1` (if present and `groff` installed)

Exit code: `0` on success, `1` on first failure.

### Usage

```sh
./scripts/check.sh              # full pipeline (matches CI)
./scripts/check.sh --no-build   # skip build, size, shell, man (faster iteration)
./scripts/check.sh --quick      # alias for --no-build
./scripts/check.sh --help       # usage
```

Run before every push to catch CI failures locally — it's faster than waiting for the runner.

POSIX shell. Runs under `dash` as well as `bash`.

## verify-build.sh

End-user / auditor script: rebuilds Proteus from a tagged release and compares the resulting binary's `sha256` against the published `.sha256` from the GitHub release. Used to confirm a downloaded binary was built from the public source.

```sh
./scripts/verify-build.sh v0.1.0-alpha
```

Exit code:

- `0` — local rebuild matches the published artifact
- `1` — mismatch (binary differs)
- `2` — usage / missing prerequisite
- `3` — clone, download, or build failure

Runs under `dash` and `bash`. No new crate dependencies; just `git`, `curl`, `sha256sum`, `cargo`, and (recommended) `rustup` so the pinned toolchain auto-installs from `rust-toolchain.toml`.

See [`wiki/reproducible-builds.md`](../wiki/reproducible-builds.md) for the design notes and the list of known caveats (glibc pinning, container layer digests).
