#!/usr/bin/env bash
# gnoblin-env.sh -- shared runtime lookup-path setup for a gnoblin prefix.
#
# Source this and call `gnoblin_env_apply "$PREFIX"` from development,
# installation, and capture scripts that need the gnoblin compositor and related tools
# to resolve against a Gnoblin build prefix. These callers share one setup for
# the Mutter ABI and library directory.
#
# Callers that need MORE than this (devkit isolation, headless-only backend
# forcing, disabling extensions for the systemd unit, ...) export their own
# extra variables after calling gnoblin_env_apply — this only owns the part
# every caller needs identically.
#
# Installed to $PREFIX/libexec/gnoblin-env.sh by scripts/install-session.sh
# for tools that need the same setup after the repository checkout is gone.
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
    local private_libdirs="$prefix/$libdir:$prefix/$libdir/mutter-$mutter_api"
    local cxx_lib=''
    if [ -r "$prefix/libexec/gnoblin-cxx-lib" ]; then
        IFS= read -r cxx_lib <"$prefix/libexec/gnoblin-cxx-lib"
    fi
    if [ -n "$cxx_lib" ]; then
        # Nix supplies a complete library closure. Host search paths can load
        # older libraries first and break its ABI on a source-build machine.
        export LD_LIBRARY_PATH="$private_libdirs:$cxx_lib"
    else
        export LD_LIBRARY_PATH="$private_libdirs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    fi
    export PATH="$prefix/bin:$PATH"
    export XDG_DATA_DIRS="$prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    # Let GSettings discover Gnoblin's private overrides and the compatible
    # desktop schemas supplied by the host.
    unset GSETTINGS_SCHEMA_DIR
    unset GNOME_SHELL_SESSION_MODE
    export XDG_CURRENT_DESKTOP=Gnoblin
}
