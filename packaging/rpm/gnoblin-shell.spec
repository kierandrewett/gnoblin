# Keep the patched runtime out of GNOME's system paths.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
# Private libraries must never satisfy dependencies of stock GNOME packages.
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(lib(gnome-shell-menu|mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)]|typelib[(](Clutter|Cogl|Gdm|Geoclue|GnomeBluetooth|GnomeQR|GWeather|Meta|Mtk|NMA4|Shell|St)[)]([[:space:]]*=[[:space:]]*.*)?)$

%global tarball_version %%(echo %{version} | tr '~' '.')
%define major_version %(c=%{version}; echo $c | cut -d. -f1 | cut -d~ -f1)

Name:           gnoblin-shell
Version:        51.0
# gnoblin: the source tarball already has gnoblin's patches applied
# (see ../../patches/gnome-shell), so this spec carries no Patch: directives.
Release:        38.gnoblin%{?dist}
%global debug_package %{nil}
Summary:        Private GNOME Shell runtime for Gnoblin

License:        GPL-2.0-or-later
URL:            https://wiki.gnome.org/Projects/GnomeShell
Source0:        gnoblin-shell-%{tarball_version}.tar.xz

# gnoblin-session subpackage sources (session mode, login entry, systemd
# --user units and control tools) — staged into the RPM sources directory by
# scripts/stage-rpm-sources.sh after Source0. These live at the gnoblin repo
# root (src/data/session/, src/tools/), not inside this tarball, since the
# rest of gnoblin's changes to gnome-shell itself are pre-applied above.
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

# gnoblin patches (tooling, control, settings, reload, branding) are
# pre-applied in the tarball produced by scripts/make-tarball.sh — no Patch:
# lines here.

%define glib2_version 2.86.0
%define gjs_version 1.87.1
%define girepository_version 2.86.0
%define gcr4_version 3.90.0
%define gtk4_version 4.0.0
%define mutter_version 51.0
%define polkit_version 0.100
%define gsettings_desktop_schemas_version 49~alpha
%define ibus_version 1.5.2
%define pipewire_version 0.3.49

BuildRequires:  pkgconfig(epoxy)
BuildRequires:  gcc
BuildRequires:  sassc
BuildRequires:  meson
BuildRequires:  git
BuildRequires:  desktop-file-utils
BuildRequires:  pkgconfig(atk-bridge-2.0)
BuildRequires:  pkgconfig(cairo)
BuildRequires:  pkgconfig(gcr-4) >= %{gcr4_version}
BuildRequires:  pkgconfig(gdk-pixbuf-2.0)
BuildRequires:  pkgconfig(girepository-2.0) >= %{girepository_version}
BuildRequires:  pkgconfig(gjs-1.0) >= %{gjs_version}
BuildRequires:  pkgconfig(gio-2.0) >= %{glib2_version}
BuildRequires:  pkgconfig(gio-unix-2.0) >= %{glib2_version}
BuildRequires:  pkgconfig(glib-2.0) >= %{glib2_version}
BuildRequires:  pkgconfig(glycin-2)
BuildRequires:  mesa-libGL-devel
BuildRequires:  mesa-libEGL-devel
BuildRequires:  pkgconfig(libnm)
BuildRequires:  pkgconfig(libsecret-1)
BuildRequires:  pkgconfig(json-glib-1.0)
BuildRequires:  pkgconfig(pango)
BuildRequires:  pkgconfig(polkit-agent-1) >= %{polkit_version}
BuildRequires:  pkgconfig(libstartup-notification-1.0)
BuildRequires:  pkgconfig(libsystemd)
BuildRequires:  pkgconfig(systemd)
BuildRequires:  pkgconfig(libpipewire-0.3) >= %{pipewire_version}
BuildRequires:  pkgconfig(gtk4) >= %{gtk4_version}
BuildRequires:  gettext >= 0.19.6
BuildRequires:  python3

BuildRequires:  pkgconfig(x11)
BuildRequires:  pkgconfig(xext)
BuildRequires:  pkgconfig(xfixes) >= 5.0
BuildRequires:  pkgconfig(librsvg-2.0)
BuildRequires:  gnoblin-mutter-devel >= 51.0
BuildRequires:  pkgconfig(gsettings-desktop-schemas) >= 51.0
BuildRequires:  pkgconfig(libpulse)
# Bootstrap requirements
Requires:       gcr-libs%{?_isa} >= %{gcr4_version}
Requires:       gjs%{?_isa} >= %{gjs_version}
Requires:       gtk4%{?_isa} >= %{gtk4_version}
# needed for loading SVG's via gdk-pixbuf
Requires:       librsvg2%{?_isa}
Requires:       gnoblin-mutter%{?_isa} >= 51.0
Requires:       polkit%{?_isa} >= %{polkit_version}
Requires:       glib2%{?_isa} >= %{glib2_version}
Requires:       gsettings-desktop-schemas >= 51.0
# needed for schemas
Requires:       at-spi2-atk%{?_isa}
# needed for gobject-introspection typelib
Requires:       ibus-libs%{?_isa} >= %{ibus_version}
# needed for the user menu
Requires:       accountsservice-libs%{?_isa}
# needed by some utilities
%description
Gnoblin's patched GNOME Shell, installed privately alongside stock GNOME Shell.

%package -n gnoblin-session
Summary: Gnoblin login session and command-line tool
License: GPL-2.0-or-later AND (LGPL-3.0-or-later OR CC-BY-SA-3.0)
Requires: %{name}%{?_isa} = %{version}-%{release}
Requires: gnoblin-portal >= 51
Requires: gnoblin-portal < 52
Requires: json-glib
Requires: adwaita-cursor-theme
Requires: dbus-tools
Requires: dconf
Requires: pipewire >= 1.6.0
Requires: systemd
Requires: wireplumber

