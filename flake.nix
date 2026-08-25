{
    description = "Gnoblin session package and NixOS module";

    inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

        mutter-src = {
            url = "git+https://gitlab.gnome.org/GNOME/mutter.git?rev=759d53b098df10a1c0b443c093f586e5b1b7de49";
            flake = false;
        };

        gnome-shell-src = {
            url = "git+https://gitlab.gnome.org/GNOME/gnome-shell.git?rev=21aa7264064b34f14bd6369215790d41853b2e12";
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
            ...
        }:
        let
            systems = [ "x86_64-linux" ];
            forAllSystems = nixpkgs.lib.genAttrs systems;
        in
        {
            packages = forAllSystems (
                system:
                let
                    pkgs = import nixpkgs { inherit system; };
                in
                rec {
                    gnoblin = pkgs.callPackage ./nix/package.nix {
                        gnoblinSrc = self.outPath;
                        mutterSrc = inputs.mutter-src.outPath;
                        gnomeShellSrc = inputs.gnome-shell-src.outPath;
                        gvdbSrc = inputs.gvdb.outPath;
                        gvcSrc = inputs.gvc.outPath;
                        libshewSrc = inputs.libshew.outPath;
                        jasmineGjsSrc = inputs.jasmineGjs.outPath;
                        gnomeShell = pkgs.gnome-shell;
                        gnomeSession = pkgs.gnome-session;
                    };
                    default = gnoblin;
                }
            );

            checks = forAllSystems (
                system:
                let
                    pkgs = import nixpkgs { inherit system; };
                    gnoblin = self.packages.${system}.gnoblin;
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
                        test "${toString (builtins.head moduleTest.config.services.displayManager.sessionPackages)}" = "${gnoblin}"
                        test "${toString (builtins.head moduleTest.config.systemd.packages)}" = "${gnoblin}"
                        test -x "${gnoblin}/bin/gnome-shell"
                        test -x "${gnoblin}/bin/gnoblin-session"
                        test -x "${gnoblin}/bin/gnoblin-shell-service"
                        test -f "${gnoblin}/share/wayland-sessions/gnoblin.desktop"
                        test -f "${gnoblin}/lib/systemd/user/org.gnoblin.Shell@wayland.service"
                        test -f "${pkgs.gnome-session}/share/systemd/user/gnome-session-wayland.target"
                        test -f "${pkgs.gnome-session}/share/systemd/user/gnome-session-wayland@.target"
                        test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell-disable-extensions.service"
                        test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell.target"
                        test ! -e "${gnoblin}/lib/systemd/user/org.gnome.Shell@wayland.service"
                        bash -n "${gnoblin}/bin/gnoblin-session" "${gnoblin}/bin/gnoblin-shell-service"
                        case "$(<"${gnoblin}/bin/gnoblin-session")" in
                            *"--no-reexec"*) ;;
                            *) exit 1 ;;
                        esac
                        schema_directory="${gnoblin}/share/glib-2.0/schemas"
                        test -f "$schema_directory/org.gnome.mutter.gschema.xml"
                        test -f "$schema_directory/org.gnome.shell.gschema.xml"
                        test -f "$schema_directory/org.gnoblin.shell.gschema.xml"
                        test -f "$schema_directory/gschemas.compiled"
                        test "$(
                            GSETTINGS_BACKEND=memory \
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

            nixosModules.default = import ./nix/module.nix { inherit self; };
        };
}
