# Proteus systemd units

Proteus has no daemon. These units are how the CLI gets called on a schedule
and at boot.

## Units

- `proteus-rotate.timer` / `proteus-rotate.service` — scheduled MAC rotation,
  *roughly* every 2h by default. Tunable via `[rotation] interval` in
  `/etc/proteus/config.toml`. `Persistent=true` so a suspended laptop catches
  up on the next wake. The default `AccuracySec=45min` +
  `RandomizedDelaySec=30min` is intentional — see issue #303 and
  `wiki/threat-model.md` ("Rotation cadence as a fingerprint"). The 2h
  cadence is preserved on average; only the cross-host wallclock cluster is
  removed.
- `proteus-check.timer` / `proteus-check.service` — probe-driven rotation
  check, *roughly* every 5 min by default (`AccuracySec=2min` +
  `RandomizedDelaySec=2min` for the same anti-fingerprint reason). If the
  probe quorum fails, rotate immediately. The probe-conditional logic lands
  in phase C; until then the service invokes
  `proteus rotate --if-needed --yes`, which is a no-op until the quorum +
  cooldown logic is wired up.
- `proteus-boot.service` — runs `proteus apply --yes` once after
  NetworkManager comes up at boot.

All five services (rotate, check, resume, boot, events) run as root with a
shared strict hardening profile (issue #228): `ProtectSystem=strict`,
`ProtectHome=true`, `PrivateTmp=true`, `PrivateDevices=true`,
`NoNewPrivileges=true`, the full `Protect*` family
(`ProtectKernelTunables`, `ProtectKernelModules`, `ProtectKernelLogs`,
`ProtectClock`, `ProtectControlGroups`, `ProtectHostname`),
`RestrictAddressFamilies=AF_UNIX AF_NETLINK AF_INET AF_INET6`,
`RestrictNamespaces=true`, `RestrictRealtime=true`,
`LockPersonality=true`, `MemoryDenyWriteExecute=true`,
`SystemCallArchitectures=native`, a `CapabilityBoundingSet` of
`CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE`, and a
`SystemCallFilter` that allows `@system-service` minus the dangerous
sets (`@privileged @resources @obsolete @cpu-emulation @debug @raw-io
@reboot @swap @mount @module @clock`) with `SystemCallErrorNumber=EPERM`.
Per-unit `ReadWritePaths=` carve out the directories each workload needs
(`/var/lib/proteus` for state, plus the `/etc/sysctl.d`,
`/etc/systemd/{system,resolved.conf.d,timesyncd.conf.d}`,
`/etc/NetworkManager` drop-in roots for the boot orchestrator).
Output goes to the journal under `SyslogIdentifier=proteus`.

## Install

    cp dist/systemd/*.{timer,service} /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable --now proteus-rotate.timer proteus-check.timer proteus-boot.service

The Proteus binary is expected at `/usr/local/bin/proteus`. If you installed
it elsewhere, edit the `ExecStart=` lines or add a drop-in under
`/etc/systemd/system/proteus-*.service.d/`.

## Tuning

See `proteus wiki rotation` for the full set of knobs (interval, probe
targets, cooldown, captive-portal interaction). The systemd timers only
control *when* Proteus is invoked; everything else is read from
`/etc/proteus/config.toml` at runtime.

## Uninstall

    systemctl disable --now proteus-rotate.timer proteus-check.timer proteus-boot.service
    rm -f /etc/systemd/system/proteus-rotate.{timer,service} \
          /etc/systemd/system/proteus-check.{timer,service} \
          /etc/systemd/system/proteus-boot.service
    systemctl daemon-reload

`proteus uninstall` (phase G) does the same thing.
