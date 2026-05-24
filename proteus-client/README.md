# proteus-client

C++ Qt6 library that wraps the `proteus` CLI for use by Qt front-ends
(`proteus-tray`, `proteus-gui`).

## Role

```
[proteus-tray / proteus-gui]
         |
         v
  [proteus-client]   <-- this library
         |
    READ: proteus <subcommand> --json          (QProcess, stdout, JSON)
   WRITE: pkexec proteus <subcommand> --yes    (polkit privilege elevation)
  EVENTS: journald tail stub (sd_journal, TODO)
```

Qt GUI deps **never enter** the `proteus` Rust binary.  Front-ends are thin
clients that read state via `--json` and mutate state via `pkexec --yes`.

## Components

| File | Purpose |
|------|---------|
| `ProteusRunner.{h,cpp}` | Read-only QProcess wrapper; runs `proteus … --json`, parses `QJsonDocument` |
| `ProteusMutate.{h,cpp}` | Build + launch `pkexec proteus … --yes` invocations |
| `JournaldTail.{h,cpp}` | **Stub** — interface for streaming live events from journald (TODO impl) |

## Fedora build dependencies

```
sudo dnf install qt6-qtbase-devel qt6-qtbase-devel-tools cmake gcc-c++
```

(`qt6-qtbase-devel` includes `QtConcurrent`, which is used by `ProteusRunner::queryAsync`
to run CLI calls off the GUI thread.)

For the journald-tail real implementation (not yet wired):

```
sudo dnf install systemd-devel pkgconfig
```

## Building

```bash
cmake -S proteus-client -B build/client
cmake --build build/client
```

## Version-pin policy

The `proteus-client` library version tracks the `proteus` CLI version it
targets.  Specifically, the Fedora sub-package (`proteus-client`) declares:

```
Requires: proteus = %{version}
```

so the CLI and this library are always upgraded together.  **Never ship a
`proteus-client` package against a different CLI version** — the JSON shapes
are not guaranteed stable across minor CLI releases during the 1.x beta
cycle.

This policy is enforced at the RPM spec level.  Do not relax it until the
CLI JSON output is declared stable (roadmap 2.0.0).

## Architecture constraints

- This library is **read-side + dispatch only**.  It adds zero network
  features beyond what the CLI exposes.  No VPN, Tor, DNS, or SSH surface.
- The `proteus` Rust binary is never modified by this library.  It remains
  the single source of truth for all system state.
- `pkexec` is the only privilege-escalation mechanism.  No setuid, no
  capabilities.
