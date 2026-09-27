{
  lib,
  stdenv,
  gcc16Stdenv ? stdenv,
  symlinkJoin,
  glib,
  gjs,
  unzip,
  hyprcursor,
  librsvg,
  adwaita-icon-theme,
  lua5_4,
  libepoxy,
  libglycin,
  json-glib,
  pkg-config,
  python3,
  wrapGAppsHook3,
  makeWrapper,

  mutter,
  gnomeShell,
  gnomePortal,
  gnomeSession,
  wireplumber,
  gsettings-desktop-schemas,
  gnoblinSrc,
  gnoblinRevision ? null,
  gnoblinSourceModified ? false,
  gnoblinRemote ? null,
  mutterSrc,
  gnomeShellSrc,
  gsettingsDesktopSchemasSrc,
  portalSrc,
  gxdpSrc,
  gvdbSrc,
  gvcSrc,
  libshewSrc,
  jasmineGjsSrc,
}:
let
  versions = builtins.fromJSON (builtins.readFile "${gnoblinSrc}/gnome-versions.json");
  gnomeVersion = versions.components.gnome-shell.version;
  gnoblinVersion =
    (builtins.fromJSON (builtins.readFile "${gnoblinSrc}/gnoblin-version.json")).version;
  patchesFor =
    project:
    lib.sort (left: right: builtins.lessThan (toString left) (toString right)) (
      lib.filter (path: lib.hasSuffix ".patch" (toString path)) (
        lib.filesystem.listFilesRecursive "${gnoblinSrc}/patches/${project}"
      )
    );

  copyOverlays = project: ''
    bash ${gnoblinSrc}/scripts/copy-overlay.sh ${project} "$PWD"
  '';

  addSubproject = source: directory: ''
    mkdir -p "subprojects/${directory}"
    cp -r --no-preserve=mode "${source}/." "subprojects/${directory}"
  '';

  requiredSchemasVersion = versions.components.gsettings-desktop-schemas.version;
  gnoblinSchemas =
    if lib.versionAtLeast gsettings-desktop-schemas.version requiredSchemasVersion then
      gsettings-desktop-schemas
    else
      gsettings-desktop-schemas.overrideAttrs {
        version = requiredSchemasVersion;
        src = gsettingsDesktopSchemasSrc;
        patches = [ ];
      };

  # Use the channel's GCC 16 toolchain where it exists. Stable channels which
  # predate GCC 16 build hyprcursor and its C++ closure with their default
  # stdenv, so the fallback keeps the consumer in that same ABI closure.
  gnoblinMutter =
    (mutter.override {
      stdenv = gcc16Stdenv;
    }).overrideAttrs
      (old: {
        pname = "gnoblin-mutter";
        version = versions.components.mutter.version;
        src = mutterSrc;
        # Nixpkgs patches target its own GNOME source revision. Gnoblin carries a
        # complete patch stack rebased onto the release pinned in the manifest.
        patches = patchesFor "mutter";
        prePatch = (old.prePatch or "") + copyOverlays "mutter" + addSubproject gvdbSrc "gvdb";
        postPatch = old.postPatch or "";
        preConfigure = ''
          export PKG_CONFIG_PATH="${gnoblinSchemas}/share/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
        ''
        + (old.preConfigure or "");
        postInstall = (old.postInstall or "") + ''
          # Mutter declares this split output even when gi-docgen has nothing to
          # install for the selected feature set. Keep the derivation contract.
          mkdir -p "$devdoc"
        '';
        buildInputs =
          map (
            dependency:
            if (dependency.pname or "") == "gsettings-desktop-schemas" then gnoblinSchemas else dependency
          ) (old.buildInputs or [ ])
          ++ [
            hyprcursor
            json-glib
            lua5_4
          ];
        mesonFlags =
          lib.filter (
            flag: !(lib.hasPrefix "-Degl_device=" flag || lib.hasPrefix "-Dwayland_eglstream=" flag)
          ) (old.mesonFlags or [ ])
          ++ [ "-Dhyprcursor=enabled" ];
      });

  gnoblinShell =
    (gnomeShell.override {
      mutter = gnoblinMutter;
      stdenv = gcc16Stdenv;
    }).overrideAttrs
      (old: {
        pname = "gnoblin-shell";
        version = gnomeVersion;
        src = gnomeShellSrc;
        # These libraries serve only the disabled captive-network helper and calendar server.
        buildInputs =
          map
            (
              dependency:
              if (dependency.pname or "") == "gsettings-desktop-schemas" then gnoblinSchemas else dependency
            )
            (
              builtins.filter (
                dependency:
                !(builtins.elem (dependency.pname or "") [
                  "webkitgtk"
                  "evolution-data-server"
                  "libical"
                ])
              ) (old.buildInputs or [ ])
            )
          ++ [
            libepoxy
            libglycin
          ];
        patches = patchesFor "gnome-shell";
        prePatch =
          (old.prePatch or "")
          + copyOverlays "gnome-shell"
          + addSubproject gvcSrc "gvc"
          + addSubproject libshewSrc "libshew"
          + addSubproject jasmineGjsSrc "jasmine-gjs";
        preConfigure = ''
          export PKG_CONFIG_PATH="${gnoblinSchemas}/share/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
        ''
        + (old.preConfigure or "");
        # Nixpkgs' hook follows its older Shell source and names files removed in
        # 51. Keep the useful fixups, scoped to paths in the pinned release.
        postPatch = ''
              patchShebangs build-aux/generate-app-list.py
              rm -f man/gnome-shell.1 data/theme/gnome-shell-{light,dark}.css
              substituteInPlace meson.build \
                  --replace-fail "gjs = find_program('gjs')" "gjs = find_program('${gjs}/bin/gjs')"
              substituteInPlace data/org.gnome.Shell-disable-extensions.service \
                  --replace-fail "ExecStart=gsettings" "ExecStart=${glib.bin}/bin/gsettings"
          substituteInPlace js/ui/extensionDownloader.js \
              --replace-fail "['unzip'," "['${unzip}/bin/unzip'," \
              --replace-fail "['glib-compile-schemas'" "['${glib.dev}/bin/glib-compile-schemas'"
          substituteInPlace src/meson.build \
              --replace-fail "extra_args: ['--quiet']," \
              "extra_args: ['--quiet', '--library-path=${gcc16Stdenv.cc.cc.lib}/lib'],"
          substituteInPlace src/st/meson.build \
              --replace-fail "extra_args: ['-DST_COMPILATION', '--quiet']," \
              "extra_args: ['-DST_COMPILATION', '--quiet', '--library-path=${gcc16Stdenv.cc.cc.lib}/lib'],"
        '';
        mesonFlags = (old.mesonFlags or [ ]) ++ [
          "-Dextensions_tool=false"
          "-Dportal_helper=false"
          "-Dcalendar_server=false"
          "-Dhotplug_sniffer=false"
        ];
        postFixup = ''
          for service in org.gnome.ScreenSaver org.gnome.Shell.Notifications org.gnome.Shell.Screencast; do
              makeWrapper ${gjs}/bin/gjs "$out/libexec/$service" \
                  --add-flags "-m" \
                  --add-flags "$out/share/gnome-shell/$service" \
                  "''${gappsWrapperArgs[@]}"
              substituteInPlace "$out/share/dbus-1/services/$service.service" \
                  --replace-fail \
                  "Exec=${gjs}/bin/gjs -m $out/share/gnome-shell/$service" \
                  "Exec=$out/libexec/$service"
          done

          # Cannot be in postInstall, otherwise the multi-output docs hook moves
          # the directory back into the primary output.
          moveToOutput "share/doc" "$devdoc"
        '';
      });

  gnoblinPortal = gnomePortal.overrideAttrs (old: {
    pname = "gnoblin-portal";
    version = versions.components.xdg-desktop-portal-gnome.version;
    src = portalSrc;
    patches = patchesFor "xdg-desktop-portal-gnome";
    prePatch =
      (old.prePatch or "") + copyOverlays "xdg-desktop-portal-gnome" + addSubproject gxdpSrc "libgxdp";
    buildInputs = (old.buildInputs or [ ]) ++ [ libglycin ];
    mesonFlags = (old.mesonFlags or [ ]) ++ [
      "-Ddbus_service_dir=${placeholder "out"}/share/dbus-1/services"
      "-Dsystemduserunitdir=${placeholder "out"}/lib/systemd/user"
    ];
  });

  session = stdenv.mkDerivation {
    pname = "gnoblin-session";
    version = gnoblinVersion;
    src = gnoblinSrc;
    dontBuild = true;
    nativeBuildInputs = [
      makeWrapper
      wrapGAppsHook3
      librsvg
      hyprcursor
      python3
      pkg-config
    ];
    buildInputs = [
      glib
      json-glib
    ];
    installPhase = ''
      install -Dm644 src/data/session/modes/gnoblin.json \
          "$out/share/gnome-shell/modes/gnoblin.json"
      install -Dm644 src/data/session/gnome-session/gnoblin.session \
          "$out/share/gnome-session/sessions/gnoblin.session"
      install -Dm644 src/tools/gnoblin-env.sh "$out/libexec/gnoblin-env.sh"
      install -Dm644 src/data/session/schemas/00_org.gnoblin.mutter.gschema.override \
          "$out/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override"

      printf '%s\n' lib > "$out/libexec/gnoblin-libdir"

      install -Dm755 src/tools/gnoblin "$out/bin/gnoblin"
      install -Dm755 src/tools/gnoblin-seed-config "$out/libexec/gnoblin-seed-config"
      install -Dm644 src/data/init.lua.example "$out/share/gnoblin/init.lua.example"
      LD_LIBRARY_PATH="${librsvg}/lib:${glib.out}/lib" \
        python3 ${gnoblinSrc}/scripts/build-adwaita-hyprcursor.py \
        --output "$TMPDIR/Adwaita-Hyprcursor" \
        --fallback "${adwaita-icon-theme}/share/icons/Adwaita"
      mkdir -p "$out/share/icons"
      cp -a "$TMPDIR/Adwaita-Hyprcursor" "$out/share/icons/Adwaita-Hyprcursor"
      install -Dm755 src/tools/gnoblin-shell-service "$out/bin/gnoblin-shell-service"
      $CC -std=gnu17 -O2 -o gnoblinctl src/tools/gnoblinctl.c \
        $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0 json-glib-1.0)
      install -Dm755 gnoblinctl "$out/bin/gnoblinctl"
      ${lib.optionalString (gnoblinRevision != null) ''
        export GNOBLIN_SOURCE_GIT_SHA=${lib.escapeShellArg gnoblinRevision}
      ''}
      ${lib.optionalString (gnoblinRemote != null) ''
        export GNOBLIN_SOURCE_GIT_REMOTE=${lib.escapeShellArg gnoblinRemote}
      ''}
      export GNOBLIN_SOURCE_MODIFIED=${if gnoblinSourceModified then "1" else "0"}
      python3 scripts/build-identity.py "$out/share/gnoblin/version.json"

      install -Dm644 src/data/session/gnoblin.desktop \
          "$out/share/wayland-sessions/gnoblin.desktop"

      install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell.target \
          "$out/lib/systemd/user/org.gnoblin.Shell.target"
      install -Dm644 src/data/session/systemd-user/gnoblin-session.target \
          "$out/lib/systemd/user/gnoblin-session.target"
      install -Dm644 src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf \
          "$out/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
      install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in \
          "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service"
      # The reused gnome-session package owns the generic session target
      # units. Keep one systemd user-unit owner.
    '';
  };
  runtime = symlinkJoin {
    name = "gnoblin-runtime-${gnomeVersion}";
    paths = [
      glib
      gnoblinMutter
      gnoblinShell
      gnoblinPortal
      gnoblinSchemas
      session
      wireplumber
    ];
    nativeBuildInputs = [
      glib
      makeWrapper
    ];

    postBuild = ''
      for tool in gnoblin gnoblin-shell-service; do
          rm "$out/bin/$tool"
          install -Dm755 "${gnoblinSrc}/src/tools/$tool" "$out/bin/$tool"
      done
      printf '%s\n' '${gcc16Stdenv.cc.cc.lib}/lib' > "$out/libexec/gnoblin-cxx-lib"
      rm "$out/share/wayland-sessions/gnoblin.desktop"
      install -Dm644 "${gnoblinSrc}/src/data/session/gnoblin.desktop" \
          "$out/share/wayland-sessions/gnoblin.desktop"
      rm "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service"
      install -Dm644 "${gnoblinSrc}/src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in" \
          "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service"

      # The host's GNOME packages can also provide these stock units. Gnoblin
      # starts only org.gnoblin.Shell@wayland.service through its own target.
      rm -f \
          "$out/lib/systemd/user/org.gnome.Shell-disable-extensions.service" \
          "$out/lib/systemd/user/org.gnome.Shell.target" \
          "$out/lib/systemd/user/org.gnome.Shell@wayland.service"


      substituteInPlace "$out/bin/gnoblin" \
          --replace-fail "    gnome-session --no-reexec" \
          "    ${gnomeSession}/bin/gnome-session --no-reexec"
      substituteInPlace "$out/share/wayland-sessions/gnoblin.desktop" \
          --replace-fail "Exec=env GNOME_SHELL_SESSION_MODE=gnoblin gnome-session --session=gnoblin" \
          "Exec=$out/bin/gnoblin"
      substituteInPlace "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service" \
          --replace-fail "@PREFIX@" "$out"

      # GNOME Shell and Mutter each keep their schemas in a versioned
      # package directory. The session wrapper needs one concrete directory,
      # because GSETTINGS_SCHEMA_DIR does not traverse those directories.
      schema_directory="$out/share/glib-2.0/schemas"
      for source_directory in "$out"/share/gsettings-schemas/*/glib-2.0/schemas; do
          [ -d "$source_directory" ] || continue
          for schema in "$source_directory"/*.xml "$source_directory"/*.override; do
              [ -e "$schema" ] || continue
              target="$schema_directory/''${schema##*/}"
              if [ -e "$target" ]; then
                  if ! cmp -s "$schema" "$target"; then
                      echo "conflicting GSettings schema: ''${schema##*/}" >&2
                      exit 1
                  fi
                  continue
              fi
              cp --no-preserve=mode "$schema" "$target"
          done
      done
      rm -f "$schema_directory/gschemas.compiled"
      glib-compile-schemas "$schema_directory"
    '';

    passthru = {
      inherit
        gnoblinMutter
        gnoblinSchemas
        gnoblinShell
        gnoblinPortal
        session
        ;
      providedSessions = [ "gnoblin" ];
    };

    meta = {
      description = "Patched Mutter and GNOME Shell session with an external-chrome contract";
      homepage = "https://github.com/kierandrewett/gnoblin";
      license = with lib.licenses; [
        gpl2Plus
        lgpl3Plus
        cc-by-sa-30
      ];
      platforms = [ "x86_64-linux" ];
    };
  };
