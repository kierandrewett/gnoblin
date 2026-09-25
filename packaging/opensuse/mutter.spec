# Tumbleweed uses capability BuildRequires because package names change more
# often than the pkg-config interfaces consumed by Mutter.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _sysconfdir %{_prefix}/etc
%global _localstatedir %{_prefix}/var
%global _sharedstatedir %{_prefix}/var/lib
%global debug_package %{nil}
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(lib(mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)])$

%global glib_version 2.81.1
%global gobject_introspection_version 1.41.4
%global gtk4_version 4.14.0
%global gsettings_desktop_schemas_version 51.0
%global libdrm_version 2.4.118
%global libinput_version 1.27.0
%global pipewire_version 1.2.7
%global libei_version 1.3.901
%global wayland_protocols_version 1.45
%global wayland_server_version 1.24
%global tarball_version %%(echo %{version} | tr '~' '.')

# Enable this when building against the locally built schema RPM.  CI leaves
# it disabled to resolve only repository-provided BuildRequires.
%bcond_with gnoblin_stack

Name:           gnoblin-mutter
Version:        51.0
Release:        1%{?dist}
Summary:        Private Mutter runtime for Gnoblin
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        mutter-%{tarball_version}.tar.xz

BuildRequires:  desktop-file-utils
BuildRequires:  gcc-c++
BuildRequires:  gettext-tools
BuildRequires:  git
BuildRequires:  meson
BuildRequires:  pam-devel
BuildRequires:  pkgconfig(bash-completion)
BuildRequires:  pkgconfig(colord)
BuildRequires:  pkgconfig(gbm)
BuildRequires:  pkgconfig(glesv2)
BuildRequires:  pkgconfig(glib-2.0) >= %{glib_version}
BuildRequires:  pkgconfig(glycin-2) >= 2.0.beta.2
BuildRequires:  pkgconfig(gnome-desktop-4)
BuildRequires:  pkgconfig(gnome-settings-daemon)
BuildRequires:  pkgconfig(gobject-introspection-1.0) >= %{gobject_introspection_version}
BuildRequires:  pkgconfig(graphene-gobject-1.0)
BuildRequires:  pkgconfig(gtk4) >= %{gtk4_version}
BuildRequires:  pkgconfig(gudev-1.0)
BuildRequires:  pkgconfig(hyprcursor) >= 0.1.13
BuildRequires:  pkgconfig(lcms2)
BuildRequires:  pkgconfig(libadwaita-1)
BuildRequires:  pkgconfig(libcanberra)
BuildRequires:  pkgconfig(libdisplay-info) >= 0.2
BuildRequires:  pkgconfig(libdrm) >= %{libdrm_version}
BuildRequires:  libxcvt
BuildRequires:  pkgconfig(libei-1.0) >= %{libei_version}
BuildRequires:  pkgconfig(libeis-1.0) >= %{libei_version}
BuildRequires:  pkgconfig(libinput) >= %{libinput_version}
BuildRequires:  pkgconfig(libpipewire-0.3) >= %{pipewire_version}
BuildRequires:  pkgconfig(libstartup-notification-1.0)
BuildRequires:  pkgconfig(libsystemd)
BuildRequires:  pkgconfig(libwacom)
BuildRequires:  pkgconfig(lua)
BuildRequires:  pkgconfig(pixman-1)
BuildRequires:  pkgconfig(sm)
BuildRequires:  pkgconfig(sysprof-capture-4)
BuildRequires:  pkgconfig(umockdev-1.0)
BuildRequires:  pkgconfig(udev)
BuildRequires:  pkgconfig(wayland-protocols) >= %{wayland_protocols_version}
BuildRequires:  pkgconfig(wayland-server) >= %{wayland_server_version}
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(xwayland)
BuildRequires:  python3dist(argcomplete)
BuildRequires:  python3dist(docutils)
%if %{with gnoblin_stack}
BuildRequires:  gnoblin-gsettings-desktop-schemas >= %{gsettings_desktop_schemas_version}
%endif
Requires:       glib2 >= %{glib_version}
Requires:       gnome-settings-daemon
Requires:       gnoblin-gsettings-desktop-schemas >= %{gsettings_desktop_schemas_version}
Requires:       polkit
Recommends:     Mesa-dri

%description
Gnoblin's patched Mutter, installed privately alongside the system compositor.

%package devel
Summary:        Headers for building Gnoblin Shell against its private Mutter
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description devel
Private headers and pkg-config files for Gnoblin builds.

%prep
%autosetup -S git -n mutter-%{tarball_version}

%build
export PKG_CONFIG_PATH=%{_datadir}/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
export GI_GIR_PATH=%{_datadir}/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}
/usr/bin/meson setup build . --buildtype=plain \
  --prefix=%{_prefix} --libdir=%{_libdir} --libexecdir=%{_libexecdir} \
  --bindir=%{_bindir} --sbindir=%{_sbindir} --includedir=%{_includedir} \
  --datadir=%{_datadir} --mandir=%{_mandir} --infodir=%{_infodir} \
  --localedir=%{_datadir}/locale --sysconfdir=%{_sysconfdir} \
  --localstatedir=%{_localstatedir} --sharedstatedir=%{_sharedstatedir} \
  --wrap-mode=nodownload --auto-features=enabled \
  -Dc_args='-std=gnu17 -fPIE' -Dcpp_args='-std=c++20 -fPIE' -Db_pie=false \
  -Dintrospection=true -Dtests=disabled -Ddocs=false -Dprofiler=false \
  -Dudev_dir=%{_prefix}/lib/udev
/usr/bin/meson compile -C build %{?_smp_mflags}

%install
DESTDIR=%{buildroot} /usr/bin/meson install -C build --no-rebuild
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
# The policy is globally visible because polkit resolves actions there.  Its
# Gnoblin action ID avoids taking ownership of the GNOME policy.
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
* Fri Sep 25 2026 Gnoblin contributors
- Initial openSUSE Tumbleweed adapter.
