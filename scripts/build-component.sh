#!/usr/bin/env bash
# One upstream Meson project, called from the Ninja superbuild graph.
set -euo pipefail
name="${1:?component required}"
root="${2:?source root required}"
build_root="${3:?build root required}"
prefix="${4:?install prefix required}"
libdir="${5:?library directory required}"
buildtype="${6:?build type required}"
devkit="${7:?devkit state required}"
jobs="${8:?job count required}"
mode="${9:?source mode required}"
stage_root="${10:-}"
xwayland="${11:-ON}"
vector_cursors="${12:-OFF}"
mutter_version="$("$root/scripts/gnome-versions.py" get mutter version)"
mutter_api="${mutter_version%%.*}"
source_dir="$root/subprojects/$name"
build_dir="$build_root/$name"

if [ "$mode" = checkout ]; then
    inputs=("$root/gnome-versions.json")
    while IFS= read -r -d '' patch; do inputs+=("$patch"); done < <(
        find "$root/patches/$name" -type f -name '*.patch' -print0 | sort -z
    )
    while IFS= read -r -d '' manifest; do
        inputs+=("$manifest")
        while read -r project source _destination _rest; do
            [ "$project" = "$name" ] || continue
            inputs+=("$(dirname "$manifest")/$source")
        done <"$manifest"
    done < <(find "$root/src" -type f -name manifest -print0 | sort -z)
    signature="$(sha256sum -- "${inputs[@]}" | sha256sum | cut -d ' ' -f 1)"
    stamp="$build_root/$name.patch-inputs.sha256"
    tag="$("$root/scripts/gnome-versions.py" get "$name" version)"
    if [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$signature" ] ||
        [ "$(git -C "$source_dir" rev-parse HEAD)" = "$(git -C "$source_dir" rev-parse "$tag^{commit}")" ]; then
        "$root/scripts/apply-patches.sh" "$name"
        printf '%s\n' "$signature" >"$stamp"
    fi
fi
if [ "$name" = xdg-desktop-portal-gnome ]; then
    options=("-Ddbus_service_dir=$prefix/share/dbus-1/services"
        "-Dsystemduserunitdir=$prefix/lib/systemd/user")
elif [ "$name" = mutter ]; then
    case "$xwayland" in
        ON | TRUE | true | 1) xwayland_option=true ;;
        OFF | FALSE | false | 0) xwayland_option=false ;;
        *)
            echo "invalid XWayland setting: $xwayland" >&2
            exit 2
            ;;
    esac
    options=("-Ddevkit=$devkit" "-Dxwayland=$xwayland_option" -Dlibgnome_desktop=false -Dtests=disabled -Ddocs=false -Dprofiler=false -Dbash_completion=false "-Dudev_dir=$prefix/lib/udev")
    case "$vector_cursors" in
        ON | TRUE | true | 1) options+=("-Dhyprcursor=enabled") ;;
        OFF | FALSE | false | 0) options+=("-Dhyprcursor=disabled") ;;
        *)
            echo "invalid vector cursor setting: $vector_cursors" >&2
            exit 2
            ;;
    esac
else
    options=()
fi
installed_prefix="$stage_root$prefix"
if [ -n "$stage_root" ]; then
    private_pc="$build_root/pkgconfig"
    mkdir -p "$private_pc"
    export PKG_CONFIG_PATH="$private_pc${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
else
    export PKG_CONFIG_PATH="$prefix/$libdir/pkgconfig:$prefix/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi
export GI_GIR_PATH="$installed_prefix/share/gir-1.0${GI_GIR_PATH:+:$GI_GIR_PATH}"
export GI_TYPELIB_PATH="$installed_prefix/$libdir/girepository-1.0:$installed_prefix/$libdir/mutter-$mutter_api${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
export LD_LIBRARY_PATH="$installed_prefix/$libdir:$installed_prefix/$libdir/mutter-$mutter_api${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$installed_prefix/bin:$PATH"
if [ -f "$build_dir/meson-private/coredata.dat" ] &&
    [ -f "$source_dir/meson.options" ] &&
    [ "$source_dir/meson.options" -nt "$build_dir/meson-private/coredata.dat" ]; then
    # Meson checks command-line options against cached coredata before it
    # discovers newly added project options. Reconfigure cannot add them.
    meson setup --wipe "$build_dir" "$source_dir" --prefix="$prefix" --libdir="$libdir" --buildtype="$buildtype" "${options[@]}"
elif [ -f "$build_dir/meson-private/coredata.dat" ]; then
    meson setup --reconfigure "$build_dir" "$source_dir" --prefix="$prefix" --libdir="$libdir" --buildtype="$buildtype" "${options[@]}"
else
    meson setup "$build_dir" "$source_dir" --prefix="$prefix" --libdir="$libdir" --buildtype="$buildtype" "${options[@]}"
fi
meson compile -C "$build_dir" -j "$jobs"
if [ -n "$stage_root" ]; then
    meson install -C "$build_dir" --destdir "$stage_root" --no-rebuild
    while IFS= read -r -d '' pc; do
        sed "s|$prefix|$installed_prefix|g" "$pc" >"$private_pc/${pc##*/}"
    done < <(find "$installed_prefix/$libdir/pkgconfig" "$installed_prefix/share/pkgconfig" -maxdepth 1 -name '*.pc' -type f -print0 2>/dev/null)
else
    meson install -C "$build_dir" --no-rebuild
fi
if [ "$name" = mutter ]; then
    python3 "$root/scripts/generate-mutter-keybinding-catalog.py" \
        "$source_dir/src/core/keybindings.c" \
        "$installed_prefix/share/gnoblin/native-keybindings.json"
    devkit_marker="$installed_prefix/share/gnoblin/mutter-devkit-enabled"
    case "$devkit" in
        enabled | true | TRUE | 1)
            install -Dm644 /dev/null "$devkit_marker"
            ;;
        *)
            rm -f -- "$devkit_marker"
            ;;
    esac
fi
if [ "$name" = "xdg-desktop-portal-gnome" ] &&
    [ -d "$installed_prefix/share/glib-2.0/schemas" ]; then
    glib-compile-schemas "$installed_prefix/share/glib-2.0/schemas"
fi
