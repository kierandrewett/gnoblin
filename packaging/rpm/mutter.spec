# Keep the patched runtime out of GNOME's system paths.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
# Private libraries must never satisfy dependencies of stock GNOME packages.
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(lib(mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)]|typelib[(](Clutter|Cogl|Mtk|Shell|St)[)]([[:space:]]*=[[:space:]]*.*)?)$

%global glib_version 2.81.1
%global gobject_introspection_version 1.41.4
%global gtk3_version 3.19.8
%global gtk4_version 4.14.0
%global glycin_version 2.0.beta.2
%global hyprcursor_version 0.1.11
%global gsettings_desktop_schemas_version 51.0
%global libdrm_version 2.4.118
%global libdisplay_info_version 0.2
%global libinput_version 1.27.0
%global pixman_version 0.42
%global pipewire_version 1.2.7
%global lcms2_version 2.6
%global colord_version 1.4.5
%global libei_version 1.3.901
%global mutter_api_version 51
%global wayland_protocols_version 1.48
%global wayland_server_version 1.24

%global major_version %%(echo %{version} | cut -d '.' -f1 | cut -d '~' -f 1)
%global tarball_version %%(echo %{version} | tr '~' '.')

Name:          gnoblin-mutter
Version:       51.0
# gnoblin: the source tarball already has gnoblin's patches applied
# (see ../../patches/mutter), so this spec carries no Patch: directives.
Release:       24.gnoblin%{?dist}
%global debug_package %{nil}
Summary:       Private Mutter runtime for Gnoblin

# Automatically converted from old format: GPLv2+ - review is highly recommended.
License:       GPL-2.0-or-later
URL:           http://www.gnome.org
Source0:       mutter-%{tarball_version}.tar.xz

# https://pagure.io/fedora-workstation/issue/357
Source1:       org.gnome.mutter.fedora.gschema.override

# gnoblin patches (tooling, layer-shell, screencopy, window-management) are
# pre-applied in the tarball produced by scripts/make-tarball.sh — no Patch:
# lines here.

BuildRequires: cvt
BuildRequires: desktop-file-utils
BuildRequires: mesa-libEGL-devel
BuildRequires: mesa-libGLES-devel
BuildRequires: mesa-libGL-devel
BuildRequires: mesa-libgbm-devel
BuildRequires: pam-devel
BuildRequires: pkgconfig(bash-completion)
BuildRequires: pkgconfig(colord) >= %{colord_version}
BuildRequires: pkgconfig(glib-2.0) >= %{glib_version}
BuildRequires: pkgconfig(hyprcursor) >= %{hyprcursor_version}
BuildRequires: pkgconfig(gobject-introspection-1.0) >= %{gobject_introspection_version}
BuildRequires: pkgconfig(sm)
BuildRequires: pkgconfig(lcms2) >= %{lcms2_version}
BuildRequires: pkgconfig(lua)
BuildRequires: pkgconfig(libadwaita-1)
BuildRequires: pkgconfig(libwacom)
BuildRequires: pkgconfig(xkbcommon)
BuildRequires: pkgconfig(glesv2)
BuildRequires: pkgconfig(graphene-gobject-1.0)
BuildRequires: pkgconfig(libdisplay-info) >= %{libdisplay_info_version}
BuildRequires: pkgconfig(libpipewire-0.3) >= %{pipewire_version}
BuildRequires: pkgconfig(sysprof-capture-4)
BuildRequires: pkgconfig(libsystemd)
BuildRequires: pkgconfig(umockdev-1.0)
BuildRequires: python3-argcomplete
BuildRequires: python3-docutils
# Bootstrap requirements
BuildRequires: gettext-devel git-core
BuildRequires: gcc-c++
BuildRequires: pkgconfig(libcanberra)
BuildRequires: gnoblin-gsettings-desktop-schemas >= %{gsettings_desktop_schemas_version}
BuildRequires: pkgconfig(gtk4) >= %{gtk4_version}
BuildRequires: pkgconfig(gnome-settings-daemon)
BuildRequires: meson
BuildRequires: pkgconfig(gbm)
BuildRequires: pkgconfig(glycin-2) >= %{glycin_version}
BuildRequires: pkgconfig(gnome-desktop-4)
BuildRequires: pkgconfig(gudev-1.0)
BuildRequires: pkgconfig(libdrm) >= %{libdrm_version}
BuildRequires: pkgconfig(libei-1.0) >= %{libei_version}
BuildRequires: pkgconfig(libeis-1.0) >= %{libei_version}
BuildRequires: pkgconfig(libstartup-notification-1.0)
BuildRequires: pkgconfig(wayland-protocols) >= %{wayland_protocols_version}
BuildRequires: pkgconfig(wayland-server) >= %{wayland_server_version}
BuildRequires: sysprof-devel