%description -n gnoblin-session
Adds a lean Gnoblin login entry without replacing the GNOME session.

%prep
%autosetup -S git -n gnome-shell-%{tarball_version}

%build
export PKG_CONFIG_PATH=%{_libdir}/pkgconfig:%{_datadir}/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
export GI_GIR_PATH=%{_datadir}/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}
export LDFLAGS="${LDFLAGS//-Wl,-z,pack-relative-relocs/}"
export LDFLAGS="${LDFLAGS} -fPIE"
export CFLAGS="${CFLAGS} -fPIE"
# Refuse an accidental build against Fedora's Mutter.
test "$(pkg-config --variable=prefix libmutter-51)" = "%{_prefix}"
%meson -Dc_args='-std=gnu17 -fPIE' -Dcpp_args='-std=c++20 -fPIE' \
  -Dextensions_tool=false -Dtests=false -Dman=false -Dgtk_doc=false -Dportal_helper=false \
  -Dcalendar_server=false -Dhotplug_sniffer=false
%meson_build
%{__cc} %{optflags} -o gnoblin-idle %{SOURCE18} \
  $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0)
%{__cc} %{optflags} -o gnoblinctl %{SOURCE9} \
  $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0 json-glib-1.0)

%install
%meson_install
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
# Gnoblin does not expose GNOME Shell extension management. The runtime uses
# only its Gnoblin-named systemd units.
rm -f %{buildroot}%{_libdir}/systemd/user/org.gnome.Shell-disable-extensions.service
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
install -Dm644 %{SOURCE15} %{buildroot}%{_datadir}/licenses/gnoblin-session/COPYING

# Only Gnoblin-named entry points are installed outside the private runtime.
install -Dm644 %{SOURCE3} %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
sed -i -e 's|^Exec=.*|Exec=env GNOBLIN_STANDALONE_SESSION=1 %{_bindir}/gnoblin|' \
  -e 's|^DesktopNames=.*|DesktopNames=Gnoblin;|' \
  %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
install -Dm644 %{SOURCE4} %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell.target
install -Dm644 %{SOURCE16} %{buildroot}/usr/lib/systemd/user/gnoblin-session.target
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE19} > %{buildroot}/usr/lib/systemd/user/gnoblin-idle.service
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE5} \
  > %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
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
# DesktopNames is a display-manager key, not an application desktop-file key.
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
* Sun Sep 27 2026 Gnoblin contributors - 51.0-38.gnoblin
- Ship the login command as gnoblin without the old gnoblin-session executable alias.
- Record source provenance in the version shown by gnoblin and gnoblinctl.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-37.gnoblin
- Drop Shell's unused libxml2 build dependency.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-36.gnoblin
- Omit the unused removable-media hotplug sniffer from Gnoblin builds.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-35.gnoblin
- Build gnoblinctl as a native GLib client and drop the Python runtime requirement.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-34.gnoblin
- Use Mutter's native monitor query from the control bridge.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-33.gnoblin
- Include the standalone idle service and its session unit.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-32.gnoblin
- Stop requiring unused GStreamer development headers for the Shell build.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-31.gnoblin
- Deliver Shell-originated Lua events through Mutter's document signal.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-30.gnoblin
- Watch generic Mutter signals only while Lua subscribes to them.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-29.gnoblin
- Keep Lua event documents live until Mutter finishes signal delivery.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-28.gnoblin
- Apply each Lua runtime event document once before routing touchpad gestures.
- Pass the logind session class to desktop services in the lean login.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-27.gnoblin
- Install the Gnoblin version read by gnoblinctl from the private prefix.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-26.gnoblin
- Load the current default Lua configuration and handle console input events.
- Stop graphical portal services and clear the session environment at logout.
- Require the D-Bus activation environment tool used by the lean launcher.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-25.gnoblin
- Require Gcr's typelib and library without its viewer and SSH agent.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-24.gnoblin
- Install the session's PipeWire manager and persistent settings backend.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-23.gnoblin
- Leave the separate GNOME Shell screencast service's GStreamer runtime optional.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-22.gnoblin
- Keep the IBus daemon optional; only its typelib is required at startup.

* Sun Sep 27 2026 Gnoblin contributors - 51.0-21.gnoblin
- Declare direct build dependencies and filter the private menu library requirement.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-20.gnoblin
- Filter private Meta typelib requirement.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-19.gnoblin
- Filter private GnomeQR typelib requirements.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-18.gnoblin
- Stage the session package license in the buildroot.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-17.gnoblin
- Match the GNOME 51 GJS source floor.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-16.gnoblin
- Filter private GNOME Shell and Mutter typelib requirements.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-15.gnoblin
- Match declared GCR and GIRepository source API floors.

* Sun Sep 20 2026 Gnoblin contributors - 51.0-14.gnoblin
- Filter private Shell and Mutter library requirements from RPM metadata.

* Mon Sep 14 2026 Gnoblin contributors - 49.6-6.gnoblin
- Rebuild against Mutter without incompatible Fedora 44 introspection output.

* Mon Sep 14 2026 Gnoblin contributors - 49.6-5.gnoblin
- Disable Fedora 44 pack-relative-relocs for GObject Introspection links.

* Mon Sep 14 2026 Gnoblin contributors - 49.6-4.gnoblin
- Compile GNOME 49 C sources with GNU17 on Fedora 44 GCC 16.

* Sun Sep 13 2026 Gnoblin contributors - 49.6-3.gnoblin
- Rebuild for Fedora 44.

* Thu Sep 10 2026 Gnoblin contributors - 49.6-2.gnoblin
- Install alongside stock GNOME Shell under /usr/lib/gnoblin.
