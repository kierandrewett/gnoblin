%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(lib(gnome-shell-menu|mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)]|typelib[(](Clutter|Cogl|Gdm|Geoclue|GnomeBluetooth|GnomeQR|GWeather|Meta|Mtk|NMA4|Shell|St)[)]([[:space:]]*=[[:space:]]*.*)?)$
%global tarball_version %%(echo %{version} | tr '~' '.')
%bcond_with gnoblin_stack

Name:           gnoblin-shell
Version:        51.0
Release:        15%{?dist}
Summary:        Private GNOME Shell runtime for Gnoblin
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gnoblin-shell-%{tarball_version}.tar.xz
Source1:        gnoblin.json
Source2:        gnoblin.session
Source3:        gnoblin.desktop
Source4:        org.gnoblin.Shell.target
Source5:        org.gnoblin.Shell@wayland.service.in
Source6:        gnoblin-env.sh
Source7:        gnoblin
Source8:        gnoblin-shell-service
Source9:        gnoblinctl.c
Source10:       00_org.gnoblin.mutter.gschema.override
Source11:       gnome-session@gnoblin.target.d.conf
Source12:       gnoblin-seed-config
Source13:       init.lua.example
Source15:       gnoblin-COPYING
Source16:       gnoblin-session.target
Source17:       gnoblin-version.json
Source18:       gnoblin-idle.c
Source19:       gnoblin-idle.service.in

BuildRequires:  desktop-file-utils
BuildRequires:  gcc-c++
BuildRequires:  gettext-tools
BuildRequires:  git
BuildRequires:  meson
BuildRequires:  sassc
BuildRequires:  pkgconfig(epoxy)
BuildRequires:  pkgconfig(gcr-4) >= 3.90.0
BuildRequires:  pkgconfig(gio-2.0) >= 2.86
BuildRequires:  pkgconfig(gio-unix-2.0) >= 2.86
BuildRequires:  pkgconfig(girepository-2.0) >= 2.86.0
BuildRequires:  pkgconfig(gjs-1.0) >= 1.87.1
BuildRequires:  pkgconfig(json-glib-1.0)
BuildRequires:  pkgconfig(glib-2.0) >= 2.86
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(libcanberra)
BuildRequires:  pkgconfig(libnm)
BuildRequires:  pkgconfig(libpipewire-0.3)
BuildRequires:  pkgconfig(libpulse)
BuildRequires:  pkgconfig(librsvg-2.0)
BuildRequires:  pkgconfig(libstartup-notification-1.0)
BuildRequires:  pkgconfig(libsystemd)
BuildRequires:  pkgconfig(polkit-agent-1)
BuildRequires:  pkgconfig(xfixes)
%if %{with gnoblin_stack}
BuildRequires:  pkgconfig(gsettings-desktop-schemas) >= 51
BuildRequires:  gnoblin-mutter-devel >= 51
%endif
Requires:       gjs >= 1.87.1
Requires:       typelib(AccountsService) = 1.0
Requires:       typelib(IBus) = 1.0
Requires:       glib2 >= 2.86
Requires:       gsettings-desktop-schemas >= 51
Requires:       gnoblin-mutter >= 51
Requires:       systemd

%description
Gnoblin's patched GNOME Shell, installed privately alongside stock GNOME Shell.

%package -n gnoblin-session
Summary:        Gnoblin login session and command-line tool
License:        GPL-2.0-or-later AND (LGPL-3.0-or-later OR CC-BY-SA-3.0)
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       gnoblin-portal >= 51
Requires:       gnoblin-portal < 52
Requires:       adwaita-icon-theme
Requires:       dbus-1-tools
Requires:       dconf
Requires:       pipewire >= 1.6
Requires:       systemd
Requires:       wireplumber

%description -n gnoblin-session
Adds a lean Gnoblin login entry without replacing the GNOME session.

%prep
%autosetup -S git -n gnome-shell-%{tarball_version}

%build
export PKG_CONFIG_PATH=%{_libdir}/pkgconfig:%{_datadir}/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
export GI_GIR_PATH=%{_datadir}/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}
test "$(pkg-config --variable=prefix libmutter-51)" = "%{_prefix}"
/usr/bin/meson setup build . --buildtype=plain \
  --prefix=%{_prefix} --libdir=%{_libdir} --libexecdir=%{_libexecdir} \
  --bindir=%{_bindir} --sbindir=%{_sbindir} --includedir=%{_includedir} \
  --datadir=%{_datadir} --mandir=%{_mandir} --infodir=%{_infodir} \
  --localedir=%{_datadir}/locale --sysconfdir=%{_sysconfdir} \
  --localstatedir=%{_localstatedir} --sharedstatedir=%{_sharedstatedir} \
  --wrap-mode=nodownload --auto-features=enabled \
  -Dc_args='-std=gnu17 -fPIE' -Dcpp_args='-std=c++20 -fPIE' \
  -Dextensions_tool=false -Dtests=false -Dman=false -Dgtk_doc=false -Dportal_helper=false \
  -Dcalendar_server=false -Dhotplug_sniffer=false
