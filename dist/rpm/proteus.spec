Name:           proteus
Version:        0.4.2~beta
Release:        1%{?dist}
Summary:        Erase network-layer identifiers your Linux laptop hands out on every join

License:        GPL-3.0-or-later
URL:            https://github.com/Kit3713/Proteus
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

# Milestone 5: explicit cargo + rust >= 1.85 BRs (edition 2024 floor).
# `rust-packaging` would also pull these via %cargo_build, but we drive
# cargo by hand (see %build) so the BRs need to be spelled out.
BuildRequires:  cargo
BuildRequires:  rust >= 1.85
BuildRequires:  systemd-rpm-macros
BuildRequires:  pkgconfig(dbus-1)

Requires:       NetworkManager
Requires:       systemd
Requires:       glibc
Recommends:     bluez
Recommends:     nftables
Recommends:     polkit

# Roadmap Milestone 5: arch-neutral. The CI cross-compile matrix covers
# x86_64 / aarch64 / armv7 today (laptops, ARM SBCs, Apple Silicon Linux
# VMs, Pi 2/3, ARM Chromebooks); other arches are best-effort but no
# longer gated out at the spec level — packagers can rebuild on whatever
# Fedora/Copr targets.

%description
A Rust CLI that rotates and scrubs network-layer identifiers (MAC addresses,
DHCP options, IPv6 stable-privacy, hostname, mDNS chatter, TCP fingerprint
quirks, Bluetooth name) so your Linux laptop is harder to track across
networks. Single binary, embedded wiki, runs on Fedora 43+ with systemd and
NetworkManager.

%prep
%autosetup -n Proteus-%{version}

%build
# Use an explicit cargo invocation rather than %cargo_build. The
# rust-rpm-macros / systemd-rpm-macros %cargo_build expansion has been
# observed to exit non-zero on fedora:43 (likely vendoring or %{__cargo}
# resolution issues in the container). Calling cargo directly matches the
# rest of CI and is the same recipe used for the raw-binary release jobs.
cargo build --release --locked

%check
# Library tests only — integration tests need a privileged systemd
# container (Phase G) and aren't `cargo test --lib` clean. Matches the
# Alpine APKBUILD, Void template, and Debian rules.
#
# NPKG.9: `--without check` is *available* (rpmbuild honors `%bcond_without
# check` automatically) but a packager who reaches for it should know they
# are skipping the only build-time test gate this spec offers. The
# documented bypass risk lives in dist/rpm/README.md. The default below
# leaves `check` ON; setting `--without check` on the rpmbuild command line
# expands the conditional to a no-op.
%bcond_without check
%if %{with check}
cargo test --release --locked --lib
%else
echo "WARNING: %check skipped via --without check (NPKG.9). Validate the build elsewhere."
%endif

%install
install -Dm755 target/release/proteus %{buildroot}%{_bindir}/proteus

# man page
install -Dm644 dist/man/proteus.1 %{buildroot}%{_mandir}/man1/proteus.1

# shell completions
install -Dm644 dist/completions/proteus.bash %{buildroot}%{_datadir}/bash-completion/completions/proteus
install -Dm644 dist/completions/proteus.zsh %{buildroot}%{_datadir}/zsh/site-functions/_proteus
install -Dm644 dist/completions/proteus.fish %{buildroot}%{_datadir}/fish/vendor_completions.d/proteus.fish

