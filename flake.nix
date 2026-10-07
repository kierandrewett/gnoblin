{
  description = "Gnoblin session package and NixOS module";

  inputs = {
    # Track the release train that carries the current GNOME major. Upstream
    # component versions and source commits are pinned by gnome-versions.json.
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

    imgui-src = {
      url = "git+https://github.com/ocornut/imgui.git?rev=f1cc2ae15e53a861a874c3034aae6798fde194ab";
      flake = false;
    };

    gsettings-desktop-schemas-src = {
      url = "git+https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas.git?rev=1db238b6a349ea7fae6f1c0713afe04d1bb7ea5c";
      flake = false;
    };

    portal-src = {
      url = "git+https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome.git?rev=99446c9c9d8197ac4651c5a8e1ee9eb771e1e120";
      flake = false;
    };

    gxdp-src = {
      url = "git+https://gitlab.gnome.org/GNOME/libgxdp.git?rev=df896e3412b749947bc6f62a91a1aac8e6b6d19b";
      flake = false;
    };

    gvdb = {
      url = "git+https://gitlab.gnome.org/GNOME/gvdb.git?rev=b54bc5da25127ef416858a3ad92e57159ff565b3";
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
      nativePackages = builtins.fromJSON (builtins.readFile ./packaging/native-packages.json);
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
        pkgs: overrides:
        pkgs.callPackage ./nix/package.nix (
          {
            gnoblinSrc = self.outPath;
            gnoblinRevision =
              self.rev or (if self ? dirtyRev then builtins.substring 0 40 self.dirtyRev else null);
            gnoblinSourceModified = self ? dirtyRev;
            gnoblinRemote = if self ? original && self.original ? url then self.original.url else null;
            mutterSrc = inputs.mutter-src.outPath;
            imguiSrc = inputs.imgui-src.outPath;
            gsettingsDesktopSchemasSrc = inputs.gsettings-desktop-schemas-src.outPath;
            portalSrc = inputs.portal-src.outPath;
            gxdpSrc = inputs.gxdp-src.outPath;
            gnomePortal = pkgs.xdg-desktop-portal-gnome;
            gvdbSrc = inputs.gvdb.outPath;
          }
          // overrides
        );
      assessChannel =
        system: channel:
        let
          pkgs = import channel.input { inherit system; };
          hasGcc16Stdenv = pkgs ? gcc16Stdenv;
          hasLibglycin = pkgs ? libglycin;
          buildBlockers =
            nixpkgs.lib.optional (!hasLibglycin) "missing-libglycin"
            ++ nixpkgs.lib.optional (!nixpkgs.lib.versionAtLeast pkgs.glib.version "2.86.0") "glib-below-2.86"
            ++ nixpkgs.lib.optional (
              !nixpkgs.lib.versionAtLeast pkgs.wayland.version "1.25"
            ) "wayland-below-1.25"
            ++ nixpkgs.lib.optional (
              !nixpkgs.lib.versionAtLeast pkgs.wayland-protocols.version "1.48"
            ) "wayland-protocols-below-1.48"
            ++ nixpkgs.lib.optional (
              !nixpkgs.lib.versionAtLeast pkgs.libinput.version "1.30.0"
            ) "libinput-below-1.30"
            ++ nixpkgs.lib.optional (
              !nixpkgs.lib.versionAtLeast pkgs.pipewire.version "1.4.11"
            ) "pipewire-below-1.4.11";
        in
        {
          inherit
            pkgs
            hasGcc16Stdenv
            hasLibglycin
            buildBlockers
            ;
        };
      mkChannelPackage =
        system: channel:
        let
          assessment = assessChannel system channel;
        in
        if assessment.buildBlockers == [ ] then
          mkGnoblin assessment.pkgs { }
        else
          throw ''
            Gnoblin does not currently provide an installable package for NixOS ${channel.release}.
            This channel is blocked by: ${nixpkgs.lib.concatStringsSep ", " assessment.buildBlockers}.

            Install a channel whose development libraries meet the pinned GNOME
            requirements. See docs/install-nixos.md and lib.nixChannelEvaluations.
          '';
      # These values evaluate a package and enabled NixOS module with the exact
      # release channel pinned in flake.lock. They do not build or run a
      # compositor, so a true result is not a session or coexistence claim.
      nixChannelEvaluations = forAllSystems (
        system:
        nixpkgs.lib.mapAttrs (
          _: channel:
          let
            assessment = assessChannel system channel;
            inherit (assessment)
              pkgs
              hasGcc16Stdenv
              hasLibglycin
              buildBlockers
              ;
            gnoblin = if buildBlockers == [ ] then mkGnoblin pkgs { } else null;
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
            packageEvaluation =
              if buildBlockers == [ ] then builtins.tryEval gnoblin.drvPath else { success = false; };
            moduleEvaluation =
              if buildBlockers == [ ] then
                builtins.tryEval (
                  toString (builtins.head moduleTest.config.services.displayManager.sessionPackages)
                )
              else
                { success = false; };
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
      nixChannelPackages = forAllSystems (
        system: nixpkgs.lib.mapAttrs (_: channel: mkChannelPackage system channel) nixpkgsChannels
      );
    in
    {
      packages = forAllSystems (
        system:
        let
          assessment = assessChannel system nixpkgsChannels.nixos_unstable;
        in
        if assessment.buildBlockers == [ ] then
          rec {
            gnoblin = mkGnoblin assessment.pkgs { };
            gnoblin-runtime = gnoblin;
            default = gnoblin;
          }
        else
          { }
      );

      checks = forAllSystems (
        system:
        let
          assessment = assessChannel system nixpkgsChannels.nixos_unstable;
          pkgs = import nixpkgs { inherit system; };
          gnoblin = if assessment.buildBlockers == [ ] then self.packages.${system}.gnoblin else null;
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
        if assessment.buildBlockers == [ ] then
          {
            gnoblin-session = pkgs.runCommand "gnoblin-session-check" { } ''
              set -euxo pipefail
              export GSETTINGS_BACKEND=memory

              test "${toString (builtins.head moduleTest.config.services.displayManager.sessionPackages)}" = "${gnoblin}"
              test "${toString (builtins.head moduleTest.config.systemd.packages)}" = "${gnoblin}"
              test ! -e "${gnoblin}/bin/gnome-shell"
              test ! -e "${gnoblin}/bin/gnoblin-shell-service"
              test -x "${gnoblin}/bin/gnoblinctl"
              test -x "${gnoblin}/bin/gnoblin"
              test ! -e "${gnoblin}/bin/gnoblin-mutter"
              test -f "${gnoblin}/share/wayland-sessions/gnoblin.desktop"
              test -f "${gnoblin}/lib/systemd/user/gnoblin-session.target"
              test -f "${gnoblin}/lib/systemd/user/gnoblin-idle.service"
              portal_configuration="${gnoblin}/share/xdg-desktop-portal/gnoblin-portals.conf"
              test -f "$portal_configuration"
              grep -Fxq 'default=gnoblin;*;' "$portal_configuration"
              test -f "${gnoblin}/share/xdg-desktop-portal/portals/gnoblin.portal"
              test ! -e "${gnoblin}/share/glib-2.0/schemas/org.gnome.shell.gschema.xml"
              schema_directory="${gnoblin}/share/gsettings-schemas/gnoblin-${gnoblin.version}/glib-2.0/schemas"
              test -f "$schema_directory/org.gnome.mutter.gschema.xml"
              test -f "$schema_directory/gschemas.compiled"
              test "$(
                  GSETTINGS_SCHEMA_DIR="$schema_directory" ${pkgs.glib.bin}/bin/gsettings \
                      get org.gnome.mutter overlay-key
              )" = "'Super'"
              touch "$out"
            '';
          }
        else
          { }
      );

      lib = {
        inherit nativePackages nixChannelEvaluations nixChannelPackages;
      };

      nixosModules.default = import ./nix/module.nix { inherit self; };
    };
}
