{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "proteus";
  version = "0.1.0";

  # The crate root is two levels up from this file (dist/nix/ → repo root).
  src = lib.cleanSource ../..;

  # Pin via the in-tree lockfile so the derivation is reproducible without
  # baking in a vendored hash that drifts every time deps change.
  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  # Proteus has no build-time native deps. Everything network-related goes
  # through zbus over D-Bus and rtnetlink at runtime; nothing links against
  # system libs at compile time.

  # Don't run unit tests during nix build — many of them touch netlink,
  # systemd, or NetworkManager state and have to run in CI containers, not
  # in a sandboxed nix builder.
  doCheck = false;

  meta = with lib; {
    description = "Erases the network identifiers your Linux laptop hands out (MAC, DHCP, IPv6, hostname, mDNS, TCP fingerprint, Bluetooth name)";
    homepage = "https://github.com/Kit3713/Proteus";
    license = licenses.gpl3Plus;
    platforms = platforms.linux;
    mainProgram = "proteus";
  };
}
