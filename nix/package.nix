{
    lib,
    stdenv,
    symlinkJoin,
    glib,
    unzip,

    mutter,
    gnomeShell,
    gnomeSession,
    gnoblinSrc,
    mutterSrc,
    gnomeShellSrc,
    gvdbSrc,
    gvcSrc,
    libshewSrc,
    jasmineGjsSrc,
}:
let
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

    gnoblinMutter = mutter.overrideAttrs (old: {
        pname = "gnoblin-mutter";
        version = "49.5";
        src = mutterSrc;
        patches = (old.patches or [ ]) ++ patchesFor "mutter";
        prePatch = (old.prePatch or "") + copyOverlays "mutter" + addSubproject gvdbSrc "gvdb";
        postPatch = old.postPatch or "";
    });

    gnoblinShell = (gnomeShell.override { mutter = gnoblinMutter; }).overrideAttrs (old: {
        pname = "gnoblin-shell";
        version = "49.6";
        src = gnomeShellSrc;
        patches =
            lib.filter (patch: !(lib.hasSuffix "-fix-paths.patch" (toString patch))) (old.patches or [ ])
            ++ patchesFor "gnome-shell";
        prePatch =
            (old.prePatch or "")
            + copyOverlays "gnome-shell"
            + addSubproject gvcSrc "gvc"
            + addSubproject libshewSrc "libshew"
            + addSubproject jasmineGjsSrc "jasmine-gjs";
        postPatch =
            lib.replaceStrings
                [ "rm data/theme/gnome-shell-{light,dark}.css" ]
                [ "rm -f data/theme/gnome-shell-{light,dark}.css" ]
                (old.postPatch or "")
            + ''
                substituteInPlace data/org.gnome.Shell-disable-extensions.service \
                    --replace-fail "ExecStart=gsettings" "ExecStart=${glib.bin}/bin/gsettings"
                substituteInPlace js/ui/extensionDownloader.js \
                    --replace-fail "['unzip'," "['${unzip}/bin/unzip'," \
                    --replace-fail "['glib-compile-schemas'" "['${glib.dev}/bin/glib-compile-schemas'"
                substituteInPlace subprojects/extensions-tool/src/command-install.c \
                    --replace-fail '"glib-compile-schemas"' '"${glib.dev}/bin/glib-compile-schemas"'
            '';
    });

    session = stdenv.mkDerivation {
        pname = "gnoblin-session";
        version = "49.6";
        src = gnoblinSrc;
        dontBuild = true;

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

            install -Dm644 src/data/session/gnoblin.desktop \
                "$out/share/wayland-sessions/gnoblin.desktop"

            install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell.target \
                "$out/lib/systemd/user/org.gnoblin.Shell.target"
            install -Dm644 src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf \
                "$out/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
            install -Dm644 src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in \
                "$out/lib/systemd/user/org.gnoblin.Shell@wayland.service"
        '';
    };
in
symlinkJoin {
    name = "gnoblin-49.6";
    paths = [
        gnoblinMutter
        gnoblinShell
        session
    ];
    nativeBuildInputs = [ glib ];

    postBuild = ''
        for tool in gnoblin-session gnoblin-shell-service gnoblinctl; do
            rm "$out/bin/$tool"
            install -Dm755 "${gnoblinSrc}/src/tools/$tool" "$out/bin/$tool"
        done
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
            --replace-fail "exec gnome-session" \
            "exec ${gnomeSession}/bin/gnome-session"
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
        inherit gnoblinMutter gnoblinShell session;
        providedSessions = [ "gnoblin" ];
    };

    meta = {
        description = "Patched Mutter and GNOME Shell session with an external-chrome contract";
        homepage = "https://github.com/kierandrewett/gnoblin";
        license = lib.licenses.gpl2Plus;
        platforms = lib.platforms.x86_64;
    };
}
