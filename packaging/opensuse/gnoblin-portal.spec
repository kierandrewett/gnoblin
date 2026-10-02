%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _libexecdir %{_prefix}/libexec
%global _datadir %{_prefix}/share
%global tarball_version %%(echo %{version} | tr '~' '.')

Name:           gnoblin-portal
Version:        51.0
Release:        2%{?dist}
Summary:        Gnoblin desktop portal backend
License:        LGPL-2.1-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        xdg-desktop-portal-gnome-%{tarball_version}.tar.xz

BuildRequires:  gcc
BuildRequires:  gettext-tools
BuildRequires:  meson
BuildRequires:  ninja
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(gio-unix-2.0)
BuildRequires:  pkgconfig(glib-2.0) >= 2.76
BuildRequires:  pkgconfig(glycin-2)
BuildRequires:  pkgconfig(gsettings-desktop-schemas)
BuildRequires:  pkgconfig(gtk4) >= 4.22.0
BuildRequires:  pkgconfig(gtk4-unix-print)
BuildRequires:  pkgconfig(libadwaita-1) >= 1.7
BuildRequires:  pkgconfig(xdg-desktop-portal) >= 1.21.1
Requires:       gtk4 >= 4.22.0
Requires:       xdg-desktop-portal >= 1.21.1

%description
Provides a separate portal backend for Gnoblin sessions. An existing GNOME
portal service remains available for GNOME logins.

%prep
%autosetup -n xdg-desktop-portal-gnome-%{tarball_version}

%build
%meson --prefix=%{_prefix} --libdir=%{_libdir} \
  --libexecdir=%{_libexecdir} --datadir=%{_datadir} \
  -Ddbus_service_dir=%{_datadir}/dbus-1/services \
  -Dsystemduserunitdir=%{_prefix}/lib/systemd/user \
  --wrap-mode=nodownload
%meson_build

%install
%meson_install
install -Dm644 %{buildroot}%{_datadir}/xdg-desktop-portal/portals/gnoblin.portal \
  %{buildroot}/usr/share/xdg-desktop-portal/portals/gnoblin.portal
install -Dm644 %{buildroot}%{_datadir}/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service \
  %{buildroot}/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
install -Dm644 %{buildroot}%{_prefix}/lib/systemd/user/xdg-desktop-portal-gnoblin.service \
  %{buildroot}/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service

%files
%license COPYING
%{_prefix}/
/usr/share/xdg-desktop-portal/portals/gnoblin.portal
/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service
