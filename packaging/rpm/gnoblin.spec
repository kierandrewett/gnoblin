# Generated from packaging/native-packages.json; do not edit.
Name:           gnoblin
Version:        0.1.7
Epoch:          1
Release:        20%{?dist}
Summary:        Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
BuildArch:      noarch
Requires:       gnoblin-mutter >= 0.1.7
Requires:       gnoblin-portal >= 0.1.7
Requires:       gnoblin-session >= 0.1.7
Requires:       gnoblin-shell >= 0.1.7
Requires:       gnoblin-mutter < 52
Requires:       gnoblin-portal < 52
Requires:       gnoblin-session < 52
Requires:       gnoblin-shell < 52
Requires:       accountsservice-libs
Requires:       adwaita-cursor-theme
Requires:       dbus-tools
Requires:       dconf
Requires:       gjs >= 1.87.1
Requires:       glib2 >= 2.86.0
Requires:       gsettings-desktop-schemas >= 51.0
Requires:       ibus-libs
Requires:       json-glib
Requires:       libinput >= 1.31.0
Requires:       pipewire >= 1.6.0
Requires:       libwayland-client >= 1.26
Requires:       wireplumber
Requires:       xdg-desktop-portal >= 1.21.1
Requires:       gnoblin-mutter = 51.0-44.gnoblin%{?dist}

%package -n gnoblin-gnome-integration
Summary:        Optional GNOME application services for Gnoblin
Requires:       gnoblin-session >= 51
Requires:       gnoblin-session < 52
Requires:       gvfs
Requires:       gnome-keyring
Requires:       xdg-user-dirs

%description
Installs the complete Gnoblin session while reusing compatible GNOME userspace.

%description -n gnoblin-gnome-integration
Adds GNOME Keyring, GVfs, and standard user directories to a Gnoblin session.
Applications are installed separately.

%files

%files -n gnoblin-gnome-integration
