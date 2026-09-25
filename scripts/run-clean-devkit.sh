#!/usr/bin/env bash
# Run the visible devkit with a disposable user profile.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
host_display="${WAYLAND_DISPLAY:-}"
case "$host_display" in
    /* | "") ;;
    *) host_display="$host_runtime/$host_display" ;;
esac

profile="$(mktemp -d /tmp/gnoblin-clean-devkit.XXXXXX)"
cleanup() {
    # Shell children can briefly write their cache after the devkit exits.
    for _ in 1 2 3; do
        rm -rf -- "$profile" 2>/dev/null || true
        if [ ! -e "$profile" ]; then return; fi
        sleep 1
    done
    if [ -e "$profile" ]; then
        printf 'run-clean-devkit: could not remove disposable profile %s\n' "$profile" >&2
        return 1
    fi
}
trap 'cleanup || exit 1' EXIT
mkdir -m 700 "$profile/home" "$profile/config" "$profile/data" \
    "$profile/cache" "$profile/state" "$profile/runtime"

# The devkit viewer needs the host compositor and, on some builds, PipeWire.
# Neither is a source of user config; all persistent XDG paths stay disposable.
if [ -S "$host_runtime/pipewire-0" ]; then
    ln -s "$host_runtime/pipewire-0" "$profile/runtime/pipewire-0"
fi

unset GNOBLIN_CONFIG
export HOME="$profile/home"
export XDG_CONFIG_HOME="$profile/config"
export XDG_DATA_HOME="$profile/data"
export XDG_CACHE_HOME="$profile/cache"
export XDG_STATE_HOME="$profile/state"
export XDG_RUNTIME_DIR="$profile/runtime"
export MESA_SHADER_CACHE_DISABLE=true
export WAYLAND_DISPLAY="$host_display"
export GNOBLIN_COMPOSITOR_SOCKET="$profile/runtime/gnoblin/compositor-v1.sock"
export GNOBLIN_PREFIX="${GNOBLIN_PREFIX:-$root/install}"

bash "$root/scripts/run-gnome-devkit.sh" "$@"
