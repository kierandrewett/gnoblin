{ versions }:
let
  component = name: versions.components.${name};
  requirement = minVersion: rpm: deb: arch: {
    inherit minVersion;
    names = { inherit rpm deb arch; };
  };
in
{
  formatVersion = 1;
  release = {
    gnomeMajor = versions.major;
    mutterApi = (component "mutter").api;
  };

  sources = builtins.mapAttrs (_: value: {
    inherit (value) version commit;
  }) versions.components;

  requirements = {
    glib = requirement "2.86.0" "glib2" "libglib2.0-0t64" "glib2";
    gjs = requirement "1.87.1" "gjs" "gjs" "gjs";
    gsettings-desktop-schemas =
      requirement "51.rc" "gsettings-desktop-schemas" "gsettings-desktop-schemas"
        "gsettings-desktop-schemas";
    gnome-session = requirement "51.0" "gnome-session" "gnome-session" "gnome-session";
    gnome-settings-daemon =
      requirement "51.0" "gnome-settings-daemon" "gnome-settings-daemon"
        "gnome-settings-daemon";
    xdg-desktop-portal-gnome =
      requirement "51.0" "xdg-desktop-portal-gnome" "xdg-desktop-portal-gnome"
        "xdg-desktop-portal-gnome";
    wayland = requirement "1.26" "wayland" "libwayland-client0" "wayland";
    wayland-protocols =
      requirement "1.48" "wayland-protocols-devel" "wayland-protocols"
        "wayland-protocols";
    libinput = requirement "1.31.0" "libinput" "libinput10" "libinput";
    pipewire = requirement "1.6.0" "pipewire" "pipewire" "pipewire";
  };

  packages = {
    gnoblin-mutter = {
      version = (component "mutter").version;
      source = "mutter";
      requires = [
        "glib"
        "gsettings-desktop-schemas"
        "gnome-settings-daemon"
        "wayland"
        "libinput"
        "pipewire"
      ];
    };
    gnoblin-mutter-devel = {
      version = (component "mutter").version;
      source = "mutter";
      requiresExact = [ "gnoblin-mutter" ];
      requires = [ "wayland-protocols" ];
    };
    gnoblin-shell = {
      version = (component "gnome-shell").version;
      source = "gnome-shell";
      requiresExact = [ "gnoblin-mutter" ];
      requires = [
        "glib"
        "gjs"
        "gsettings-desktop-schemas"
        "gnome-settings-daemon"
        "xdg-desktop-portal-gnome"
      ];
    };
    gnoblin-session = {
      version = (component "gnome-shell").version;
      source = "gnome-shell";
      requiresExact = [ "gnoblin-shell" ];
      requires = [ "gnome-session" ];
    };
  };
}
