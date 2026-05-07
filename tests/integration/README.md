# Proteus integration tests

Privileged Podman + systemd container that exercises the Proteus CLI
against a real (but minimal) Fedora 43 stack. Scoped to the network-layer
fingerprint surface Proteus owns: NetworkManager presence, BlueZ presence,
sysctl reachability, systemd timers, doctor output shape, and the apply /
revert orchestrator path.

`docs/PLAN.md` phase G calls this out:
> Integration tests in a privileged Podman + systemd container with
> stubbed NM and BlueZ.

## Why a container

- Catches Bluetooth / NM / sysctl regressions that `cargo test` alone
  cannot — those require a live system bus, real systemd, real
  `/sys/class/net`, real `nft` etc.
- Reproducible: every run starts from the same Fedora 43 image, so a
  passing test on your laptop matches a passing test in CI matches a
  passing test on the next contributor's box.
- Idempotent: the driver wipes any stale `proteus-it` container before
  starting and removes its own on exit.

## Running locally

```sh
# From the repo root:
tests/integration/run.sh
```

Prerequisites:

- `podman` >= 4 (Fedora 43 ships with 5.x)
- `cargo` (any stable toolchain that builds Proteus)
- The host kernel must support cgroup v2 (every Fedora 38+ does)
- ~1 GB free for the image layer

The driver builds `target/release/proteus`, copies the binary into the
container image at build time, then runs each `scenarios/*.sh` script
inside via `podman exec`.

Exit code: `0` if every scenario passes, `1` otherwise. The driver
prints a one-line summary at the end.

You do not need root on the host. Podman runs the container privileged
(needed for systemd as PID 1); commands inside the container run as root.

## What gets tested

Each scenario script is independent and prints `ok` / `FAIL` lines for
each individual check, so a failure in one assertion does not mask
others in the same scenario.

| Scenario              | Asserts                                                                |
|-----------------------|------------------------------------------------------------------------|
| `01-read-only.sh`     | `doctor`, `status`, `current`, `original`, `show-config`,              |
|                       | `show-defaults`, `wiki`, `wiki intro` all exit 0                       |
| `02-stub-mutators.sh` | `apply`, `revert`, `uninstall`, `reset`, `config set/enable/...` all   |
|                       | reject when `--yes` is missing (rc=64 or 66)                           |
| `03-config-cli.sh`    | `config show`, `keys`, `validate`, `get` all exit 0 with no `/etc`     |
|                       | config file present                                                    |
| `04-timer-cli.sh`     | `timer status`, `list` work; short-name → unit mapping is correct      |
|                       | (`rotate` → `proteus-rotate.timer`, etc.)                              |
| `05-doctor.sh`        | `doctor --json` produces parseable JSON with the documented top-level  |
|                       | fields and check categories (system, daemons, files, runtime)          |

In addition the driver itself runs three orchestrator-level smokes after
the scenario suite:

- `proteus apply --yes` — runs end-to-end across enabled components.
  Most components skip in the container (no Wi-Fi, no Bluetooth
  hardware), which is fine — the driver only fails on hard parse /
  permission errors (rc >= 64).
- `proteus revert --yes` — must always exit 0 (revert is the
  "I tinkered and broke it" hatch).
- `proteus rotate --iface lo --yes` — must fail cleanly (rc < 64); `lo`
  is not NM-managed, so the path through the rotator runs but produces
  no rotation.

## Adding a new scenario

1. Drop a new POSIX shell script in `tests/integration/scenarios/`.
   Name it `NN-short-name.sh` where `NN` controls execution order
   (e.g. `06-dhcp.sh`).
2. Make it executable: `chmod +x tests/integration/scenarios/NN-...sh`.
3. Source the shared helpers and initialise the failure counter:

   ```sh
   #!/bin/sh
   set -u
   . "$(dirname "$0")/lib.sh"
   FAILED=0

   printf 'scenario: NN-short-name\n'
   expect_zero "describe the check" proteus some-command
   finish NN-short-name
   ```

4. Use the helpers from `lib.sh`:
   - `expect_zero <desc> <cmd...>` — asserts rc=0
   - `expect_rc_in <desc> <code>... -- <cmd...>` — asserts rc is one of
     the listed codes, e.g. `expect_rc_in "no --yes" 64 66 -- proteus apply`
   - `expect_contains <desc> <needle> <cmd...>` — asserts stdout
     contains the literal needle
   - `finish <name>` — prints summary and returns 0/1
5. Re-run `tests/integration/run.sh`. The driver auto-discovers every
   `NN-*.sh` and runs them in lexical order. `lib.sh` is skipped.

POSIX shell rules apply (the `Containerfile` doesn't install `bash` for
scenarios; `dash` is the de facto shell). No bashisms, no arrays, no
`local`. Use `command -v` not `which`. `set -u` is encouraged. `set -e`
inside scenarios tends to skip the rest of your assertions on the first
fail, which hides regressions — prefer the `expect_*` helper pattern.
The proteus binary lives at `/usr/local/bin/proteus` inside the container.

## CI integration

This driver is not currently wired into `.github/workflows/`. Container
tests are slow (1-2 min for the image layer cache miss, 15-30s warm) and
should not block PR feedback. A follow-up PR will add a workflow that
runs on push to `main` and on `workflow_dispatch`.

## Cleanup

`run.sh` removes the `proteus-it` container on exit (success or
failure, including SIGINT). To wipe the cached image:

```sh
podman rmi proteus-it
```

The test driver also removes the staged `tests/integration/proteus.bin`
binary on exit.
