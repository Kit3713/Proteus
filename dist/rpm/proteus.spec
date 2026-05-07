Name:           proteus
Version:        0.1.0
Release:        1%{?dist}
Summary:        Erase network-layer identifiers your Linux laptop hands out on every join

License:        GPL-3.0-or-later
URL:            https://github.com/Kit3713/Proteus
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  rust >= 1.85
BuildRequires:  cargo
BuildRequires:  systemd-rpm-macros
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  openssl-devel

Requires:       NetworkManager
Requires:       systemd
Requires:       glibc
Recommends:     bluez
Recommends:     nftables
Recommends:     polkit

# x86_64 covers laptops; aarch64 covers ARM SBCs and Apple Silicon Linux VMs.
# Other arches aren't tested and the project is laptop-focused, so don't
# silently produce broken packages on i686/ppc64le/s390x.
ExclusiveArch:  x86_64 aarch64

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
%systemd_post proteus-rotate.timer proteus-check.timer proteus-resume.service

%preun
%systemd_preun proteus-rotate.timer proteus-check.timer proteus-resume.service

%postun
%systemd_postun_with_restart proteus-rotate.timer proteus-check.timer

%changelog
* Wed May 06 2026 Kit3713 <noreply@example.com> - 0.1.0-1
- Initial RPM packaging for Phase A/B
