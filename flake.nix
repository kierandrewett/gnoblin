{
  description = "Gnoblin session package and NixOS module";

  inputs = {
    # Track the release train that carries the current GNOME major. The source
    # revisions below remain pinned by gnome-versions.json.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Keep each release input independent.  The compatibility evaluations below
    # deliberately do not follow the rolling package set: they need to expose
    # a regression in a specific NixOS release channel.
    nixpkgs_25_05.url = "github:NixOS/nixpkgs/nixos-25.05";
    nixpkgs_25_11.url = "github:NixOS/nixpkgs/nixos-25.11";
    nixpkgs_26_05.url = "github:NixOS/nixpkgs/nixos-26.05";

    mutter-src = {
      url = "git+https://gitlab.gnome.org/GNOME/mutter.git?rev=138a14fbeef09d49ebf5be8a0cb83b042dd5c841";
      flake = false;
    };

    gnome-shell-src = {
      url = "git+https://gitlab.gnome.org/GNOME/gnome-shell.git?rev=2177bdf9624b2d285de7c1d34274073d3769d6b8";
      flake = false;
    };

    gsettings-desktop-schemas-src = {
      url = "git+https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas.git?rev=1db238b6a349ea7fae6f1c0713afe04d1bb7ea5c";
      flake = false;
    };

    gvdb = {
      url = "git+https://gitlab.gnome.org/GNOME/gvdb.git?rev=b54bc5da25127ef416858a3ad92e57159ff565b3";
      flake = false;
    };

    gvc = {
      url = "git+https://gitlab.gnome.org/GNOME/libgnome-volume-control.git?rev=d2442f455844e5292cb4a74ffc66ecc8d7595a9f";
      flake = false;
    };

    libshew = {
      url = "git+https://gitlab.gnome.org/GNOME/libshew.git?rev=ed782477cb5164320ae4f731d49bc5d475ab2a52";
      flake = false;
    };

    jasmineGjs = {
      url = "github:ptomato/jasmine-gjs/856465dddbd92e82e574891e1ebc79e17d7b708a";
      flake = false;
    };

  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      nixpkgs_25_05,
      nixpkgs_25_11,
      nixpkgs_26_05,
      ...
    }:
    let
      systems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      versions = builtins.fromJSON (builtins.readFile ./gnome-versions.json);
      gnoblinRelease = builtins.fromJSON (builtins.readFile ./gnoblin-version.json);
      nativePackages = import ./nix/native-packages.nix { inherit versions gnoblinRelease; };
      nixpkgsChannels = {
        nixos_25_05 = {
          release = "25.05";
          input = nixpkgs_25_05;
        };
        nixos_25_11 = {
          release = "25.11";
          input = nixpkgs_25_11;
        };
        nixos_26_05 = {
          release = "26.05";
          input = nixpkgs_26_05;
        };
        nixos_unstable = {
          release = "unstable";
          input = nixpkgs;
        };
      };
      mkGnoblin =
        pkgs:
        pkgs.callPackage ./nix/package.nix {
          gnoblinSrc = self.outPath;
          mutterSrc = inputs.mutter-src.outPath;
          gnomeShellSrc = inputs.gnome-shell-src.outPath;
          gsettingsDesktopSchemasSrc = inputs.gsettings-desktop-schemas-src.outPath;
          gvdbSrc = inputs.gvdb.outPath;
          gvcSrc = inputs.gvc.outPath;
          libshewSrc = inputs.libshew.outPath;
          jasmineGjsSrc = inputs.jasmineGjs.outPath;
          gnomeShell = pkgs.gnome-shell;
          gnomeSession = pkgs.gnome-session;
        };
      # These values evaluate a package and enabled NixOS module with the exact
      # release channel pinned in flake.lock. They do not build or run a
      # compositor, so a true result is not a session or coexistence claim.
      nixChannelEvaluations = forAllSystems (
        system:
        nixpkgs.lib.mapAttrs (
          _: channel:
          let
            pkgs = import channel.input { inherit system; };
            gnoblin = mkGnoblin pkgs;
            moduleTest = channel.input.lib.nixosSystem {
              inherit system;
              modules = [
                self.nixosModules.default
                {
                  system.stateVersion = channel.release;
                  programs.gnoblin = {
                    enable = true;
                    package = gnoblin;
                  };
                }
              ];
            };
            hasGcc16Stdenv = pkgs ? gcc16Stdenv;
            hasLibglycin = pkgs ? libglycin;
            packageEvaluation = if hasLibglycin then builtins.tryEval gnoblin.drvPath else { success = false; };
            moduleEvaluation =
              if hasLibglycin then
                builtins.tryEval (
                  toString (builtins.head moduleTest.config.services.displayManager.sessionPackages)
                )
              else
                { success = false; };
            buildBlockers =
              nixpkgs.lib.optional (!hasLibglycin) "missing-libglycin"
              ++ nixpkgs.lib.optional (!nixpkgs.lib.versionAtLeast pkgs.glib.version "2.86.0") "glib-below-2.86"
              ++ nixpkgs.lib.optional (!nixpkgs.lib.versionAtLeast pkgs.gjs.version "1.85.90") "gjs-below-1.85.90"
              ++ nixpkgs.lib.optional (
                !nixpkgs.lib.versionAtLeast pkgs.wayland.version "1.26"
              ) "wayland-below-1.26"
              ++ nixpkgs.lib.optional (
                !nixpkgs.lib.versionAtLeast pkgs.wayland-protocols.version "1.48"
              ) "wayland-protocols-below-1.48"
              ++ nixpkgs.lib.optional (
                !nixpkgs.lib.versionAtLeast pkgs.libinput.version "1.30.0"
              ) "libinput-below-1.30";
          in
          {
            inherit (channel) release;
            compiler = if hasGcc16Stdenv then "gcc16Stdenv" else "stdenv";
            evaluationBlocker = if hasLibglycin then null else "missing-libglycin";
            inherit buildBlockers;
            package.evaluates = packageEvaluation.success;
            module.evaluates = moduleEvaluation.success;
          }
        ) nixpkgsChannels
      );
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        rec {
          gnoblin = mkGnoblin pkgs;
          default = gnoblin;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          gnoblin = self.packages.${system}.gnoblin;
          combinedProfile = pkgs.buildEnv {
            name = "gnome-and-gnoblin";
            paths = [
              pkgs.gnome-shell
              pkgs.mutter
              gnoblin
            ];
            ignoreCollisions = false;
          };
          moduleTest = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                system.stateVersion = "25.11";
                programs.gnoblin.enable = true;
              }
            ];
          };
        in
        {
          gnoblin-session = pkgs.runCommand "gnoblin-session-check" { } ''
            set -euxo pipefail
            export GSETTINGS_BACKEND=memory

            test "${toString (builtins.head moduleTest.config.services.displayManager.sessionPackages)}" = "${gnoblin}"
            test "${toString (builtins.head moduleTest.config.systemd.packages)}" = "${gnoblin}"
            test ! -e "${gnoblin}/bin/gnome-shell"
            test ! -e "${gnoblin}/bin/mutter"
            test ! -e "${gnoblin}/share/glib-2.0/schemas"
            test ! -e "${gnoblin}/share/dbus-1"
            test -x "${gnoblin}/bin/gnoblinctl"
            test -x "${gnoblin.runtime}/bin/gnome-shell"
            test -x "${gnoblin.runtime}/bin/gnoblin-session"
            test -x "${gnoblin.runtime}/bin/gnoblin-shell-service"
            test "$(readlink -f ${combinedProfile}/bin/gnome-shell)" = "$(readlink -f ${pkgs.gnome-shell}/bin/gnome-shell)"
            test "$(readlink -f ${combinedProfile}/bin/mutter)" = "$(readlink -f ${pkgs.mutter}/bin/mutter)"
            test -f "${gnoblin}/share/wayland-sessions/gnoblin.desktop"
            test -f "${gnoblin}/lib/systemd/user/org.gnoblin.Shell@wayland.service"
            test -f "${pkgs.gnome-session}/share/systemd/user/gnome-session.target"
            test -f "${pkgs.gnome-session}/share/systemd/user/gnome-session@.target"
            test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell-disable-extensions.service"
            test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell.target"
            test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell@wayland.service"
            bash -n "${gnoblin.runtime}/bin/gnoblin-session" "${gnoblin.runtime}/bin/gnoblin-shell-service"
            case "$(<"${gnoblin.runtime}/bin/gnoblin-session")" in
                *"--no-reexec"*) ;;
                *) exit 1 ;;
            esac
            schema_directory="${gnoblin.runtime}/share/glib-2.0/schemas"
            test -f "$schema_directory/org.gnome.mutter.gschema.xml"
            test -f "$schema_directory/org.gnome.shell.gschema.xml"
            test -f "$schema_directory/org.gnoblin.shell.gschema.xml"
            test -f "$schema_directory/gschemas.compiled"
            test "$(
                GSETTINGS_SCHEMA_DIR="$schema_directory" ${pkgs.glib.bin}/bin/gsettings \
                    get org.gnome.mutter overlay-key
            )" = "'Super'"
            GSETTINGS_SCHEMA_DIR="$schema_directory" ${pkgs.glib.bin}/bin/gsettings \
                get org.gnome.shell enabled-extensions >/dev/null
            GSETTINGS_SCHEMA_DIR="$schema_directory" ${pkgs.glib.bin}/bin/gsettings \
                get org.gnoblin.shell disabled-features >/dev/null
            touch "$out"
          '';
        }
      );

      lib = {
        inherit nativePackages nixChannelEvaluations;
      };

      nixosModules.default = import ./nix/module.nix { inherit self; };
    };
}
