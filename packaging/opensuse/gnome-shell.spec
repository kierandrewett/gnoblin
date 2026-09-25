%global _prefix /usr/lib/gnoblin
%global __meson /usr/bin/meson
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(/usr/sbin/python3|lib(mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)])$
%global tarball_version %%(echo %{version} | tr '~' '.')
%bcond_with gnoblin_stack

Name:           gnoblin-shell
Version:        51.0
Release:        1%{?dist}
Summary:        Private GNOME Shell runtime for Gnoblin
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gnome-shell-%{tarball_version}.tar.xz
Source1:        gnoblin.json
Source2:        gnoblin.session
Source3:        gnoblin.desktop
Source4:        org.gnoblin.Shell.target
Source5:        org.gnoblin.Shell@wayland.service.in
Source6:        gnoblin-env.sh
Source7:        gnoblin-session
Source8:        gnoblin-shell-service
Source9:        gnoblinctl
Source10:       00_org.gnoblin.mutter.gschema.override
Source11:       gnome-session@gnoblin.target.d.conf
Source12:       gnoblin-seed-config
Source13:       init.lua.example
Source14:       Adwaita-Hyprcursor.tar.xz
Source15:       gnoblin-COPYING

BuildRequires:  desktop-file-utils
BuildRequires:  gcc-c++
BuildRequires:  gettext-tools
BuildRequires:  git
BuildRequires:  meson
BuildRequires:  pkgconfig(bash-completion)
BuildRequires:  pkgconfig(epoxy)
BuildRequires:  pkgconfig(gcr-4)
BuildRequires:  pkgconfig(gio-2.0) >= 2.86
BuildRequires:  pkgconfig(gjs-1.0) >= 1.85.90
BuildRequires:  pkgconfig(glib-2.0) >= 2.86
BuildRequires:  pkgconfig(gnome-autoar-0)
BuildRequires:  pkgconfig(gnome-desktop-4)
BuildRequires:  pkgconfig(gstreamer-base-1.0)
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(libadwaita-1)
BuildRequires:  pkgconfig(libcanberra)
BuildRequires:  pkgconfig(libedataserver-1.2)
BuildRequires:  pkgconfig(libnm)
BuildRequires:  pkgconfig(libpipewire-0.3)
BuildRequires:  pkgconfig(libpulse)
BuildRequires:  pkgconfig(librsvg-2.0)
BuildRequires:  pkgconfig(libstartup-notification-1.0)
BuildRequires:  pkgconfig(libsystemd)
BuildRequires:  pkgconfig(polkit-agent-1)
BuildRequires:  pkgconfig(xfixes)
%if %{with gnoblin_stack}
BuildRequires:  gnoblin-gsettings-desktop-schemas >= 51
BuildRequires:  gnoblin-mutter-devel >= 51
%endif
Requires:       gjs >= 1.85.90
Requires:       glib2 >= 2.86
Requires:       gnome-session
Requires:       gnome-settings-daemon
Requires:       gnoblin-gsettings-desktop-schemas >= 51
Requires:       gnoblin-mutter >= 51
Requires:       systemd
Requires:       xdg-desktop-portal-gnome

%description
Gnoblin's patched GNOME Shell, installed privately alongside stock GNOME Shell.

%package -n gnoblin-session
Summary:        Gnoblin login session and command-line tool
License:        GPL-2.0-or-later AND (LGPL-3.0-or-later OR CC-BY-SA-3.0)
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       gnome-session
Requires:       systemd

%description -n gnoblin-session
Adds Gnoblin to the login screen without replacing the GNOME session.

%prep
%autosetup -S git -n gnome-shell-%{tarball_version}

%build
export PKG_CONFIG_PATH=%{_libdir}/pkgconfig:%{_datadir}/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
export GI_GIR_PATH=%{_datadir}/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}
test "$(pkg-config --variable=prefix libmutter-51)" = "%{_prefix}"
%meson -Dc_args='-std=gnu17 -fPIE' -Dcpp_args='-std=c++20 -fPIE' \
  -Dextensions_tool=false -Dtests=false -Dman=false
%meson_build

%install
%meson_install
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
rm -f %{buildroot}%{_libdir}/systemd/user/org.gnome.Shell-disable-extensions.service
install -Dm644 %{SOURCE1} %{buildroot}%{_datadir}/gnome-shell/modes/gnoblin.json
install -Dm644 %{SOURCE2} %{buildroot}%{_datadir}/gnome-session/sessions/gnoblin.session
install -Dm644 %{SOURCE6} %{buildroot}%{_libexecdir}/gnoblin-env.sh
install -Dm755 %{SOURCE12} %{buildroot}%{_libexecdir}/gnoblin-seed-config
install -Dm644 %{SOURCE13} %{buildroot}%{_datadir}/gnoblin/init.lua.example
printf '%%s\n' '%{_lib}' > %{buildroot}%{_libexecdir}/gnoblin-libdir
install -Dm755 %{SOURCE7} %{buildroot}%{_bindir}/gnoblin-session
install -Dm755 %{SOURCE8} %{buildroot}%{_bindir}/gnoblin-shell-service
install -Dm755 %{SOURCE9} %{buildroot}%{_bindir}/gnoblinctl
install -Dm644 %{SOURCE10} %{buildroot}%{_datadir}/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override
mkdir -p %{buildroot}%{_datadir}/icons
tar -xJf %{SOURCE14} -C %{buildroot}%{_datadir}/icons

# These are the only Gnoblin files outside the private runtime.  The paths
# match Tumbleweed's systemd and display-manager search paths.
install -Dm644 %{SOURCE3} %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
sed -i 's|^Exec=.*|Exec=%{_bindir}/gnoblin-session|' %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
install -Dm644 %{SOURCE2} %{buildroot}/usr/share/gnome-session/sessions/gnoblin.session
install -Dm644 %{SOURCE4} %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell.target
install -Dm644 %{SOURCE11} %{buildroot}/usr/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE5} > %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
mkdir -p %{buildroot}/usr/bin
ln -s %{_bindir}/gnoblinctl %{buildroot}/usr/bin/gnoblinctl

%posttrans
/usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas

%postun
if [ -d %{_datadir}/glib-2.0/schemas ]; then
  /usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas
fi

%check
sed '/^DesktopNames=/d' %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop > gnoblin-validation.desktop
desktop-file-validate gnoblin-validation.desktop

%files
%license COPYING
%{_prefix}/

%files -n gnoblin-session
%license %{SOURCE15}
/usr/bin/gnoblinctl
/usr/share/wayland-sessions/gnoblin.desktop
/usr/share/gnome-session/sessions/gnoblin.session
/usr/lib/systemd/user/org.gnoblin.Shell.target
/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
/usr/lib/systemd/user/gnome-session@gnoblin.target.d/

%changelog
* Fri Sep 25 2026 Gnoblin contributors
- Initial openSUSE Tumbleweed adapter.
