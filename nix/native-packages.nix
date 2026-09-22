{ versions, gnoblinRelease }:
let
  component = name: versions.components.${name};
  requirement = minVersion: rpm: deb: arch: {
    inherit minVersion;
    names = { inherit rpm deb arch; };
  };
  requirements = {
    glib = requirement "2.86.0" "glib2" "libglib2.0-0t64" "glib2";
    gjs = requirement "1.85.90" "gjs" "gjs" "gjs";
    gnome-session = requirement null "gnome-session" "gnome-session" "gnome-session";
    gnome-settings-daemon =
      requirement null "gnome-settings-daemon" "gnome-settings-daemon"
        "gnome-settings-daemon";
    xdg-desktop-portal-gnome =
      requirement null "xdg-desktop-portal-gnome" "xdg-desktop-portal-gnome"
        "xdg-desktop-portal-gnome";
    wayland = requirement "1.26" "libwayland-client" "libwayland-client0" "wayland";
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
    gnomeVersion = (component "gnome-shell").version;
    gnoblinVersion = gnoblinRelease.version;
    # Keep the meta package coupled to the private Mutter build that contains
    # the current compositor fixes.  This is an RPM release, not a GNOME ABI.
    mutterRpmRelease = "20.gnoblin";
    # Older COPR metadata used 51.0 as the Gnoblin package version.  Preserve
    # a clean public version while making this package sortable as its successor.
    rpmEpoch = 1;
    mutterApi = (component "mutter").api;
  };

  sources = builtins.mapAttrs (_: value: {
    inherit (value) version commit;
  }) versions.components;

  inherit requirements;

  packages = {
    gnoblin = {
      version = gnoblinRelease.version;
      meta = true;
      requiresSameMajor = [ "gnoblin-session" ];
      requires = [ ];
    };
    gnoblin-gsettings-desktop-schemas = {
      version = (component "gsettings-desktop-schemas").version;
      source = "gsettings-desktop-schemas";
      requires = [ "glib" ];
    };
    gnoblin-mutter = {
      version = (component "mutter").version;
      source = "mutter";
      requiresSameMajor = [ "gnoblin-gsettings-desktop-schemas" ];
      requires = [
        "glib"
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
      requiresSameMajor = [
        "gnoblin-gsettings-desktop-schemas"
        "gnoblin-mutter"
      ];
      requires = [
        "glib"
        "gjs"
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
