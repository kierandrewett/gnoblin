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

gnoblin_env_validate_libdir() {
    local libdir="${1-}"

    case "$libdir" in
        ""|..|/*|../*|*/../*|*/..)
            echo "invalid GNOBLIN_LIBDIR (must stay below the prefix): $libdir" >&2
            return 2
            ;;
    esac
}

gnoblin_env_apply() {
    local prefix="${1:?usage: gnoblin_env_apply <prefix> [libdir]}"
    local libdir="${2:-${GNOBLIN_LIBDIR:-}}"

    if [ -z "$libdir" ] && [ -r "$prefix/libexec/gnoblin-libdir" ]; then
        IFS= read -r libdir < "$prefix/libexec/gnoblin-libdir"
    fi
    libdir="${libdir:-lib64}"

    gnoblin_env_validate_libdir "$libdir" || return

    export GNOBLIN_PREFIX="$prefix"
    export GNOBLIN_LIBDIR="$libdir"
    export LD_LIBRARY_PATH="$prefix/$libdir:$prefix/$libdir/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export GI_TYPELIB_PATH="$prefix/$libdir/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
    export PATH="$prefix/bin:$PATH"
    export GSETTINGS_SCHEMA_DIR="$prefix/share/glib-2.0/schemas"
    export XDG_DATA_DIRS="$prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    export GNOME_SHELL_SESSION_MODE=gnoblin
    export XDG_CURRENT_DESKTOP=GNOME:Gnoblin
}
