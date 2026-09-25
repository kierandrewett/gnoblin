# Keep the patched runtime out of GNOME's system paths.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
# Private libraries must never satisfy dependencies of stock GNOME packages.
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(/usr/sbin/python3|lib(mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)])$

%global tarball_version %%(echo %{version} | tr '~' '.')
%define major_version %(c=%{version}; echo $c | cut -d. -f1 | cut -d~ -f1)

%if 0%{?rhel}
%global portal_helper 0
%else
%global portal_helper 1
%endif

Name:           gnoblin-shell
Version:        51.0
# gnoblin: the source tarball already has gnoblin's patches applied
# (see ../../patches/gnome-shell), so this spec carries no Patch: directives.
Release:        15.gnoblin%{?dist}
%global debug_package %{nil}
Summary:        Private GNOME Shell runtime for Gnoblin

License:        GPL-2.0-or-later
URL:            https://wiki.gnome.org/Projects/GnomeShell
Source0:        gnome-shell-%{tarball_version}.tar.xz

# gnoblin-session subpackage sources (session mode, login entry, systemd
# --user units and control tools) — staged into the RPM sources directory by
# scripts/make-tarball.sh alongside Source0. These live at the gnoblin repo
# root (src/data/session/, src/tools/), not inside this tarball, since the
# rest of gnoblin's changes to gnome-shell itself are pre-applied above.
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

# gnoblin patches (tooling, control, settings, reload, branding) are
# pre-applied in the tarball produced by scripts/make-tarball.sh — no Patch:
# lines here.

%define eds_version 3.45.1
%define gnome_desktop_version 44.0-7
%define glib2_version 2.86.0
%define gjs_version 1.85.90
%define girepository_version 2.86.0
%define gcr4_version 3.90.0
%define gtk4_version 4.0.0
%define adwaita_version 1.5.0
%define mutter_version 51.0
%define polkit_version 0.100
%define gsettings_desktop_schemas_version 49~alpha
%define ibus_version 1.5.2
%define gnome_bluetooth_version 1:42.3
%define gstreamer_version 1.4.5
%define pipewire_version 0.3.49
%define gnome_settings_daemon_version 3.37.1

BuildRequires:  pkgconfig(bash-completion)
BuildRequires:  pkgconfig(epoxy)
BuildRequires:  gcc
BuildRequires:  sassc
BuildRequires:  meson
BuildRequires:  git
BuildRequires:  desktop-file-utils
BuildRequires:  pkgconfig(libedataserver-1.2) >= %{eds_version}
BuildRequires:  pkgconfig(gcr-4) >= %{gcr4_version}
BuildRequires:  pkgconfig(girepository-2.0) >= %{girepository_version}
BuildRequires:  pkgconfig(gjs-1.0) >= %{gjs_version}
BuildRequires:  pkgconfig(gio-2.0) >= %{glib2_version}
BuildRequires:  pkgconfig(gnome-autoar-0)
BuildRequires:  pkgconfig(gnome-desktop-4) >= %{gnome_desktop_version}
BuildRequires:  mesa-libGL-devel
BuildRequires:  mesa-libEGL-devel
BuildRequires:  pkgconfig(libnm)
BuildRequires:  pkgconfig(polkit-agent-1) >= %{polkit_version}
BuildRequires:  pkgconfig(libstartup-notification-1.0)
BuildRequires:  pkgconfig(libsystemd)
# for screencast recorder functionality
BuildRequires:  pkgconfig(gstreamer-base-1.0) >= %{gstreamer_version}
BuildRequires:  pkgconfig(libpipewire-0.3) >= %{pipewire_version}
BuildRequires:  pkgconfig(gtk4) >= %{gtk4_version}
BuildRequires:  gettext >= 0.19.6
BuildRequires:  python3

