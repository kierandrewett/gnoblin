# Private GNOME dependencies for legacy openSUSE targets.  The source archive
# is created by build-chain.sh only after every manifest download verifies its
# pinned SHA-256, then this RPM owns the installed private prefix.
%global _prefix /usr/lib/gnoblin
%global _compat_dir %{_prefix}/deps
%global debug_package %{nil}
%global __provides_exclude_from ^%{_compat_dir}/.*$
%global __requires_exclude ^%{_compat_dir}/.*$

Name:           gnoblin-compat-runtime
Version:        51.0
Release:        1%{?dist}
Summary:        Private GNOME compatibility runtime for Gnoblin
License:        GPL-2.0-or-later AND LGPL-2.1-or-later
URL:            https://github.com/kierandrewett/gnoblin
Source0:        gnoblin-compat-runtime-%{version}.tar.xz

%description
Pinned GNOME runtime libraries that Gnoblin needs on stable RPM distributions
whose repository interfaces are older than GNOME 51.  Files stay under
Gnoblin's private prefix and never replace stock GNOME libraries.

%prep
%setup -q -c -T
tar -xJf %{SOURCE0}

%install
install -d %{buildroot}%{_prefix}
cp -a deps %{buildroot}%{_prefix}/

%files
%{_compat_dir}/

%changelog
* Sat Sep 26 2026 Gnoblin contributors
- Package the private compatibility runtime for stable openSUSE targets.
