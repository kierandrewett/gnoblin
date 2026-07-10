#!/usr/bin/env bash
# gnoblin-env.sh -- shared runtime lookup-path setup for a gnoblin prefix.
#
# Source this and call `gnoblin_env_apply "$PREFIX"` from anything that needs
# gnome-shell/mutter to resolve against a gnoblin build prefix instead of
# whatever the ambient PATH/library search path offers. Four different
# callers need the exact same core exports: the headless test harness
# (scripts/run-gnome-shell.sh), the visible devkit (scripts/run-gnome-devkit.sh),
# the installed login-manager wrapper (gnoblin-session), and the installed
# systemd ExecStart wrapper (gnoblin-shell-service). One source of truth
# instead of four copies that can drift out of sync (e.g. if mutter's ABI
# version bumps from mutter-17 to mutter-18, this is the one place to change).
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

gnoblin_env_apply() {
    local prefix="${1:?usage: gnoblin_env_apply <prefix>}"

    export LD_LIBRARY_PATH="$prefix/lib64:$prefix/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export GI_TYPELIB_PATH="$prefix/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
    export PATH="$prefix/bin:$PATH"
    export GSETTINGS_SCHEMA_DIR="$prefix/share/glib-2.0/schemas"
    export XDG_DATA_DIRS="$prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    export GNOME_SHELL_SESSION_MODE=gnoblin
    export XDG_CURRENT_DESKTOP=GNOME:Gnoblin
}
