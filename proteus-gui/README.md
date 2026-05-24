# proteus-gui

Desktop GUI for [Proteus](../README.md) — a Qt6 Widgets shell that is a **thin
client over the `proteus` CLI**.  No Rust code is linked into this binary.

> **Skeleton status (roadmap 1.4.0–1.4.2).**  This directory compiles once Qt6
> development packages are installed.  See "Building" below.  The pages are
> scaffolded; interactive behaviour (threading, live event feed, single-instance
> lock, etc.) is deferred to 1.4.1–1.4.2 and marked with `TODO` comments in
> source.

---

## Architecture

```
proteus-gui (Qt6 Widgets, C++17)
    │
    ├── reads  →  proteus … --json       (QProcess, no elevation)
    └── writes →  pkexec proteus … --yes (QProcess + PolicyKit)
```

**GUI/Qt deps never enter the Rust binary.**  The GUI is a separate executable.
It calls the installed `proteus` binary via `QProcess::setArguments` (never
shell-interpolated).

### Local JSON helper (temporary)

`src/client/ProteusRunner.{h,cpp}` and `src/client/PkexecRunner.{h,cpp}` are
**placeholder** wrappers (`TODO: replace with proteus-client once available`).
Once the `proteus-client` shared-library unit merges these files will be
replaced by a `#include <proteus-client/…>` thin wrapper.  No page code needs
to change — the public API is identical.

---

## Pages

| Page | CLI reads | CLI mutations (pkexec) |
|---|---|---|
| Status dashboard | `proteus status --json` | none |
| Personas | `proteus persona list/show --json` | `persona use/random/clear --yes` |
| Per-SSID rules | `proteus ssid list/show --json` | `ssid set/clear --yes` |
| Doctor / tools | `proteus doctor/diff/dry-run --json` | `apply/revert --yes` |

The GUI surface is a **faithful mirror of the CLI surface** — no VPN/Tor/DNS/
traffic features will be added here.

---

## Building

### Fedora 43+ (primary target)

```sh
sudo dnf install \
    qt6-qtbase-devel \
    kf6-kirigami-devel \   # optional — graceful fallback if absent
    cmake \
    gcc-c++

cmake -S proteus-gui -B build/gui -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build build/gui -j$(nproc)
```

### Kirigami fallback

When `kf6-kirigami-devel` is absent, CMake prints:

```
proteus-gui: Kirigami NOT found — building Qt Widgets shell (fully supported)
```

The Qt Widgets shell is the production path until the Kirigami QML shell is
complete (roadmap 1.4.2).  Both paths compile to the same page topology.

---

## Version-pin policy

The installed `proteus-gui` RPM will declare:

```
Requires: proteus-tray = %{version}
```

so the GUI, tray daemon, and CLI all move in lock-step.  The JSON shapes the
CLI emits are NOT versioned independently of the binary — never relax this pin
without an explicit API-stability review.

The RPM spec (`dist/rpm/proteus-gui.spec`) is a future deliverable (1.4.2).

---

## Roadmap deferred items (TODOs in source)

| Milestone | Item |
|---|---|
| 1.4.1 | Single-instance lock (QLocalServer) |
| 1.4.1 | Threading — run PkexecRunner in QThread with progress indicator |
| 1.4.1 | Live event feed (connect to proteus events daemon IPC/D-Bus) |
| 1.4.1 | Persona detail structured form (not raw JSON) |
| 1.4.2 | Kirigami QML shell replacing Qt Widgets sidebar |
| 1.4.2 | RPM spec (`dist/rpm/proteus-gui.spec`) |
| 1.4.2 | App icon (`resources/proteus.svg`) |
| 1.4.2 | `persona new / edit / import / export` surfaces |
