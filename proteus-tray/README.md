# proteus-tray

`QSystemTrayIcon` (StatusNotifierItem) daemon for Proteus.  A thin Qt6
front-end over the `proteus` CLI.

## Role

```
[Desktop session]
      |
      v
proteus-tray  (this package)
      |
      ├── READ:   proteus status --json         (via proteus-client)
      ├── WRITE:  pkexec proteus rotate --yes   (via proteus-client)
      └── POPUP:  PopupWindow (normal top-level window, cross-DE)
```

No network logic, no VPN/Tor/DNS.  Everything is delegated to the CLI.

## Features (skeleton)

- Registers a `QSystemTrayIcon`; works on KDE, Xfce, and GNOME with the
  AppIndicator extension.
- Tooltip reflects current applied/partial/reverted state from
  `proteus status --json`.
- Context menu: **Rotate** / **Kill switch** / **Open Proteus** (hidden if
  `proteus-gui` not in PATH) / **Quit**.
- Left-click opens a small top-level **PopupWindow** (placeholder content;
  full layout is roadmap 1.3.x).
- XDG autostart `.desktop` file installs to `/etc/xdg/autostart/`.

## Fedora build dependencies

```
sudo dnf install qt6-qtbase-devel qt6-qtwidgets-devel cmake gcc-c++
```

`proteus-client` must be built (or installed) first:

```bash
# From the monorepo root:
cmake -S proteus-client -B build/client && cmake --build build/client
cmake -S proteus-tray   -B build/tray   && cmake --build build/tray
```

Or build both at once via the top-level CMake if a root `CMakeLists.txt`
aggregates them.

## Building

```bash
# Configure (will pull in proteus-client from the sibling directory)
cmake -S proteus-tray -B build/tray

# Build
cmake --build build/tray

# Install (to /usr/local by default)
cmake --install build/tray
```

## Version-pin policy

This package depends on `proteus-client`, which in turn declares
`Requires: proteus = %{version}`.  All three packages must be at the same
version.  See `proteus-client/README.md` for rationale.

## Architecture constraints

- The `proteus` Rust binary is **never modified** by this package.
- All mutations go through `pkexec` — the tray process is unprivileged.
- No features beyond the CLI surface: no VPN, Tor, DNS, SSH.
- Cross-DE normal window (no Wayland layer-shell protocol required).
