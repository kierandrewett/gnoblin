{
  lib,
  stdenv,
  gcc16Stdenv,
  symlinkJoin,
  glib,
  gjs,
  unzip,
  hyprcursor,
  lua5_4,
  libepoxy,
  libglycin,
  python3,
  gtk3,
  wrapGAppsHook3,
  systemd,
  makeWrapper,

  mutter,
  gnomeShell,
  gnomeSession,
  gsettings-desktop-schemas,
  gnoblinSrc,
  mutterSrc,
  gnomeShellSrc,
  gsettingsDesktopSchemasSrc,
  gvdbSrc,
  gvcSrc,
  libshewSrc,
  jasmineGjsSrc,
}:
let
  versions = builtins.fromJSON (builtins.readFile "${gnoblinSrc}/gnome-versions.json");
  gnomeVersion = versions.components.gnome-shell.version;
  clipboardPython = python3.withPackages (ps: [ ps.pygobject3 ]);
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

  # hyprcursor and its C++ dependencies use Nixpkgs' GCC 16 ABI. Build the
  # consumer with the same toolchain so its final executable resolves the
  # matching libstdc++ symbol versions.
  gnoblinMutter = (mutter.override { stdenv = gcc16Stdenv; }).overrideAttrs (old: {
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
        buildInputs =
          map (
            dependency:
            if (dependency.pname or "") == "gsettings-desktop-schemas" then gnoblinSchemas else dependency
          ) (old.buildInputs or [ ])
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
        mesonFlags = (old.mesonFlags or [ ]) ++ [ "-Dextensions_tool=false" ];
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

  session = stdenv.mkDerivation {
    pname = "gnoblin-session";
    version = gnomeVersion;
    src = gnoblinSrc;
    dontBuild = true;
    nativeBuildInputs = [
      makeWrapper
      wrapGAppsHook3
    ];
    buildInputs = [ gtk3 ];
    dontWrapGApps = true;
    postFixup = ''
      makeWrapper ${clipboardPython}/bin/python3 "$out/libexec/gnoblin-clipboard-paste" \
          --add-flags "$out/share/gnoblin/scripts/lib/clipboard-paste.py" "''${gappsWrapperArgs[@]}"
    '';

    installPhase = ''
      install -Dm644 src/data/session/modes/gnoblin.json \
          "$out/share/gnome-shell/modes/gnoblin.json"
      install -Dm644 src/data/session/gnome-session/gnoblin.session \
          "$out/share/gnome-session/sessions/gnoblin.session"
      install -Dm644 src/tools/gnoblin-env.sh "$out/libexec/gnoblin-env.sh"
      install -Dm644 src/data/session/schemas/00_org.gnoblin.mutter.gschema.override \
          "$out/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override"

      printf '%s\n' lib > "$out/libexec/gnoblin-libdir"

      install -Dm755 src/tools/gnoblin-session "$out/bin/gnoblin-session"
      install -Dm755 src/tools/gnoblin-shell-service "$out/bin/gnoblin-shell-service"
      install -Dm755 src/tools/gnoblinctl "$out/bin/gnoblinctl"
      substituteInPlace "$out/bin/gnoblinctl" --replace-fail '#!/usr/bin/env python3' '#!${python3}/bin/python3'
      wrapProgram "$out/bin/gnoblinctl" --set-default GNOBLIN_BUSCTL "${systemd}/bin/busctl"
      install -Dm644 src/scripts/compositor-bridge.js "$out/share/gnoblin/scripts/compositor-bridge.js"
      install -Dm644 src/scripts/lib/ui-sessions.js "$out/share/gnoblin/scripts/lib/ui-sessions.js"
      install -Dm644 src/scripts/lib/layer-companions.js "$out/share/gnoblin/scripts/lib/layer-companions.js"
      install -Dm644 src/scripts/lib/window-switcher-fallback.js "$out/share/gnoblin/scripts/lib/window-switcher-fallback.js"
      install -Dm644 src/scripts/lib/clipboard-paste.js "$out/share/gnoblin/scripts/lib/clipboard-paste.js"
      install -Dm644 src/scripts/lib/clipboard-paste.py "$out/share/gnoblin/scripts/lib/clipboard-paste.py"
      substituteInPlace "$out/share/gnoblin/scripts/lib/clipboard-paste.js" \
          --replace-fail '["python3", helper]' '["'$out'/libexec/gnoblin-clipboard-paste"]'
      substituteInPlace "$out/share/gnoblin/scripts/lib/clipboard-paste.py" \
          --replace-fail '"libgtk-3.so.0"' '"${gtk3}/lib/libgtk-3.so.0"' \
          --replace-fail '"libgdk-3.so.0"' '"${gtk3}/lib/libgdk-3.so.0"'


      install -Dm644 src/data/session/gnoblin.desktop \
          "$out/share/wayland-sessions/gnoblin.desktop"

      install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell.target \
          "$out/lib/systemd/user/org.gnoblin.Shell.target"
      install -Dm644 src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf \
          "$out/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
      install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in \
          "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service"
      # NixOS 25.11's gnome-session package owns the shared Wayland
      # target units. Keep one systemd user-unit owner.
    '';
  };
  runtime = symlinkJoin {
    name = "gnoblin-runtime-${gnomeVersion}";
    paths = [
      gnoblinMutter
      gnoblinShell
      gnoblinSchemas
      session
    ];
    nativeBuildInputs = [
      glib
      makeWrapper
    ];

    postBuild = ''
      for tool in gnoblin-session gnoblin-shell-service gnoblinctl; do
          rm "$out/bin/$tool"
          install -Dm755 "${gnoblinSrc}/src/tools/$tool" "$out/bin/$tool"
      done
      substituteInPlace "$out/bin/gnoblinctl" --replace-fail '#!/usr/bin/env python3' '#!${python3}/bin/python3'
      # The session output already contains a wrapped CLI; remove only its
      # copied wrapper target before wrapping the updated source here.
      rm -f "$out/bin/.gnoblinctl-wrapped"
      wrapProgram "$out/bin/gnoblinctl" --set-default GNOBLIN_BUSCTL "${systemd}/bin/busctl"
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


      substituteInPlace "$out/bin/gnoblin-session" \
          --replace-fail "    gnome-session --no-reexec" \
          "    ${gnomeSession}/bin/gnome-session --no-reexec"
      substituteInPlace "$out/share/wayland-sessions/gnoblin.desktop" \
          --replace-fail "Exec=env GNOME_SHELL_SESSION_MODE=gnoblin gnome-session --session=gnoblin" \
          "Exec=$out/bin/gnoblin-session"
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
        session
        ;
      providedSessions = [ "gnoblin" ];
    };

    meta = {
      description = "Patched Mutter and GNOME Shell session with an external-chrome contract";
      homepage = "https://github.com/kierandrewett/gnoblin";
      license = lib.licenses.gpl2Plus;
      platforms = lib.platforms.x86_64;
    };
  };
in
symlinkJoin {
  name = "gnoblin-${gnomeVersion}";
  paths = [ ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = ''
    # Only Gnoblin entry points enter the system profile. The runtime's
    # stock-named binaries, schemas and D-Bus services stay private.
    mkdir -p "$out/bin" "$out/share/wayland-sessions" \
        "$out/share/gnome-session/sessions" "$out/lib/systemd/user" \
        "$out/share/polkit-1/actions"
    makeWrapper ${runtime}/bin/gnoblinctl "$out/bin/gnoblinctl"
    ln -s ${runtime}/share/wayland-sessions/gnoblin.desktop \
        "$out/share/wayland-sessions/gnoblin.desktop"
    ln -s ${runtime}/share/gnome-session/sessions/gnoblin.session \
        "$out/share/gnome-session/sessions/gnoblin.session"
    for unit in org.gnoblin.Shell.target org.gnoblin.Shell@wayland.service \
        gnome-session@gnoblin.target.d; do
        ln -s "${runtime}/lib/systemd/user/$unit" "$out/lib/systemd/user/$unit"
    done
    sed 's/org.gnome.mutter.backlight-helper/org.gnoblin.mutter.backlight-helper/g' \
        ${gnoblinMutter}/share/polkit-1/actions/org.gnome.mutter.backlight-helper.policy \
        > "$out/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
  '';
  passthru = {
    inherit
      runtime
      gnoblinMutter
      gnoblinSchemas
      gnoblinShell
      session
      ;
    providedSessions = [ "gnoblin" ];
  };
  meta = runtime.meta;
}
