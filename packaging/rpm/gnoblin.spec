# Generated from this template and packaging/native-packages.json.
%global _prefix /usr/lib/gnoblin
%global _libdir %{_prefix}/%{_lib}
%global _datadir %{_prefix}/share

Name:           gnoblin
Version:        0.1.10
Epoch:          1
Release:        21%{?dist}
Summary:        Standalone Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gnoblin-%{version}-source.tar.xz
Provides:       gnoblin-session = %{version}
Obsoletes:      gnoblin-session <= %{version}-%{release}

Requires:       gnoblin-mutter >= 51
Requires:       gnoblin-mutter < 52
Requires:       adwaita-cursor-theme
Requires:       dbus-tools
Requires:       dconf
Requires:       glib2 >= 2.86.0
Requires:       gsettings-desktop-schemas >= 49.1
Requires:       json-glib
Requires:       libinput >= 1.30.0
Requires:       lua-libs >= 5.4
Requires:       pipewire >= 1.4.11
Requires:       systemd
Requires:       libwayland-client >= 1.25
Requires:       wireplumber
Requires:       gnoblin-mutter = 51.0-44.gnoblin%{?dist}

BuildRequires:  cmake
BuildRequires:  desktop-file-utils
BuildRequires:  gcc
BuildRequires:  glib2-devel >= 2.86
BuildRequires:  gnoblin-mutter-devel >= 51
BuildRequires:  gnoblin-mutter-devel < 52
BuildRequires:  json-glib-devel
BuildRequires:  ninja-build
BuildRequires:  pkgconfig(gio-2.0)
BuildRequires:  pkgconfig(gio-unix-2.0)
BuildRequires:  pkgconfig(json-glib-1.0)
BuildRequires:  pkgconfig(lua) >= 5.4
BuildRequires:  python3

%description
Installs the Lua-supervised Gnoblin session, private runtime tools, and login
entry. The package uses the separately packaged Gnoblin Mutter and does not
require GNOME Shell or GJS. The GTK-based Gnoblin portal backend is optional.
It replaces the earlier
gnoblin-session payload package on upgrade.

%prep
%autosetup -n gnoblin-%{version}

%build
cmake -S . -B build/session -G Ninja \
  -DGNOBLIN_PREFIX=%{_prefix} -DGNOBLIN_LIBDIR=%{_lib} \
  -DGNOBLIN_BUILD_TYPE=release -DGNOBLIN_SOURCE_MODE=release-archive \
  -DGNOBLIN_JOBS=%{?_smp_build_ncpus}
cmake --build build/session --target gnoblin gnoblin-idle gnoblinctl \
  --parallel %{?_smp_build_ncpus}

%install
mkdir -p %{buildroot}%{_datadir}/glib-2.0/schemas
cp -a %{_datadir}/glib-2.0/schemas/*.xml %{buildroot}%{_datadir}/glib-2.0/schemas/
GNOBLIN_LIBDIR=%{_lib} \
GNOBLIN_STAGE_ROOT=%{buildroot} \
GNOBLIN_IDLE_BINARY="$PWD/build/session/gnoblin-idle" \
GNOBLINCTL_BINARY="$PWD/build/session/gnoblinctl" \
GNOBLIN_IDENTITY_FILE="$PWD/build/session/gnoblinctl-identity.json" \
GNOBLIN_VERSION_METADATA_FILE="$PWD/build/session/gnoblin-version.ini" \
GNOBLIN_BINARY="$PWD/build/session/gnoblin" \
  scripts/install-session.sh %{_prefix}
# The Mutter schemas are available while the aggregate schema is compiled but
# belong to gnoblin-mutter. Keep only the session-owned override in this RPM.
find %{buildroot}%{_datadir}/glib-2.0/schemas -maxdepth 1 -type f -name '*.xml' \
  ! -name '00_org.gnoblin.mutter.gschema.override' -delete
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
/usr/lib/systemd/user/gnoblin-session.target
/usr/lib/systemd/user/gnoblin-idle.service

%package -n gnoblin-gnome-integration
Summary:        Optional GNOME application services for Gnoblin
Requires:       gnoblin = 1:0.1.10
Requires:       gvfs
Requires:       gnome-keyring
Requires:       xdg-user-dirs

%description -n gnoblin-gnome-integration
Adds GNOME Keyring, GVfs, and standard user directories to a Gnoblin session.
Applications are installed separately.

%files -n gnoblin-gnome-integration
