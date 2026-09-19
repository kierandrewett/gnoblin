{ versions }:
let
  component = name: versions.components.${name};
  requirement = minVersion: rpm: deb: arch: {
    inherit minVersion;
    names = { inherit rpm deb arch; };
  };
  requirements = {
    glib = requirement "2.86.0" "glib2" "libglib2.0-0t64" "glib2";
    gjs = requirement "1.85.90" "gjs" "gjs" "gjs";
    gsettings-desktop-schemas =
      requirement "51.0" "gsettings-desktop-schemas" "gsettings-desktop-schemas"
        "gsettings-desktop-schemas";
    gnome-session = requirement null "gnome-session" "gnome-session" "gnome-session";
    gnome-settings-daemon =
      requirement null "gnome-settings-daemon" "gnome-settings-daemon"
        "gnome-settings-daemon";
    xdg-desktop-portal-gnome =
      requirement null "xdg-desktop-portal-gnome" "xdg-desktop-portal-gnome"
        "xdg-desktop-portal-gnome";
    wayland = requirement "1.26" "wayland" "libwayland-client0" "wayland";
    wayland-protocols =
      requirement "1.48" "wayland-protocols-devel" "wayland-protocols"
        "wayland-protocols";
    libinput = requirement "1.31.0" "libinput" "libinput10" "libinput";
    pipewire = requirement "1.6.0" "pipewire" "pipewire" "pipewire";
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

  inherit requirements;

  packages = {
    gnoblin = {
      version = (component "gnome-shell").version;
      meta = true;
      requiresSameMajor = [ "gnoblin-session" ];
      requires = [ ];
    };
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
      requiresSameMajor = [ "gnoblin-mutter" ];
      requires = [ "wayland-protocols" ];
    };
    gnoblin-shell = {
      version = (component "gnome-shell").version;
      source = "gnome-shell";
      requiresSameMajor = [ "gnoblin-mutter" ];
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
      requiresSameMajor = [ "gnoblin-shell" ];
      requires = [ "gnome-session" ];
    };
  };
}