in
symlinkJoin {
  name = "gnoblin-${gnoblinVersion}";
  paths = [ ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = ''
    # Only Gnoblin entry points enter the system profile. The runtime's
    # stock-named binaries, schemas and D-Bus services stay private.
    mkdir -p "$out/bin" "$out/share/wayland-sessions" \
        "$out/share/gnome-session/sessions" "$out/lib/systemd/user" \
        "$out/share/polkit-1/actions" "$out/share/xdg-desktop-portal/portals" \
        "$out/share/dbus-1/services"
    makeWrapper ${runtime}/bin/gnoblin "$out/bin/gnoblin"
    makeWrapper ${runtime}/bin/gnoblinctl "$out/bin/gnoblinctl"
    ln -s ${runtime}/share/wayland-sessions/gnoblin.desktop \
        "$out/share/wayland-sessions/gnoblin.desktop"
    ln -s ${runtime}/share/gnome-session/sessions/gnoblin.session \
        "$out/share/gnome-session/sessions/gnoblin.session"
    for unit in gnoblin-session.target org.gnoblin.Shell.target org.gnoblin.Shell@wayland.service \
        gnome-session@gnoblin.target.d; do
        ln -s "${runtime}/lib/systemd/user/$unit" "$out/lib/systemd/user/$unit"
    done
    sed 's/org.gnome.mutter.backlight-helper/org.gnoblin.mutter.backlight-helper/g' \
        ${gnoblinMutter}/share/polkit-1/actions/org.gnome.mutter.backlight-helper.policy \
        > "$out/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
    ln -s ${runtime}/share/xdg-desktop-portal/gnoblin-portals.conf \
        "$out/share/xdg-desktop-portal/gnoblin-portals.conf"
    ln -s ${runtime}/share/xdg-desktop-portal/portals/gnoblin.portal \
        "$out/share/xdg-desktop-portal/portals/gnoblin.portal"
    ln -s ${runtime}/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service \
        "$out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service"
    ln -s ${runtime}/lib/systemd/user/xdg-desktop-portal-gnoblin.service \
        "$out/lib/systemd/user/xdg-desktop-portal-gnoblin.service"
  '';
  passthru = {
    inherit
      runtime
      gnoblinMutter
      gnoblinSchemas
      gnoblinShell
      gnoblinPortal
      session
      ;
    providedSessions = [ "gnoblin" ];
  };
  meta = runtime.meta;
}
