{ config, lib, pkgs, ... }:

let
  cfg = config.services.proteus;

  # The TOML format generator ships in nixpkgs. It writes a config file
  # to the nix store so we can copy it (not symlink) into /etc/proteus,
  # matching the layout install.sh produces.
  tomlFormat = pkgs.formats.toml { };
  configFile = tomlFormat.generate "proteus-config.toml" cfg.config;

  # Hardening profile mirrored from dist/systemd/*.service so a NixOS
  # install gets the same security posture as the install.sh path.
  # Issue #228: strict shape unified across all units.
  hardening = {
    Type = "oneshot";
    User = "root";
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    PrivateDevices = true;
    NoNewPrivileges = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectClock = true;
    ProtectControlGroups = true;
    ProtectHostname = true;
    RestrictAddressFamilies = [ "AF_UNIX" "AF_NETLINK" "AF_INET" "AF_INET6" ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    SystemCallArchitectures = "native";
    CapabilityBoundingSet = [ "CAP_NET_ADMIN" "CAP_NET_RAW" "CAP_NET_BIND_SERVICE" ];
    AmbientCapabilities = [ "CAP_NET_ADMIN" "CAP_NET_RAW" ];
    # Default to state-dir + read-only config; the boot orchestrator
    # extends this with the additional /etc drop-in roots it writes to.
    ReadWritePaths = [ "/var/lib/proteus" ];
    ReadOnlyPaths = [ "/etc/proteus" ];
    SystemCallFilter = [
      "@system-service"
      "~@privileged @resources @obsolete @cpu-emulation @debug @raw-io @reboot @swap @mount @module @clock"
    ];
    SystemCallErrorNumber = "EPERM";
    StandardOutput = "journal";
    StandardError = "journal";
    SyslogIdentifier = "proteus";
  };

  # All four units share the "wait for NetworkManager + network-online"
  # ordering — connection profile mutations need a live NM daemon.
  nmAfter = [ "NetworkManager.service" "network-online.target" ];

  # Build a hardened oneshot that invokes `proteus <args>`.
  mkProteusService = { description, args, extraServiceConfig ? { } }: {
    inherit description;
    after = nmAfter;
    wants = [ "NetworkManager.service" ];
    serviceConfig = hardening // {
      ExecStart = "${lib.getExe cfg.package} ${args}";
    } // extraServiceConfig;
  };
in
{
  options.services.proteus = {
    enable = lib.mkEnableOption "Proteus network identifier rotation";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage ./package.nix { }";
      description = "The proteus package to use.";
    };

    config = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = lib.literalExpression ''
        {
          mac.rotation_interval = "2h";
          dns.strip_edns_client_subnet = true;
        }
      '';
      description = ''
        Proteus configuration, serialized to /etc/proteus/config.toml.
        See `proteus wiki config` for the full schema.
      '';
    };

    timer.rotate.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable the 2h scheduled MAC rotation timer.";
    };

    timer.check.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable the 5m probe-driven rotation check timer.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    # The binary reads /etc/proteus/config.toml at runtime. Use `source`
    # (a copy) rather than `text` to land at 0644 root:root, matching the
    # install.sh layout the binary expects.
    environment.etc."proteus/config.toml" = {
      source = configFile;
      mode = "0644";
    };

    # State dir holds the cached permanent MAC and original hostname,
    # which are sacred and root-only — hence 0700.
    systemd.tmpfiles.rules = [
      "d /var/lib/proteus 0700 root root - -"
    ];

    systemd.services.proteus-boot = (mkProteusService {
      description = "Proteus boot-time apply";
      args = "apply --yes";
      # `apply` writes drop-ins under sysctl/systemd/NetworkManager
      # subtrees on top of /var/lib/proteus, so widen ReadWritePaths
      # here only.
      extraServiceConfig.ReadWritePaths = [
        "/var/lib/proteus"
        "/etc/sysctl.d"
        "/etc/systemd/system"
        "/etc/systemd/resolved.conf.d"
        "/etc/systemd/timesyncd.conf.d"
        "/etc/NetworkManager"
      ];
    }) // {
      wantedBy = [ "multi-user.target" ];
    };

    systemd.services.proteus-resume = (mkProteusService {
      description = "Proteus rotate on resume from suspend";
      args = "rotate --yes";
      extraServiceConfig.SyslogIdentifier = "proteus-resume";
    }) // {
      after = nmAfter ++ [
        "suspend.target"
        "hibernate.target"
        "hybrid-sleep.target"
        "suspend-then-hibernate.target"
      ];
      wantedBy = [
        "suspend.target"
        "hibernate.target"
        "hybrid-sleep.target"
        "suspend-then-hibernate.target"
      ];
    };

    systemd.services.proteus-rotate = lib.mkIf cfg.timer.rotate.enable
      (mkProteusService {
        description = "Proteus scheduled MAC rotation";
        args = "rotate --yes";
      });

    systemd.timers.proteus-rotate = lib.mkIf cfg.timer.rotate.enable {
      description = "Proteus scheduled MAC rotation (~ every 2h by default, jittered)";
      wantedBy = [ "timers.target" ];
      # Issue #303: widen AccuracySec + add RandomizedDelaySec so the
      # default rotation cadence is not itself a Proteus fingerprint
      # observable across hosts at the WLAN-controller layer. See
      # dist/systemd/proteus-rotate.timer for the long-form rationale.
      timerConfig = {
        OnCalendar = "*-*-* 00/2:00:00";
        Persistent = true;
        AccuracySec = "45min";
        RandomizedDelaySec = "30min";
        Unit = "proteus-rotate.service";
      };
    };

    systemd.services.proteus-check = lib.mkIf cfg.timer.check.enable
      (mkProteusService {
        description = "Proteus probe-driven rotation check";
        args = "rotate --if-needed --yes";
      });

    systemd.timers.proteus-check = lib.mkIf cfg.timer.check.enable {
      description = "Proteus probe-driven rotation check (~ every 5 min by default, jittered)";
      wantedBy = [ "timers.target" ];
      # Issue #303: see proteus-rotate above; same anti-fingerprint
      # rationale, scaled to the 5-min cadence.
      timerConfig = {
        OnCalendar = "*-*-* *:00/5:00";
        AccuracySec = "2min";
        RandomizedDelaySec = "2min";
        Unit = "proteus-check.service";
      };
    };
  };
}
