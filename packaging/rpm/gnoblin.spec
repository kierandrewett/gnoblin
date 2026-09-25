# Generated from nix/native-packages.nix; do not edit.
Name:           gnoblin
Version:        0.1.7
Epoch:          1
Release:        1%{?dist}
Summary:        Gnoblin desktop session
License:        GPL-2.0-or-later
URL:            https://github.com/kierandrewett/gnoblin
BuildArch:      noarch
Requires:       gnoblin-gsettings-desktop-schemas >= 0.1.7
Requires:       gnoblin-mutter >= 0.1.7
Requires:       gnoblin-session >= 0.1.7
Requires:       gnoblin-shell >= 0.1.7
Requires:       gnoblin-gsettings-desktop-schemas < 52
Requires:       gnoblin-mutter < 52
Requires:       gnoblin-session < 52
Requires:       gnoblin-shell < 52
Requires:       brightnessctl
Requires:       gjs >= 1.85.90
Requires:       glib2 >= 2.86.0
Requires:       gnome-session
Requires:       gnome-settings-daemon
Requires:       libinput >= 1.30.0
Requires:       pipewire >= 1.4.0
Requires:       playerctl
Requires:       libwayland-client >= 1.26
Requires:       wireplumber
Requires:       xdg-desktop-portal-gnome
Requires:       gnoblin-mutter = 51.0-20.gnoblin%{?dist}

%description
Installs the complete Gnoblin session while reusing compatible GNOME userspace.

%files
