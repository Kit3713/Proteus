# Proteus on NixOS

Nix flake exposing the Proteus package and a NixOS module. The module
installs the binary, writes `/etc/proteus/config.toml` from your Nix
config, and enables the same systemd timers as the `install.sh` path.

## Quick start

Add Proteus to your flake inputs:

```nix
{
  inputs.proteus.url = "github:Kit3713/Proteus?dir=dist/nix";
}
```

Import the module and turn it on in your NixOS configuration:

```nix
{ inputs, ... }:
{
  imports = [ inputs.proteus.nixosModules.default ];

  services.proteus = {
    enable = true;
    config.mac.rotation_interval = "2h";
    config.dns.strip_edns_client_subnet = true;
  };
}
```

Rebuild with `sudo nixos-rebuild switch` and the binary lands on PATH,
the systemd timers come up, and `/etc/proteus/config.toml` matches your
Nix config.

## Module options

| Option                              | Default | Description                                              |
| ----------------------------------- | ------- | -------------------------------------------------------- |
| `services.proteus.enable`           | `false` | Master switch.                                           |
| `services.proteus.package`          | (auto)  | Override the proteus package.                            |
| `services.proteus.config`           | `{ }`   | Attrs serialized to `/etc/proteus/config.toml`.          |
| `services.proteus.timer.rotate.enable` | `true`  | Enable the 2h scheduled rotation timer.               |
| `services.proteus.timer.check.enable`  | `true`  | Enable the 5m probe-driven check timer.               |

The full config schema lives in `proteus wiki config` (phase F). Anything
the binary accepts in TOML, you can write here as a Nix attrset.

## Ad-hoc usage

Run without installing system-wide:

```sh
nix run github:Kit3713/Proteus?dir=dist/nix -- status
```

Or build the package on its own:

```sh
nix build github:Kit3713/Proteus?dir=dist/nix
./result/bin/proteus --help
```

## Development shell

```sh
nix develop github:Kit3713/Proteus?dir=dist/nix
```

Drops you into a shell with `cargo`, `rustc`, `rustfmt`, and `clippy`.

## What the module does, exactly

- Adds `pkgs.proteus` to `environment.systemPackages`.
- Writes `/etc/proteus/config.toml` (mode `0644`, root-owned) from
  `services.proteus.config`.
- Creates `/var/lib/proteus` (mode `0700`) via `systemd.tmpfiles`.
- Enables `proteus-boot.service` (runs `proteus apply --yes` on boot
  after NetworkManager) and `proteus-resume.service` (rotates on wake).
- Conditionally enables `proteus-rotate.timer` (2h) and
  `proteus-check.timer` (5m), each with the matching service.
- All services run as root with the strict shared hardening profile
  mirrored from `dist/systemd/` (issue #228): `ProtectSystem=strict`,
  the full `Protect*` family (Home, KernelTunables, KernelModules,
  KernelLogs, Clock, ControlGroups, Hostname), `PrivateTmp=true`,
  `PrivateDevices=true`, `RestrictNamespaces=true`,
  `RestrictRealtime=true`, `LockPersonality=true`,
  `MemoryDenyWriteExecute=true`, `SystemCallArchitectures=native`,
  capability bounding set limited to `CAP_NET_ADMIN CAP_NET_RAW
  CAP_NET_BIND_SERVICE`, and `SystemCallFilter=@system-service` minus
  the dangerous sets (`@privileged @resources @obsolete @cpu-emulation
  @debug @raw-io @reboot @swap @mount @module @clock`). Per-unit
  `ReadWritePaths=` carve out `/var/lib/proteus`; `proteus-boot`
  additionally allows the `/etc/sysctl.d`, `/etc/systemd/{system,
  resolved.conf.d, timesyncd.conf.d}`, and `/etc/NetworkManager`
  drop-in roots that `proteus apply` writes to.

## Caveats

- Linux only. The flake declares `x86_64-linux` and `aarch64-linux`
  systems. macOS and other non-systemd platforms are out of scope.
- The package's `cargoLock.lockFile = ../../Cargo.lock` keeps builds
  reproducible without a vendored hash that drifts every release.
- `doCheck = false`: many Proteus tests need netlink, systemd, or
  NetworkManager state and only run in CI containers, not in the Nix
  build sandbox.
- SELinux contexts (`semanage fcontext`) and the polkit policy from
  `dist/polkit/` are not applied by this module — NixOS uses its own
  security model. If you mix Nix and SELinux policy, set the contexts
  outside this module.
