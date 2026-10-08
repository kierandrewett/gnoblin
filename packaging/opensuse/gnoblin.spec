# Standalone Gnoblin session package for openSUSE Tumbleweed.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _datadir %{_prefix}/share
%global glib_version 2.81.1
%global gobject_introspection_version 1.41.4
%global gtk4_version 4.14.0
%global gsettings_desktop_schemas_version 49.1
%global libdrm_version 2.4.118
%global libinput_version 1.30.0
%global pipewire_version 1.4.11
%global libei_version 1.3.901
%global wayland_protocols_version 1.48
%global wayland_server_version 1.25
%global debug_package %{nil}
%global __provides_exclude_from ^%{_prefix}/.*$
%global __requires_exclude ^(lib(mutter[^()]*|shell-[0-9]+|st-[0-9]+)[.]so.*|pkgconfig[(](libmutter|mutter-)[^)]*[)]|typelib[(](Clutter|Cogl|Mtk|Shell|St)[)]([[:space:]]*=[[:space:]]*.*)?)$

Name:           gnoblin
Version:        %{gnoblin_version}
Epoch:          1
Release:        22%{?dist}
Summary:        Standalone Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gnoblin-%{version}-source.tar.xz
Provides:       gnoblin-session = %{version}
Obsoletes:      gnoblin-session <= %{version}-%{release}
Obsoletes:      gnoblin-mutter < 52
Obsoletes:      gnoblin-mutter-devel < 52

Requires:       adwaita-icon-theme
Requires:       dbus-1
Requires:       dconf
Requires:       glib2 >= 2.86
Requires:       gsettings-desktop-schemas >= 49.1
Requires:       json-glib
Requires:       libinput10 >= 1.30
Requires:       pipewire >= 1.4.11
Requires:       systemd-libs
Requires:       libwayland-client0 >= 1.25
Requires:       wireplumber

BuildRequires:  cmake
BuildRequires:  gcc-c++
BuildRequires:  gettext-tools
BuildRequires:  git
BuildRequires:  meson
BuildRequires:  pam-devel
BuildRequires:  pkgconfig(atk)
BuildRequires:  pkgconfig(colord)
BuildRequires:  pkgconfig(gbm)
BuildRequires:  pkgconfig(gcr-4)
BuildRequires:  pkgconfig(ibus-1.0) >= 1.5.33
BuildRequires:  pkgconfig(glesv2)
BuildRequires:  pkgconfig(glib-2.0) >= %{glib_version}
BuildRequires:  pkgconfig(glycin-2) >= 2.0.beta.2
BuildRequires:  pkgconfig(gobject-introspection-1.0) >= %{gobject_introspection_version}
BuildRequires:  pkgconfig(graphene-gobject-1.0)
BuildRequires:  pkgconfig(gtk4) >= %{gtk4_version}
BuildRequires:  pkgconfig(gudev-1.0)
BuildRequires:  pkgconfig(egl)
BuildRequires:  pkgconfig(fribidi)
BuildRequires:  pkgconfig(gl)
BuildRequires:  pkgconfig(harfbuzz) >= 2.6
BuildRequires:  pkgconfig(lcms2)
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
BuildRequires:  pkgconfig(libudev) >= 228
BuildRequires:  pkgconfig(libwacom)
BuildRequires:  pkgconfig(pango) >= 1.46.0
BuildRequires:  pkgconfig(pangocairo) >= 1.20
BuildRequires:  pkgconfig(pixman-1)
BuildRequires:  pkgconfig(sm)
BuildRequires:  pkgconfig(wayland-client) >= %{wayland_server_version}
BuildRequires:  pkgconfig(wayland-cursor)
BuildRequires:  pkgconfig(wayland-egl)
BuildRequires:  pkgconfig(x11) >= 1.7.0
BuildRequires:  pkgconfig(x11-xcb)
BuildRequires:  pkgconfig(xau)
BuildRequires:  pkgconfig(xcb-res)
BuildRequires:  pkgconfig(xcomposite) >= 0.4
BuildRequires:  pkgconfig(xcursor)
BuildRequires:  pkgconfig(xdamage)
BuildRequires:  pkgconfig(xext)
BuildRequires:  pkgconfig(xfixes) >= 6
BuildRequires:  pkgconfig(xinerama)
BuildRequires:  pkgconfig(xi) >= 1.7.4
BuildRequires:  pkgconfig(xrandr) >= 1.5.0
BuildRequires:  pkgconfig(xkbregistry)
BuildRequires:  pkgconfig(xkeyboard-config)
BuildRequires:  pkgconfig(udev)
BuildRequires:  pkgconfig(wayland-protocols) >= %{wayland_protocols_version}
BuildRequires:  pkgconfig(wayland-server) >= %{wayland_server_version}
BuildRequires:  pkgconfig(xwayland)
BuildRequires:  python3dist(docutils)
BuildRequires:  pkgconfig(gsettings-desktop-schemas) >= %{gsettings_desktop_schemas_version}
BuildRequires:  desktop-file-utils
BuildRequires:  gcc
BuildRequires:  glib2-devel >= 2.86
BuildRequires:  json-glib-devel
BuildRequires:  ninja
BuildRequires:  pkgconfig(gio-2.0)
BuildRequires:  pkgconfig(gio-unix-2.0)
BuildRequires:  pkgconfig(json-glib-1.0)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(lua) >= 5.4
BuildRequires:  python3

