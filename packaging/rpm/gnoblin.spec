# Generated from nix/native-packages.nix; do not edit.
Name:           gnoblin
Version:        51.0
Release:        1%{?dist}
Summary:        Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kdrew7/gnoblin
BuildArch:      noarch
Requires:       gnoblin-gsettings-desktop-schemas >= 51.0
Requires:       gnoblin-mutter >= 51.0
Requires:       gnoblin-session >= 51.0
Requires:       gnoblin-shell >= 51.0
Requires:       gnoblin-gsettings-desktop-schemas < 52
Requires:       gnoblin-mutter < 52
Requires:       gnoblin-session < 52
Requires:       gnoblin-shell < 52
Requires:       gjs >= 1.85.90
Requires:       glib2 >= 2.86.0
Requires:       gnome-session
Requires:       gnome-settings-daemon
Requires:       libinput >= 1.31.0
Requires:       pipewire >= 1.6.0
Requires:       libwayland-client >= 1.26
Requires:       xdg-desktop-portal-gnome

%description
Installs the complete Gnoblin session while reusing compatible GNOME userspace.

%files
