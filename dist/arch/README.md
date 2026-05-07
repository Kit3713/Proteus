# Arch Linux package

A `PKGBUILD` for building Proteus as an Arch Linux package.

Proteus targets Fedora 43+ as its primary platform, but Arch is a supported
secondary target — same systemd, same NetworkManager, same BlueZ. The only
real difference is packaging.

## Build and install locally

From the directory containing the `PKGBUILD`:

    makepkg -si

`-s` installs missing build dependencies, `-i` installs the resulting package
with `pacman -U`. The first build will fetch the source tarball from
`v$pkgver` on GitHub.

For a build without installing:

    makepkg

The package lands in the current directory as
`proteus-$pkgver-$pkgrel-$arch.pkg.tar.zst`.

## AUR

Not published yet. Once Proteus tags `v0.1.0` with a real release tarball:

1. The maintainer line, `pkgver`, and `sha256sums` get filled in.
2. The PKGBUILD is pushed to the AUR as `proteus`.
3. Users install with an AUR helper: `paru -S proteus` or
   `yay -S proteus`.

A `-git` variant tracking `main` may follow if there's demand.

## Dependencies

Hard:

- `networkmanager` — Proteus talks to NM via dbus
- `systemd` — timers, boot oneshot, journald logging
- `glibc` — runtime

Optional:

- `bluez` — Bluetooth adapter alias and BLE RPA mode
- `nftables` — discovery silencing rules (Phase E)
- `firewalld` — alternative path for the same rules
- `polkit` — lets a future GUI elevate via pkexec

Build:

- `rust` and `cargo` — Rust 1.85+ (edition 2024)

## Where files land

| File                                                  | Path                                                       |
| ----------------------------------------------------- | ---------------------------------------------------------- |
| Binary                                                | `/usr/bin/proteus`                                         |
| Man page                                              | `/usr/share/man/man1/proteus.1`                            |
| Bash completion                                       | `/usr/share/bash-completion/completions/proteus`           |
| Zsh completion                                        | `/usr/share/zsh/site-functions/_proteus`                   |
| Fish completion                                       | `/usr/share/fish/vendor_completions.d/proteus.fish`        |
| systemd units                                         | `/usr/lib/systemd/system/proteus-*.{service,timer}`        |
| NM dispatcher hook                                    | `/etc/NetworkManager/dispatcher.d/01-proteus`              |
| polkit policy                                         | `/usr/share/polkit-1/actions/com.kit3713.proteus.policy`   |
| License                                               | `/usr/share/licenses/proteus/LICENSE`                      |

`/etc/proteus/config.toml` is listed in `backup=()` so pacman preserves
local edits across upgrades. The state file at `/var/lib/proteus/state.json`
is created on first run, not by the package.

## Post-install

The package does not enable timers automatically — that's a `proteus apply`
decision. After install:

    sudo proteus apply --yes
    sudo systemctl enable --now proteus-rotate.timer proteus-check.timer proteus-boot.service

See `proteus wiki quickstart` once the binary is in place.

## Uninstall

    sudo pacman -R proteus

This removes everything except `/etc/proteus/` (kept by `backup=()`) and
`/var/lib/proteus/`. To purge state and config too:

    sudo proteus uninstall --purge --yes

(once Phase G lands; for now, `rm -rf` the directories yourself.)

## Notes for packagers

- `pkgver` matches `Cargo.toml`'s `version`.
- `sha256sums=('SKIP')` is a placeholder. The first real release will
  populate it; do not ship `SKIP` to the AUR.
- `cargo build --release --frozen` requires `Cargo.lock` to be present and
  in sync — Proteus commits `Cargo.lock`, so this is fine.
- The release profile in `Cargo.toml` already does `strip = true`, so no
  explicit `strip` call is needed in `build()`.
- No `check()` function: the integration tests need privileged systemd
  containers (Phase G) and don't fit the standard `cargo test` mold.
