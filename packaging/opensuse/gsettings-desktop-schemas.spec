# openSUSE Tumbleweed package recipe.  This package deliberately keeps GNOME
# 51 schemas under Gnoblin's prefix, so it cannot replace the host schemas.
%global _prefix /usr/lib/gnoblin
%global _datadir %{_prefix}/share
%global _includedir %{_prefix}/include
%global debug_package %{nil}

Name:           gnoblin-gsettings-desktop-schemas
Version:        51.0
Release:        1%{?dist}
Summary:        Private GNOME desktop schemas for Gnoblin
License:        LGPL-2.1-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gsettings-desktop-schemas-%{version}.tar.xz
BuildRequires:  gcc
BuildRequires:  gettext-tools
BuildRequires:  meson
BuildRequires:  pkgconfig(gio-2.0)
BuildRequires:  pkgconfig(gobject-introspection-1.0)
Requires:       glib2

%description
GNOME desktop GSettings schemas installed in Gnoblin's private prefix.  They
provide the matching GNOME major without replacing the host desktop schemas.

%prep
%autosetup -n gsettings-desktop-schemas-%{version}

%build
%meson
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
* Fri Sep 25 2026 Gnoblin contributors
- Initial openSUSE Tumbleweed adapter.
