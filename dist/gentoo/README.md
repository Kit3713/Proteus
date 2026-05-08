# Gentoo ebuild

An EAPI=8 ebuild for Proteus, drafted for the GURU overlay first and
::gentoo proper as a follow-up via proxy maintenance.

**Status: untested in production.** Validated locally with `pkgcheck
scan` and `repoman full -x manifest`; no Gentoo bug filed yet, no GURU
PR merged yet.

## Files

| File                              | Purpose                              |
| --------------------------------- | ------------------------------------ |
| `proteus-<version>.ebuild`        | The ebuild itself; rename per release tag. |
| `metadata.xml`                    | Package metadata + USE-flag descs.   |

## USE flags

| Flag              | Effect                                                        |
| ----------------- | ------------------------------------------------------------- |
| `bluetooth`       | Pull `net-wireless/bluez` for the `proteus bluetooth` family. |
| `enterprise-wifi` | Pull `wpa_supplicant` for 802.1X identifier rotation.         |
| `nft`             | Pull `nftables` for discovery-silencing rules.                |
| `openrc`          | Install OpenRC service + periodic shims.                      |
| `systemd`         | Install systemd timers + services.                            |

`REQUIRED_USE="|| ( openrc systemd )"` — at least one init has to be
selected so the rotation cadence actually has a driver.

## Build locally

In a Gentoo system or chroot with a personal overlay set up. Substitute `<ver>` for the version of the ebuild file you have (e.g. `0.4.0-beta1`):

```sh
mkdir -p /var/db/repos/myoverlay/net-misc/proteus
cp dist/gentoo/proteus-<ver>.ebuild  /var/db/repos/myoverlay/net-misc/proteus/
cp dist/gentoo/metadata.xml          /var/db/repos/myoverlay/net-misc/proteus/

# Generate Manifest (needs network for the GitHub tarball).
cd /var/db/repos/myoverlay/net-misc/proteus
ebuild proteus-<ver>.ebuild manifest

# Build with tests.
FEATURES=test emerge --usepkg=n =net-misc/proteus-<ver>
```

## Verification step

```sh
proteus doctor                  # 'init system: openrc' or 'systemd'
qlist -I proteus                # confirms install
equery files proteus | head     # spot-check install paths
```

If you built with `USE=systemd`:

```sh
systemctl enable --now proteus-rotate.timer
```

If you built with `USE=openrc`:

```sh
rc-update add proteus default
rc-service proteus start
```

## How to help

Gentoo developers and proxy maintainers: please review against
[Gentoo's devmanual][1] and PR fixes. The biggest unknowns are:

- Whether `cargo-ebuild` should be run to populate the `CRATES=` list,
  or whether vendoring the tarball is preferable.
- Whether the `BDEPEND` on `dev-libs/openssl` is actually needed (zbus
  doesn't link OpenSSL by default; this is defensive).
- Whether GURU is the right first home, or whether to skip directly to
  ::gentoo with proxy maintenance.

[1]: https://devmanual.gentoo.org/

See `wiki/distro-support.md` for the full distro × init × backend matrix.
