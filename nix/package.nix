{
  lib,
  stdenv,
  gcc16Stdenv ? stdenv,
  glib,
  json-glib,
  lua5_4,
  pkg-config,
  cmake,
  ninja,
  python3,
  git,
  gsettings-desktop-schemas,
  mutter,
  gnomePortal,
  gnoblinSrc,
  gnoblinRevision ? null,
  gnoblinSourceModified ? false,
  gnoblinRemote ? null,
  mutterSrc,
  gsettingsDesktopSchemasSrc,
  portalSrc,
  gxdpSrc,
  gvdbSrc,
  hyprcursor,
  libglycin,
}:
let
  versions = builtins.fromJSON (builtins.readFile "${gnoblinSrc}/gnome-versions.json");
  gnoblinVersion =
    (builtins.fromJSON (builtins.readFile "${gnoblinSrc}/gnoblin-version.json")).version;
  mutterVersion = versions.components.mutter.version;
  mutterApi = versions.components.mutter.api;
  patchesFor =
    project:
    lib.sort (left: right: builtins.lessThan (toString left) (toString right)) (
      lib.filter (path: lib.hasSuffix ".patch" (toString path)) (
        lib.filesystem.listFilesRecursive "${gnoblinSrc}/patches/${project}"
      )
    );
  copyOverlay = project: ''
    bash ${gnoblinSrc}/scripts/copy-overlay.sh ${project} "$PWD"
  '';
  addSubproject = source: directory: ''
    mkdir -p "subprojects/${directory}"
    cp -r --no-preserve=mode "${source}/." "subprojects/${directory}"
  '';
  schemas =
    if
      lib.versionAtLeast gsettings-desktop-schemas.version versions.components.gsettings-desktop-schemas.version
    then
      gsettings-desktop-schemas
    else
      gsettings-desktop-schemas.overrideAttrs {
        version = versions.components.gsettings-desktop-schemas.version;
        src = gsettingsDesktopSchemasSrc;
        patches = [ ];
      };
  gnoblinMutter = (mutter.override { stdenv = gcc16Stdenv; }).overrideAttrs (old: {
    pname = "gnoblin-mutter";
    version = mutterVersion;
    src = mutterSrc;
    patches = patchesFor "mutter";
    prePatch = (old.prePatch or "") + copyOverlay "mutter" + addSubproject gvdbSrc "gvdb";
    postPatch = old.postPatch or "";
    nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ git ];
    preConfigure = ''
      export PKG_CONFIG_PATH="${schemas}/share/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    ''
    + (old.preConfigure or "");
    buildInputs =
      map (
        dependency: if (dependency.pname or "") == "gsettings-desktop-schemas" then schemas else dependency
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
    postInstall = (old.postInstall or "") + ''
      mkdir -p "$devdoc"
    '';
  });
  gnoblinPortal = gnomePortal.overrideAttrs (old: {
    pname = "gnoblin-portal";
    version = versions.components.xdg-desktop-portal-gnome.version;
    src = portalSrc;
    patches = patchesFor "xdg-desktop-portal-gnome";
    prePatch =
      (old.prePatch or "") + copyOverlay "xdg-desktop-portal-gnome" + addSubproject gxdpSrc "libgxdp";
    nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ git ];
    buildInputs = (old.buildInputs or [ ]) ++ [
      json-glib
      libglycin
    ];
    mesonFlags = (old.mesonFlags or [ ]) ++ [
      "-Ddbus_service_dir=${placeholder "out"}/share/dbus-1/services"
      "-Dsystemduserunitdir=${placeholder "out"}/lib/systemd/user"
    ];
  });
  session = stdenv.mkDerivation {
    pname = "gnoblin";
    version = gnoblinVersion;
    src = gnoblinSrc;
    env = {
      GNOBLIN_SOURCE_MODIFIED = if gnoblinSourceModified then "1" else "0";
    }
    // lib.optionalAttrs (gnoblinRevision != null) {
      GNOBLIN_SOURCE_GIT_SHA = gnoblinRevision;
    }
    // lib.optionalAttrs (gnoblinRemote != null) {
      GNOBLIN_SOURCE_GIT_REMOTE = gnoblinRemote;
    };
    nativeBuildInputs = [
      cmake
      ninja
      pkg-config
      python3
      git
      glib.bin
    ];
    buildInputs = [
      glib
      json-glib
      lua5_4
    ];
    configurePhase = ''
      runHook preConfigure
      mkdir -p subprojects/mutter subprojects/xdg-desktop-portal-gnome
      cmake -S . -B build/session -G Ninja \
        -DGNOBLIN_PREFIX="$out" \
        -DGNOBLIN_LIBDIR=lib \
        -DGNOBLIN_BUILD_TYPE=release \
        -DGNOBLIN_SOURCE_MODE=release-archive
      runHook postConfigure
    '';
    buildPhase = ''
      runHook preBuild
      cmake --build build/session --target gnoblin gnoblin-idle gnoblinctl \
        --parallel "''${NIX_BUILD_CORES:-1}"
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      mkdir -p "$out/share/glib-2.0/schemas"
      for source in ${schemas} ${gnoblinMutter}; do
        while IFS= read -r -d $'\0' schema; do
          install -Dm644 "$schema" "$out/share/glib-2.0/schemas/''${schema##*/}"
        done < <(find "$source" -path '*/glib-2.0/schemas/*.xml' -print0)
      done
      GNOBLIN_VECTOR_CURSORS=OFF \
      GNOBLIN_IDLE_BINARY="$PWD/build/session/gnoblin-idle" \
      GNOBLINCTL_BINARY="$PWD/build/session/gnoblinctl" \
      GNOBLIN_IDENTITY_FILE="$PWD/build/session/gnoblinctl-identity.json" \
      GNOBLIN_VERSION_METADATA_FILE="$PWD/build/session/gnoblin-version.ini" \
      GNOBLIN_BINARY="$PWD/build/session/gnoblin" \
        bash scripts/install-session.sh "$out"

      mkdir -p "$out/bin" "$out/lib"
      ln -s ${gnoblinMutter}/bin/gnoblin-mutter "$out/bin/gnoblin-mutter"
      ln -s ${gnoblinMutter}/lib/mutter-${mutterApi} "$out/lib/mutter-${mutterApi}"
      printf '%s\n' '${gcc16Stdenv.cc.cc.lib}/lib' > "$out/libexec/gnoblin-cxx-lib"

      for schema in ${schemas} ${gnoblinMutter}; do
        while IFS= read -r -d $'\0' override; do
          install -Dm644 "$override" "$out/share/glib-2.0/schemas/''${override##*/}"
        done < <(find "$schema" -path '*/glib-2.0/schemas/*.override' -print0)
      done
      glib-compile-schemas "$out/share/glib-2.0/schemas"

      for path in \
        share/xdg-desktop-portal/portals/gnoblin.portal \
        share/xdg-desktop-portal/gnoblin-portals.conf \
        share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service \
        lib/systemd/user/xdg-desktop-portal-gnoblin.service; do
        if [ -e "${gnoblinPortal}/$path" ]; then
          mkdir -p "$out/$(dirname "$path")"
          ln -s "${gnoblinPortal}/$path" "$out/$path"
        fi
      done
      runHook postInstall
    '';
    passthru = {
      inherit gnoblinMutter gnoblinPortal schemas;
      providedSessions = [ "gnoblin" ];
    };
    meta = {
      description = "Standalone Lua-supervised Gnoblin Wayland session";
      homepage = "https://github.com/kierandrewett/gnoblin";
      mainProgram = "gnoblin";
      platforms = [ "x86_64-linux" ];
    };
  };
in
session
