{
  description = "Proteus — erases the network identifiers your Linux laptop hands out";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      # Linux only — Proteus targets systemd + NetworkManager.
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems
        (system: f (import nixpkgs { inherit system; }));
    in
    {
      packages = forAllSystems (pkgs: rec {
        proteus = pkgs.callPackage ./package.nix { };
        default = proteus;
      });

      # `nix run .#proteus` for ad-hoc invocation without a system install.
      apps = forAllSystems (pkgs: rec {
        proteus = {
          type = "app";
          program = "${pkgs.callPackage ./package.nix { }}/bin/proteus";
        };
        default = proteus;
      });

      nixosModules = rec {
        proteus = ./module.nix;
        default = proteus;
      };

      # Surface common dev tooling for contributors building from the flake.
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy ];
        };
      });
    };
}
