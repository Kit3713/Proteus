# Copyright 2026 Kit3713
# Distributed under the terms of the GNU General Public License v3 or later
#
# Build:    ebuild proteus-0.1.0.ebuild manifest && \
#           emerge --usepkg=n =net-misc/proteus-0.1.0
# Verify:   FEATURES=test emerge =net-misc/proteus-0.1.0 && proteus doctor
#
# Status:   UNTESTED in production. Validated locally with `pkgcheck scan`
#           and `repoman full -x manifest`. No Gentoo bug filed yet — this
#           is a candidate for the GURU overlay first, then proper proxy
#           maintenance for ::gentoo.
#
# Reference: https://devmanual.gentoo.org/

EAPI=8

CRATES=""  # populated by `cargo-ebuild` once a release tarball is staged.

inherit cargo systemd

DESCRIPTION="Erase the network identifiers your Linux laptop hands out on every join"
HOMEPAGE="https://github.com/Kit3713/Proteus"
SRC_URI="https://github.com/Kit3713/Proteus/archive/v${PV}.tar.gz -> ${P}.tar.gz
	${CARGO_CRATE_URIS}"

LICENSE="GPL-3"
SLOT="0"
KEYWORDS="~amd64 ~arm ~arm64"

IUSE="bluetooth enterprise-wifi nft openrc systemd test"
REQUIRED_USE="|| ( openrc systemd )"
RESTRICT="!test? ( test )"

# Hard runtime: dbus + a network manager. We don't pin networkmanager
# strictly because a future release will support systemd-networkd.
RDEPEND="
	sys-apps/dbus
	|| (
		net-misc/networkmanager
		sys-apps/systemd[-networkd?]
	)
	bluetooth?       ( net-wireless/bluez )
	nft?             ( net-firewall/nftables )
	enterprise-wifi? ( net-wireless/wpa_supplicant )
	systemd?         ( sys-apps/systemd )
	openrc?          ( sys-apps/openrc )
"
DEPEND="${RDEPEND}"
BDEPEND="
	>=virtual/rust-1.85
	dev-libs/openssl
"

S="${WORKDIR}/Proteus-${PV}"

src_unpack() {
	cargo_src_unpack
}

src_configure() {
	# Pass USE flags through as cargo features once the source supports
	# them. Today's Cargo.toml has no [features] table, so this is a
	# no-op that documents intent.
	local myfeatures=()
	use bluetooth       && myfeatures+=(bluetooth)
	use enterprise-wifi && myfeatures+=(enterprise-wifi)
	use nft             && myfeatures+=(nft)
	cargo_src_configure --no-default-features
}

src_compile() {
	cargo_src_compile
}

src_test() {
	# Library tests only — see Alpine/Debian/Void recipes for the same
	# rationale (integration tests need a privileged systemd container).
	cargo test --release --frozen --lib || die "cargo test --lib failed"
}

src_install() {
	cargo_src_install

	doman dist/man/proteus.1

	newbashcomp dist/completions/proteus.bash proteus
	insinto /usr/share/zsh/site-functions
	newins  dist/completions/proteus.zsh _proteus
	insinto /usr/share/fish/vendor_completions.d
	doins   dist/completions/proteus.fish

	if use systemd; then
		systemd_dounit dist/systemd/proteus-rotate.service
		systemd_dounit dist/systemd/proteus-rotate.timer
		systemd_dounit dist/systemd/proteus-check.service
		systemd_dounit dist/systemd/proteus-check.timer
		systemd_dounit dist/systemd/proteus-boot.service
		systemd_dounit dist/systemd/proteus-resume.service
	fi

	if use openrc; then
		newinitd dist/openrc/proteus.initd proteus
		exeinto  /etc/periodic/hourly
		newexe   dist/openrc/proteus-rotate.periodic proteus-rotate
		exeinto  /etc/periodic/15min
		newexe   dist/openrc/proteus-check.periodic  proteus-check
	fi

	insinto /etc/NetworkManager/dispatcher.d
	doins   dist/networkmanager/dispatcher.d/01-proteus
	fperms 0755 /etc/NetworkManager/dispatcher.d/01-proteus

	insinto /usr/share/polkit-1/actions
	doins   dist/polkit/com.kit3713.proteus.policy

	keepdir /etc/proteus
	keepdir /var/lib/proteus
	fperms 0700 /var/lib/proteus
}

pkg_postinst() {
	elog ""
	elog "Run \`proteus doctor\` to verify your init system, libc, and"
	elog "backend are detected correctly. Then enable the timers/services"
	elog "for whichever init you use:"
	elog ""
	if use systemd; then
		elog "  systemctl enable --now proteus-rotate.timer proteus-check.timer"
	fi
	if use openrc; then
		elog "  rc-update add proteus default"
		elog "  rc-service proteus start"
	fi
	elog ""
	elog "See \`proteus wiki rotation\` for cadence tuning, and"
	elog "\`wiki/distro-support.md\` for the full distro × init × backend"
	elog "matrix."
}