# for rst2man
BuildRequires:  python3-docutils
# for barriers
BuildRequires:  libXfixes-devel >= 5.0
# used in unused BigThemeImage
BuildRequires:  librsvg2-devel
BuildRequires:  gnoblin-mutter-devel >= 51.0
BuildRequires:  gnoblin-gsettings-desktop-schemas >= 51.0
BuildRequires:  pkgconfig(libpulse)
%ifnarch s390 s390x ppc ppc64 ppc64p7
BuildRequires:  gnome-bluetooth-libs-devel >= %{gnome_bluetooth_version}
%endif
# Bootstrap requirements
BuildRequires: gtk-doc
Requires:       gcr%{?_isa} >= %{gcr4_version}
Requires:       gjs%{?_isa} >= %{gjs_version}
Requires:       gtk4%{?_isa} >= %{gtk4_version}
Requires:       libadwaita%{_isa} >= %{adwaita_version}
Requires:       libnma-gtk4%{?_isa}
# needed for loading SVG's via gdk-pixbuf
Requires:       librsvg2%{?_isa}
Requires:       gnoblin-mutter%{?_isa} >= 51.0
Requires:       upower%{?_isa}
Requires:       polkit%{?_isa} >= %{polkit_version}
Requires:       gnome-desktop4%{?_isa} >= %{gnome_desktop_version}
Requires:       glib2%{?_isa} >= %{glib2_version}
Requires:       gnoblin-gsettings-desktop-schemas >= 51.0
Requires:       gnome-settings-daemon%{?_isa} >= %{gnome_settings_daemon_version}
Requires:       gstreamer1%{?_isa} >= %{gstreamer_version}
# needed for screen recorder
Requires:       gstreamer1-plugins-good%{?_isa}
Requires:       pipewire-gstreamer%{?_isa}
Requires:       xdg-user-dirs-gtk
# needed for schemas
Requires:       at-spi2-atk%{?_isa}
# needed for on-screen keyboard
Recommends:     ibus%{?_isa} >= %{ibus_version}
# needed for gobject-introspection typelib
Requires:       ibus-libs%{?_isa} >= %{ibus_version}
# needed for "show keyboard layout"
Requires:       tecla
# needed for the user menu
Requires:       accountsservice-libs%{?_isa}
Requires:       gdm-libs%{?_isa}
# needed by some utilities
Requires:       python3%{_isa}
# needed for the dual-GPU launch menu
Requires:       switcheroo-control
# needed for clocks/weather integration
Requires:       geoclue2-libs%{?_isa}
Requires:       libgweather4%{?_isa}
# needed for thunderbolt support
Recommends:     bolt%{?_isa}
# Needed for launching flatpak apps etc
# 1.8.0 is needed for source type support in the screencast portal.
Requires:       xdg-desktop-portal-gtk >= 1.8.0
Requires:       xdg-desktop-portal-gnome

%if %{portal_helper}
# needed for captive portal helper
Requires:     webkitgtk6.0%{?_isa}
%endif

# https://github.com/containers/composefs/pull/229#issuecomment-1838735764

%description
Gnoblin's patched GNOME Shell, installed privately alongside stock GNOME Shell.

%package -n gnoblin-session
Summary: Gnoblin login session and command-line tool
License: GPL-2.0-or-later AND (LGPL-3.0-or-later OR CC-BY-SA-3.0)
Requires: %{name}%{?_isa} = %{version}-%{release}
Requires: gnome-session
Requires: systemd

%description -n gnoblin-session
Adds Gnoblin to the login screen without replacing the GNOME session.

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
  -Dextensions_tool=false -Dtests=false -Dman=false
%meson_build

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
printf '%%s\n' '%{_lib}' > %{buildroot}%{_libexecdir}/gnoblin-libdir
install -Dm755 %{SOURCE7} %{buildroot}%{_bindir}/gnoblin-session
install -Dm755 %{SOURCE8} %{buildroot}%{_bindir}/gnoblin-shell-service
install -Dm755 %{SOURCE9} %{buildroot}%{_bindir}/gnoblinctl
install -Dm644 %{SOURCE10} %{buildroot}%{_datadir}/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override
mkdir -p %{buildroot}%{_datadir}/icons
tar -xJf %{SOURCE14} -C %{buildroot}%{_datadir}/icons

# Only Gnoblin-named entry points are installed outside the private runtime.
install -Dm644 %{SOURCE3} %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
sed -i 's|^Exec=.*|Exec=%{_bindir}/gnoblin-session|' \
  %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
install -Dm644 %{SOURCE2} %{buildroot}/usr/share/gnome-session/sessions/gnoblin.session
install -Dm644 %{SOURCE4} %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell.target
install -Dm644 %{SOURCE11} \
  %{buildroot}/usr/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf
sed 's|@PREFIX@|%{_prefix}|g' %{SOURCE5} \
  > %{buildroot}/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
mkdir -p %{buildroot}/usr/bin
ln -s %{_bindir}/gnoblinctl %{buildroot}/usr/bin/gnoblinctl

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
%license %{SOURCE15}
/usr/bin/gnoblinctl
/usr/share/wayland-sessions/gnoblin.desktop
/usr/share/gnome-session/sessions/gnoblin.session
/usr/lib/systemd/user/org.gnoblin.Shell.target
/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
/usr/lib/systemd/user/gnome-session@gnoblin.target.d/

%changelog
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
