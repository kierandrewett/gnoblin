Name:           gnoblin-vr
Version:        0.1.0
Release:        1%{?dist}
Summary:        Optional OpenXR presentation client for Gnoblin
License:        GPL-2.0-or-later
BuildRequires:  cmake
BuildRequires:  gcc
BuildRequires:  glib2-devel
BuildRequires:  openxr-devel
BuildRequires:  pkgconfig
BuildRequires:  python3
Requires:       glib2
Requires:       openxr-libs
Source0:        gnoblin-vr-%{version}.tar.gz

%description
An optional, extension-owned OpenXR presentation client for Gnoblin.

%prep
%autosetup -n gnoblin-vr-%{version}

%build
%cmake -DCMAKE_BUILD_TYPE=Release
%cmake_build

%check
%ctest

%install
%cmake_install

%files
%{_bindir}/gnoblin-vr
%{_userunitdir}/gnoblin-vr.service
%{_datadir}/gnoblin-vr/lua/gnoblin-vr.lua
