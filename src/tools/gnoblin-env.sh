#!/usr/bin/env bash
# gnoblin-env.sh -- shared runtime lookup-path setup for a gnoblin prefix.
#
# Source this and call `gnoblin_env_apply "$PREFIX"` from anything that needs
# gnome-shell/mutter to resolve against a gnoblin build prefix instead of
# whatever the ambient lookup paths offer. The launchers, installed wrappers,
# and standalone integration tests all use this function so Mutter ABI and
# library-directory changes have one source of truth.
#
# Callers that need MORE than this (devkit isolation, headless-only backend
# forcing, disabling extensions for the systemd unit, ...) export their own
# extra variables after calling gnoblin_env_apply — this only owns the part
# every caller needs identically.
#
# Installed to $PREFIX/libexec/gnoblin-env.sh by scripts/install-session.sh,
# so the two installed wrappers can source it without depending on this repo
# checkout still being present at the same path.
set -uo pipefail

# Installation must never merge a Gnoblin runtime into a shared GNOME prefix.
gnoblin_env_validate_install_prefix() {
    local prefix
    prefix="$(realpath -m -- "${1:?prefix required}")" || return
    case "$prefix" in
        / | /usr | /usr/local | /bin | /sbin | /lib | /lib64)
            echo "Refusing shared system prefix $prefix; use a private directory such as ./install or /usr/lib/gnoblin." >&2
            return 2
            ;;
    esac
}

gnoblin_env_validate_libdir() {
    local libdir="${1-}"

    case "$libdir" in
        "" | .. | /* | ../* | */../* | */..)
            echo "invalid GNOBLIN_LIBDIR (must stay below the prefix): $libdir" >&2
            return 2
            ;;
    esac
}

gnoblin_env_apply() {
    local prefix="${1:?usage: gnoblin_env_apply <prefix> [libdir]}"
    local libdir="${2:-${GNOBLIN_LIBDIR:-}}"

    if [ -z "$libdir" ] && [ -r "$prefix/libexec/gnoblin-libdir" ]; then
        IFS= read -r libdir <"$prefix/libexec/gnoblin-libdir"
    fi
    libdir="${libdir:-lib64}"

    gnoblin_env_validate_libdir "$libdir" || return

    export GNOBLIN_PREFIX="$prefix"
    export GNOBLIN_LIBDIR="$libdir"
    local mutter_api="${GNOBLIN_MUTTER_API:-51}"
    local shell_libdir="$prefix/$libdir/gnome-shell"
    export LD_LIBRARY_PATH="$shell_libdir:$prefix/$libdir:$prefix/$libdir/mutter-$mutter_api${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export GI_TYPELIB_PATH="$shell_libdir/girepository-1.0:$shell_libdir:$prefix/$libdir/girepository-1.0:$prefix/$libdir/mutter-$mutter_api${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
    export PATH="$prefix/bin:$PATH"
    export XDG_DATA_DIRS="$prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    # Let GSettings discover the private schemas first, then fall back to the
    # system schemas needed by GNOME Shell services.
    unset GSETTINGS_SCHEMA_DIR
    export GNOME_SHELL_SESSION_MODE=gnoblin
    export XDG_CURRENT_DESKTOP=GNOME:Gnoblin
}
