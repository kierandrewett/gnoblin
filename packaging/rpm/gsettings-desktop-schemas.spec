%global _prefix /usr/lib/gnoblin
%global _datadir %{_prefix}/share
%global _includedir %{_prefix}/include
# This package is a private build dependency and runtime. It must not satisfy
# stock Fedora pkgconfig dependencies, whose default search path excludes it.
%global __provides_exclude ^pkgconfig[(]gsettings-desktop-schemas[)]

Name:           gnoblin-gsettings-desktop-schemas
Version:        51.0
Release:        1%{?dist}
Summary:        Private GNOME desktop schemas for Gnoblin
License:        LGPL-2.1-or-later
URL:            https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas
Source0:        gsettings-desktop-schemas-%{version}.tar.xz
BuildArch:      noarch

BuildRequires:  gettext
BuildRequires:  gcc
BuildRequires:  glib2-devel
BuildRequires:  meson
Requires:       glib2

%description
GNOME desktop GSettings schemas installed in Gnoblin's private prefix. This
allows Gnoblin to use the matching GNOME major without replacing the host
desktop's schemas.

%prep
%autosetup -n gsettings-desktop-schemas-%{version}

%build
%meson -Dintrospection=false
%meson_build

%install
%meson_install
rm -f %{buildroot}%{_datadir}/glib-2.0/schemas/gschemas.compiled

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

%changelog
* Sun Sep 20 2026 Gnoblin contributors - 51.0-1
- Package GNOME 51 schemas privately for parallel installation.
