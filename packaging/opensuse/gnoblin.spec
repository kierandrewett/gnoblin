Name:           gnoblin
Version:        0.1.7
Release:        2%{?dist}
Summary:        Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
BuildArch:      noarch
Requires:       gsettings-desktop-schemas >= 51
Requires:       gnoblin-mutter >= 51
Requires:       gnoblin-session >= 51
Requires:       adwaita-icon-theme
Requires:       dconf
Requires:       gnoblin-shell >= 51
Requires:       gnoblin-portal >= 51
Requires:       gjs >= 1.87.1
Requires:       glib2 >= 2.86
Requires:       libinput10 >= 1.31
Requires:       pipewire >= 1.6
Requires:       libwayland-client0 >= 1.26
Requires:       wireplumber

%description
Installs the complete Gnoblin session while reusing compatible GNOME userspace.

%package -n gnoblin-gnome-integration
Summary:        Optional GNOME application services for Gnoblin
Requires:       gnoblin-session >= 51
Requires:       gnoblin-session < 52
Requires:       gvfs
Requires:       gnome-keyring
Requires:       xdg-user-dirs

%description -n gnoblin-gnome-integration
Adds GNOME Keyring, GVfs, and standard user directories to a Gnoblin session.
Applications are installed separately.

%files

%files -n gnoblin-gnome-integration

%changelog
* Sun Sep 27 2026 Gnoblin contributors
- Keep the GTK folder-name updater out of the optional integration package.

* Fri Sep 25 2026 Gnoblin contributors
- Initial openSUSE Tumbleweed adapter.