BuildRequires: pkgconfig(libinput) >= %{libinput_version}
BuildRequires: pkgconfig(pixman-1) >= %{pixman_version}
BuildRequires: pkgconfig(xwayland)

BuildRequires: python3-dbusmock

Requires: gnoblin-gsettings-desktop-schemas >= %{gsettings_desktop_schemas_version}
Requires: gnome-settings-daemon
Requires: glib2
Requires: polkit
Recommends: mesa-dri-drivers

%description
Gnoblin's patched Mutter, installed privately alongside the system compositor.

%package devel
Summary: Headers for building Gnoblin Shell against its private Mutter
Requires: %{name}%{?_isa} = %{version}-%{release}

%description devel
Private headers and pkg-config files for Gnoblin builds.

%prep
%autosetup -S git -n mutter-%{tarball_version}

%build
export PKG_CONFIG_PATH=%{_datadir}/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
export GI_GIR_PATH=%{_datadir}/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}
export LDFLAGS="${LDFLAGS//-Wl,-z,pack-relative-relocs/}"
export LDFLAGS="${LDFLAGS} -fPIE"
%meson -Dc_args='-std=gnu17 -fPIE' -Dcpp_args='-std=c++20 -fPIE' -Db_pie=false \
  -Dintrospection=true -Dtests=disabled -Ddocs=false -Dprofiler=false \
  -Dudev_dir=%{_prefix}/lib/udev
%meson_build

%install
%meson_install
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
install -p %{SOURCE1} %{buildroot}%{_datadir}/glib-2.0/schemas
# pkexec needs a globally visible policy, with a distinct action ID and helper.
install -d %{buildroot}/usr/share/polkit-1/actions
sed 's/org.gnome.mutter.backlight-helper/org.gnoblin.mutter.backlight-helper/g' \
  %{buildroot}%{_datadir}/polkit-1/actions/org.gnome.mutter.backlight-helper.policy \
  > %{buildroot}/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy

%posttrans
/usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas

%postun
if [ -d %{_datadir}/glib-2.0/schemas ]; then
  /usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas
fi

%files
%license COPYING
%{_prefix}/
%ghost %{_datadir}/glib-2.0/schemas/gschemas.compiled
%exclude %{_includedir}/
%exclude %{_libdir}/pkgconfig/
%exclude %{_libdir}/lib*.so
/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy

%files devel
%{_includedir}/
%{_libdir}/pkgconfig/
%{_libdir}/lib*.so

%changelog
* Sat Sep 26 2026 Gnoblin contributors - 51.0-24.gnoblin
- Match the Hyprcursor API floor used by Gnoblin's Mutter patches.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-23.gnoblin
- Match the GNOME 51 Wayland Protocols source floor.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-22.gnoblin
- Filter private Clutter, Cogl and Mtk typelib requirements.

* Fri Sep 25 2026 Gnoblin contributors - 51.0-21.gnoblin
- Match declared Glycin, libdisplay-info and Hyprcursor source API floors.

* Tue Sep 22 2026 Gnoblin contributors - 51.0-20.gnoblin
- Respect reserved exclusive zones while moving windows.

* Sun Sep 20 2026 Gnoblin contributors - 51.0-19.gnoblin
- Discover private GNOME schema introspection data while building.

* Sun Sep 20 2026 Gnoblin contributors - 51.0-18.gnoblin
- Keep private library capabilities out of the system RPM namespace.

* Mon Sep 14 2026 Gnoblin contributors - 49.5-6.gnoblin
- Disable incompatible Fedora 44 GObject Introspection generation.

* Mon Sep 14 2026 Gnoblin contributors - 49.5-5.gnoblin
- Disable Fedora 44 pack-relative-relocs for GObject Introspection links.

* Mon Sep 14 2026 Gnoblin contributors - 49.5-4.gnoblin
- Compile GNOME 49 C sources with GNU17 on Fedora 44 GCC 16.

* Sun Sep 13 2026 Gnoblin contributors - 49.5-3.gnoblin
- Build against Fedora 44's hyprcursor 0.1.11 ABI.

* Thu Sep 10 2026 Gnoblin contributors - 49.5-2.gnoblin
- Install alongside stock Mutter under /usr/lib/gnoblin.