# systemd units (timers, services, boot oneshot, resume hook)
for unit in dist/systemd/*.service dist/systemd/*.timer; do
    install -Dm644 "$unit" "%{buildroot}%{_unitdir}/$(basename $unit)"
done

# NetworkManager dispatcher hook (event-driven rotation)
install -Dm755 dist/networkmanager/dispatcher.d/01-proteus %{buildroot}%{_sysconfdir}/NetworkManager/dispatcher.d/01-proteus

# polkit policy for desktop-GUI-friendly elevation via pkexec
install -Dm644 dist/polkit/com.kit3713.proteus.policy %{buildroot}%{_datadir}/polkit-1/actions/com.kit3713.proteus.policy

# Config dir is world-readable; state dir is root-only (caches the
# permanent MAC and the original hostname — sacred per docs/PLAN.md).
install -dm755 %{buildroot}%{_sysconfdir}/proteus
install -dm700 %{buildroot}%{_sharedstatedir}/proteus

%files
%license LICENSE
%doc README.md CONTRIBUTING.md SECURITY.md docs/PLAN.md docs/ROADMAP.md docs/PRIOR-ART.md
%{_bindir}/proteus
%{_mandir}/man1/proteus.1*
%{_datadir}/bash-completion/completions/proteus
%{_datadir}/zsh/site-functions/_proteus
%{_datadir}/fish/vendor_completions.d/proteus.fish
%{_unitdir}/proteus-*.service
%{_unitdir}/proteus-*.timer
%{_sysconfdir}/NetworkManager/dispatcher.d/01-proteus
%{_datadir}/polkit-1/actions/com.kit3713.proteus.policy
%dir %{_sysconfdir}/proteus
%dir %attr(0700,root,root) %{_sharedstatedir}/proteus

%post
# B1 / B2 / N12.8: include proteus-events.service so RPM's preset machinery
# wires it up like every other unit the package installs. The daemon itself
# short-circuits when `[events] enabled = false` in /etc/proteus/config.toml,
# so enabling the unit is harmless when the operator hasn't opted in. This
# closes the "events daemon is unreachable on every install path" bug
# (issue #279 superseded by Stream 3 acceptance).
%systemd_post proteus-rotate.timer proteus-check.timer proteus-boot.service proteus-resume.service proteus-events.service

%preun
%systemd_preun proteus-rotate.timer proteus-check.timer proteus-boot.service proteus-resume.service proteus-events.service

%postun
# Restart-on-upgrade only really matters for the timers (long-lived) and
# the events daemon (long-lived); the oneshots (boot, resume) are listed
# for symmetry with %post/%preun, and %systemd_postun_with_restart is a
# no-op for already-exited oneshots.
%systemd_postun_with_restart proteus-rotate.timer proteus-check.timer proteus-boot.service proteus-resume.service proteus-events.service

%changelog
* Fri May 08 2026 Kit3713 <noreply@example.com> - 0.4.2~beta-1
- v0.4.2-beta: closes remainder of May 2026 audit tree (#279, #285-306) +
  three audit follow-up findings (PROTEUS_*_DIR env-var lockdown, iface
  validation on ethtool/iw, iw/ip `--` defense-in-depth). Persona export
  safety parity, quarantine preserves originals, cross-layer persona
  consistency, randomized rotation cadence, SHA-256 deduplicated,
  completions regenerated, packaging dropped clashing debian/compat.
- %systemd_post + %preun + %postun_with_restart now include
  proteus-boot.service alongside the timers and resume.service.
* Fri May 08 2026 Kit3713 <noreply@example.com> - 0.4.0~beta1-1
- v0.4.0-beta1: closes May 2026 vulnerability hunt cluster (#225-#275 +
  #276/#284/#297). Persona schema validation, output sanitization, PATH
  hardening, systemd hardening parity, NM dispatcher hardening, event
  rate limits, unbiased random pickers, rotate-if-needed TOCTOU, --yes
  dispatch parity, SHA-pinned actions, packaging version sync.
* Thu May 07 2026 Kit3713 <noreply@example.com> - 0.1.0-1
- Milestone 5 polish: explicit cargo + rust >= 1.85 BRs, %check section
  running `cargo test --release --lib`, dropped stale openssl-devel BR
  (zbus 5 + tokio feature doesn't pull OpenSSL).
* Wed May 06 2026 Kit3713 <noreply@example.com> - 0.1.0-1
- Initial RPM packaging for Phase A/B
