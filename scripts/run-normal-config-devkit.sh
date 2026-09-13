#!/usr/bin/env bash
# Normal user config, copied into a disposable devkit; never blank defaults.
# GNOME_DEVKIT_HEADLESS=1 provides reliable unattended pixel-test output.
set -euo pipefail
task_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_source="${XDG_CONFIG_HOME:-$HOME/.config}"
host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
host_display="${WAYLAND_DISPLAY:-}"
test -f "$config_source/gnoblin/init.lua" || {
    echo "Normal Gnoblin init.lua is missing; refusing to test blank defaults." >&2
    exit 1
}
snapshot="$(mktemp -d /tmp/gnoblin-user-config.XXXXXX)"
mkdir -m 700 "$snapshot/runtime" "$snapshot/config" "$snapshot/data" "$snapshot/cache" "$snapshot/state"
cp -a "$config_source/gnoblin" "$snapshot/config/"
for name in bingux dconf gtk-3.0 gtk-4.0; do
    if [ -d "$config_source/$name" ]; then cp -a "$config_source/$name" "$snapshot/config/"; fi
done
if [ -S "$host_runtime/pipewire-0" ]; then
    ln -s "$host_runtime/pipewire-0" "$snapshot/runtime/pipewire-0"
fi
case "$host_display" in
    /* | "") ;;
    *) host_display="$host_runtime/$host_display" ;;
esac
export GNOBLIN_PREFIX="${GNOBLIN_TEST_PREFIX:-$task_root/install}"
export GNOBLIN_LIBDIR="${GNOBLIN_LIBDIR:-lib64}"
export WAYLAND_DISPLAY="$host_display"
export XDG_RUNTIME_DIR="$snapshot/runtime" XDG_CONFIG_HOME="$snapshot/config"
export XDG_DATA_HOME="$snapshot/data" XDG_CACHE_HOME="$snapshot/cache" XDG_STATE_HOME="$snapshot/state"
export GNOBLIN_COMPOSITOR_SOCKET="$snapshot/runtime/compositor.sock"
export BINGUX_CONFIG_PATH="${BINGUX_CONFIG_PATH:-$task_root/../bingux/shell/bingux}"
export BINGUX_QUICKSHELL="${BINGUX_QUICKSHELL:-gnoblin-quickshell}"
export GSETTINGS_BACKEND=dconf
export GNOME_DEVKIT_UNSAFE_MODE=1
export GNOME_DEVKIT_EXEC="python3 '$task_root/tests/nested-desktop-session.py'"
echo "Normal-config snapshot and proof artifacts: $snapshot"
exec "$task_root/scripts/run-gnome-devkit.sh"