%description
Installs the Lua-supervised Gnoblin session, patched Mutter runtime, and login
entry as one package with the single `gnoblin` compositor executable. It does
not require GNOME Shell or GJS. The GTK-based Gnoblin portal backend is
optional.

%prep
%autosetup -n gnoblin-%{version}

%build
cmake -S . -B build/session -G Ninja \
  -DGNOBLIN_PREFIX=%{_prefix} -DGNOBLIN_LIBDIR=%{_lib} \
  -DGNOBLIN_STAGE_ROOT=%{_builddir}/gnoblin-stage \
  -DGNOBLIN_BUILD_TYPE=release -DGNOBLIN_SOURCE_MODE=release-archive \
  -DGNOBLIN_JOBS=%{?_smp_build_ncpus}
cmake --build build/session --target mutter gnoblin-idle gnoblinctl \
  --parallel %{?_smp_build_ncpus}

%install
if [ -d %{_builddir}/gnoblin-stage ]; then
  cp -a %{_builddir}/gnoblin-stage/. %{buildroot}/
fi
GNOBLIN_LIBDIR=%{_lib} \
GNOBLIN_STAGE_ROOT=%{buildroot} \
GNOBLIN_IDLE_BINARY="$PWD/build/session/gnoblin-idle" \
GNOBLINCTL_BINARY="$PWD/build/session/gnoblinctl" \
GNOBLIN_IDENTITY_FILE="$PWD/build/session/gnoblinctl-identity.json" \
GNOBLIN_VERSION_METADATA_FILE="$PWD/build/session/gnoblin-version.ini" \
  scripts/install-session.sh %{_prefix}
install -Dm0644 %{buildroot}%{_prefix}/share/xdg-desktop-portal/gnoblin-portals.conf \
  %{buildroot}/usr/share/xdg-desktop-portal/gnoblin-portals.conf
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled
if [ -f %{buildroot}%{_datadir}/polkit-1/actions/org.gnome.mutter.backlight-helper.policy ]; then
  install -d %{buildroot}/usr/share/polkit-1/actions
  sed 's/org.gnome.mutter.backlight-helper/org.gnoblin.mutter.backlight-helper/g' \
    %{buildroot}%{_datadir}/polkit-1/actions/org.gnome.mutter.backlight-helper.policy \
    > %{buildroot}/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy
fi
install -d %{buildroot}/usr/bin %{buildroot}/usr/share/wayland-sessions
ln -s %{_prefix}/bin/gnoblin %{buildroot}/usr/bin/gnoblin
ln -s %{_prefix}/bin/gnoblinctl %{buildroot}/usr/bin/gnoblinctl
install -m 0644 %{buildroot}%{_datadir}/wayland-sessions/gnoblin.desktop \
  %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop
install -Dm644 %{buildroot}%{_prefix}/lib/systemd/user/gnoblin-session.target \
  %{buildroot}/usr/lib/systemd/user/gnoblin-session.target
install -Dm644 %{buildroot}%{_prefix}/lib/systemd/user/gnoblin-idle.service \
  %{buildroot}/usr/lib/systemd/user/gnoblin-idle.service
sed -i -e 's|^Exec=.*|Exec=%{_prefix}/bin/gnoblin|' \
  -e 's|^DesktopNames=.*|DesktopNames=Gnoblin;|' \
  %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop

%posttrans
/usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas

%postun
if [ -d %{_datadir}/glib-2.0/schemas ]; then
  /usr/bin/glib-compile-schemas %{_datadir}/glib-2.0/schemas
fi

%check
sed '/^DesktopNames=/d' %{buildroot}/usr/share/wayland-sessions/gnoblin.desktop \
  > gnoblin-validation.desktop
desktop-file-validate gnoblin-validation.desktop

%files
%license COPYING
%{_prefix}/
/usr/bin/gnoblin
/usr/bin/gnoblinctl
/usr/share/wayland-sessions/gnoblin.desktop
/usr/share/xdg-desktop-portal/gnoblin-portals.conf
/usr/lib/systemd/user/gnoblin-session.target
/usr/lib/systemd/user/gnoblin-idle.service
/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy

%package -n gnoblin-gnome-integration
Summary:        Optional GNOME application services for Gnoblin
Requires:       gnoblin = 1:%{version}-%{release}
Requires:       gvfs
Requires:       gnome-keyring
Requires:       xdg-user-dirs

%description -n gnoblin-gnome-integration
Adds GNOME Keyring, GVfs, and standard user directories to a Gnoblin session.
Applications are installed separately.

%files -n gnoblin-gnome-integration