/usr/bin/meson compile -C build %{?_smp_mflags}
%{__cc} %{optflags} -o gnoblin-idle %{SOURCE18} \
  $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0)
%{__cc} %{optflags} -o gnoblinctl %{SOURCE9} \
  $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0 json-glib-1.0)

%install
DESTDIR=%{buildroot} /usr/bin/meson install -C build --no-rebuild
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
rm -f %{buildroot}%{_libdir}/systemd/user/org.gnome.Shell-disable-extensions.service
install -Dm644 %{SOURCE15} %{buildroot}%{_datadir}/licenses/gnoblin-session/COPYING
install -Dm644 %{SOURCE1} %{buildroot}%{_datadir}/gnome-shell/modes/gnoblin.json
install -Dm644 %{SOURCE2} %{buildroot}%{_datadir}/gnome-session/sessions/gnoblin.session
install -Dm644 %{SOURCE6} %{buildroot}%{_libexecdir}/gnoblin-env.sh
install -Dm755 %{SOURCE12} %{buildroot}%{_libexecdir}/gnoblin-seed-config
install -Dm644 %{SOURCE13} %{buildroot}%{_datadir}/gnoblin/init.lua.example
install -Dm644 %{SOURCE17} %{buildroot}%{_datadir}/gnoblin/version.json
printf '%%s\n' '%{_lib}' > %{buildroot}%{_libexecdir}/gnoblin-libdir
install -Dm755 %{SOURCE7} %{buildroot}%{_bindir}/gnoblin
install -Dm755 %{SOURCE8} %{buildroot}%{_bindir}/gnoblin-shell-service
install -Dm755 gnoblinctl %{buildroot}%{_bindir}/gnoblinctl
install -Dm755 gnoblin-idle %{buildroot}%{_libexecdir}/gnoblin-idle
install -Dm644 %{SOURCE10} %{buildroot}%{_datadir}/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override

# These are the only Gnoblin files outside the private runtime.  The paths
# match Tumbleweed's systemd and display-manager search paths.
install -Dm644 %{SOURCE3} %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
sed -i -e 's|^Exec=.*|Exec=env GNOBLIN_STANDALONE_SESSION=1 %{_bindir}/gnoblin|' \
  -e 's|^DesktopNames=.*|DesktopNames=Gnoblin;|' %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
install -Dm644 %{SOURCE4} %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell.target
install -Dm644 %{SOURCE16} %{buildroot}/usr/lib/systemd/user/gnoblin-session.target
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE19} > %{buildroot}/usr/lib/systemd/user/gnoblin-idle.service
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE5} > %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
mkdir -p %{buildroot}/usr/bin
ln -s %{_bindir}/gnoblinctl %{buildroot}/usr/bin/gnoblinctl
ln -s %{_bindir}/gnoblin %{buildroot}/usr/bin/gnoblin

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
%license %{_datadir}/licenses/gnoblin-session/COPYING
/usr/bin/gnoblin
/usr/bin/gnoblinctl
/usr/share/wayland-sessions/gnoblin.desktop
/usr/lib/systemd/user/org.gnoblin.Shell.target
/usr/lib/systemd/user/gnoblin-session.target
/usr/lib/systemd/user/gnoblin-idle.service
/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service

%changelog
* Sun Sep 27 2026 Gnoblin contributors
- Ship the login command as gnoblin and retain its source provenance.

* Sun Sep 27 2026 Gnoblin contributors
- Drop the unused libxml2 development dependency from the patched Shell.

* Sun Sep 27 2026 Gnoblin contributors
- Omit the unused removable-media hotplug sniffer from Gnoblin builds.

* Sun Sep 27 2026 Gnoblin contributors
- Use Mutter's native monitor query from the control bridge.

* Sun Sep 27 2026 Gnoblin contributors
- Include the standalone idle service and its session unit.

* Sun Sep 27 2026 Gnoblin contributors
- Stop requiring unused GStreamer development headers for the Shell build.

* Sun Sep 27 2026 Gnoblin contributors
- Deliver Shell-originated Lua events through Mutter's document signal.

* Sun Sep 27 2026 Gnoblin contributors
- Watch generic Mutter signals only while Lua subscribes to them.

* Sun Sep 27 2026 Gnoblin contributors
- Finish Lua event delivery before committing its document.

* Sun Sep 27 2026 Gnoblin contributors
- Apply runtime event documents once and import the logind session class.

* Sun Sep 27 2026 Gnoblin contributors
- Require D-Bus activation tools and install the Gnoblin version for gnoblinctl.

* Sun Sep 27 2026 Gnoblin contributors
- Keep the private menu library internal and include session runtime services.

* Fri Sep 25 2026 Gnoblin contributors
- Filter private Meta typelib requirement from Shell.

* Fri Sep 25 2026 Gnoblin contributors
- Stage the session package license in the buildroot.

* Fri Sep 25 2026 Gnoblin contributors
- Initial openSUSE Tumbleweed adapter.
