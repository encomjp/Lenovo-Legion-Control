Name:           legion-control
Version:        0.1.0
Release:        1%{?dist}
Summary:        Lenovo Legion hardware control suite
License:        GPL-2.0-only
URL:            https://github.com/encomjp/lenovo-legion-tool
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.87
BuildRequires:  rust >= 1.87
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(libudev)
BuildRequires:  systemd-rpm-macros
Requires:       systemd
Requires:       polkit
Suggests:       plasma-workspace >= 6
Suggests:       dkms

%description
Legion Control provides a root daemon, CLI, GTK/libadwaita settings
application, fan and power-profile control, battery charge limits, Spectrum
RGB lighting, telemetry, diagnostics, and an embedded KDE Plasma 6 widget.

%prep
%autosetup

%build
cargo build --release --locked

%install
install -Dm755 target/release/legion-cli %{buildroot}%{_bindir}/legion-cli
install -Dm755 target/release/legion-daemon %{buildroot}%{_bindir}/legion-daemon
install -Dm755 target/release/legion-settings %{buildroot}%{_bindir}/legion-settings
install -Dm755 target/release/legion-control-setup %{buildroot}%{_libexecdir}/legion-control-setup
install -Dm644 data/polkit/com.encomjp.legion-control.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/com.encomjp.legion-control.policy
mkdir -p %{buildroot}%{_prefix}/lib/legion-control/ryzen_smu
cp -a third_party/ryzen_smu/. %{buildroot}%{_prefix}/lib/legion-control/ryzen_smu/
install -Dm644 packaging/common/legion-control.service \
    %{buildroot}%{_unitdir}/legion-control.service
install -Dm644 data/udev/99-legion.rules \
    %{buildroot}/usr/lib/udev/rules.d/99-legion.rules
install -Dm644 data/gui/com.encomjp.legion-settings.desktop \
    %{buildroot}%{_datadir}/applications/com.encomjp.legion-settings.desktop
install -Dm644 data/icons/app-mark.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg
install -Dm644 data/icons/tray.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg

%post
%systemd_post legion-control.service
if [ -d /run/systemd/system ]; then
    systemctl enable --now legion-control.service >/dev/null 2>&1 || :
fi
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || :
    udevadm trigger -s hidraw || :
fi

%preun
%systemd_preun legion-control.service

%postun
%systemd_postun_with_restart legion-control.service

%files
%doc README.md
%{_bindir}/legion-cli
%{_bindir}/legion-daemon
%{_bindir}/legion-settings
%{_libexecdir}/legion-control-setup
%{_datadir}/polkit-1/actions/com.encomjp.legion-control.policy
%{_prefix}/lib/legion-control/ryzen_smu/
%{_unitdir}/legion-control.service
/usr/lib/udev/rules.d/99-legion.rules
%{_datadir}/applications/com.encomjp.legion-settings.desktop
%{_datadir}/icons/hicolor/scalable/apps/com.encomjp.legion-settings.svg
%{_datadir}/icons/hicolor/scalable/status/com.encomjp.legion-settings-tray.svg

%changelog
* Sun Jul 26 2026 europeanpepe <noreply@github.com> - 0.1.0-1
- Initial native package
