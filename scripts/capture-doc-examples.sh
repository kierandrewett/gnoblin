#!/usr/bin/env bash
# Capture a fresh Gnoblin desktop with Waybar and Files for the documentation.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
example="${1:-desktop}"
output_dir="${2:-$root/docs/images}"
case "$example" in
    desktop | waybar-firefox | waybar-launcher | bingux-firefox) ;;
    *)
        echo "Usage: $0 [desktop] [output-directory]" >&2
        exit 2
        ;;
esac
docs_url="${GNOBLIN_DOCS_URL:-http://127.0.0.1:5180/gnoblin}"
bingux_config="${GNOBLIN_DOC_BINGUX_PATH:-$root/../bingux/shell/bingux}"
if [ "$(id -u)" -eq 0 ]; then
    echo "Run the capture as a regular user" >&2
    exit 1
fi
required=(grim swaybg waybar nautilus fuzzel mako foot python3 ydotool ydotoold)
for program in "${required[@]}"; do
    command -v "$program" >/dev/null || {
        echo "$program is required to capture the $example scene" >&2
        exit 1
    }
done
mkdir -p "$output_dir"

profile="$(mktemp -d /tmp/gnoblin-doc-example.XXXXXX)"
ydotool_socket="$profile/runtime/ydotool.sock"
ydotoold_pid=
cleanup() {
    [ -z "$ydotoold_pid" ] || kill "$ydotoold_pid" 2>/dev/null || true
    for _ in 1 2 3; do
        rm -rf -- "$profile"
        [ ! -e "$profile" ] && return
        sleep 1
    done
    rm -rf -- "$profile"
}
trap cleanup EXIT
mkdir -m 700 "$profile/home" "$profile/config" "$profile/data" \
    "$profile/cache" "$profile/state" "$profile/runtime"
ydotoold --socket-path="$ydotool_socket" --socket-perm=0600 >"$profile/ydotoold.log" 2>&1 &
ydotoold_pid=$!

host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
host_xdisplay="${DISPLAY:-:0}"
host_display="${WAYLAND_DISPLAY:-}"
case "$host_display" in /* | "") ;; *) host_display="$host_runtime/$host_display" ;; esac

unset GNOBLIN_CONFIG
export HOME="$profile/home"
export XDG_CONFIG_HOME="$profile/config" XDG_DATA_HOME="$profile/data"
export XDG_CACHE_HOME="$profile/cache" XDG_STATE_HOME="$profile/state"
export XDG_RUNTIME_DIR="$profile/runtime" WAYLAND_DISPLAY="$host_display"
export GNOBLIN_STATE_DIR="$profile/state/gnoblin"
export GNOBLIN_PREFIX="${GNOBLIN_DOC_PREFIX:-$root/install}"
GNOBLIN_MUTTER_API="$(python3 "$root/scripts/gnome-versions.py" get mutter api)"
export GNOBLIN_MUTTER_API
unset GNOBLIN_LIBDIR
export GNOBLIN_COMPOSITOR_SOCKET="$profile/runtime/gnoblin/compositor-v1.sock"

# The visible devkit viewer may use the host PipeWire daemon, while all persistent
# state remains in the disposable profile.
if [ -S "$host_runtime/pipewire-0" ]; then
    ln -s "$host_runtime/pipewire-0" "$XDG_RUNTIME_DIR/pipewire-0"
fi

source "$root/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$GNOBLIN_PREFIX"
expected_version="$(python3 "$root/scripts/gnome-versions.py" get gnome-shell version)"
installed_version="$("$GNOBLIN_PREFIX/bin/gnome-shell" --version)"
case "$installed_version" in
    *"$expected_version"*) ;;
    *)
        echo "Build the current Gnoblin source before capturing (expected $expected_version, found $installed_version)" >&2
        exit 1
        ;;
esac

mkdir -p "$XDG_CONFIG_HOME/gnoblin" "$XDG_CONFIG_HOME/waybar" \
    "$XDG_CONFIG_HOME/mako" "$XDG_CONFIG_HOME/foot" "$XDG_CONFIG_HOME/fuzzel"
mkdir -p "$HOME/Documents" "$HOME/Downloads" "$HOME/Pictures"

# Make the packaged vector cursor theme available in the disposable profile.
cursor_theme="${GNOBLIN_DOC_CURSOR_THEME:-$GNOBLIN_PREFIX/share/icons/Adwaita-Hyprcursor}"
if [ ! -d "$cursor_theme/hyprcursors" ] && [ -d "$root/build/Adwaita-Hyprcursor/hyprcursors" ]; then
    cursor_theme="$root/build/Adwaita-Hyprcursor"
fi
if [ ! -d "$cursor_theme/hyprcursors" ] && [ -d /usr/share/icons/Adwaita-Hyprcursor/hyprcursors ]; then
    cursor_theme=/usr/share/icons/Adwaita-Hyprcursor
fi
if [ ! -d "$cursor_theme/hyprcursors" ]; then
    echo "Adwaita-Hyprcursor is required; see docs/guides/cursors.md" >&2
    exit 1
fi
mkdir -p "$XDG_DATA_HOME/icons" "$HOME/.local/share/icons"
ln -s "$cursor_theme" "$XDG_DATA_HOME/icons/Adwaita-Hyprcursor"
ln -s "$cursor_theme" "$HOME/.local/share/icons/Adwaita-Hyprcursor"

cat >"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
}
LUA
if [ "$example" != bingux-firefox ]; then
    cat >>"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    autostart = {
        bar = {command = {"waybar"}},
        notifications = {command = {"mako"}},
    },
    shortcuts = {
        launcher = {binding = "<Super>d", command = {"fuzzel"}},
    },
}
LUA
fi

cat >"$XDG_CONFIG_HOME/waybar/config.jsonc" <<'JSON'
{
  "layer": "top", "position": "top", "height": 42,
  "modules-left": ["custom/brand"],
  "modules-right": ["clock"],
  "custom/brand": {"format": "Gnoblin", "tooltip": false},
  "clock": {"format": "{:%a %d %b  ·  %H:%M}", "tooltip": false}
}
JSON
cat >"$XDG_CONFIG_HOME/waybar/style.css" <<'CSS'
* { border: 0; border-radius: 0; font-family: "Adwaita Sans", sans-serif; font-size: 14px; }
window#waybar { background: #171b27; color: #e8eaf1; }
#custom-brand { color: #9ccfd8; font-weight: 700; padding: 0 18px; }
#clock { color: #c8cbd6; }
CSS
cat >"$XDG_CONFIG_HOME/mako/config" <<'MAKO'
font=Adwaita Sans 11
background-color=#202638
text-color=#edf0f7
border-color=#9ccfd8
border-size=2
border-radius=10
padding=16
width=380
height=110
default-timeout=9000
MAKO
cat >"$XDG_CONFIG_HOME/foot/foot.ini" <<'FOOT'
font=monospace:size=12
pad=18x16
initial-window-size-chars=76x18
[colors]
foreground=dce2f0
background=171b27
regular0=242a3b
regular1=ed8796
regular2=a6da95
regular3=eed49f
regular4=8aadf4
regular5=f5bde6
regular6=8bd5ca
regular7=cad3f5
FOOT
cat >"$XDG_CONFIG_HOME/fuzzel/fuzzel.ini" <<'FUZZEL'
[main]
font=monospace:size=13
width=48
horizontal-pad=22
vertical-pad=16
inner-pad=12
[colors]
background=171b27f2
text=dce2f0ff
match=9ccfd8ff
selection=30384fff
selection-text=edf0f7ff
border=9ccfd8ff
FUZZEL

capture_path="$output_dir/gnoblin-build-a-desktop.png"
case "$example" in
    desktop)
        capture_path="$output_dir/gnoblin-build-a-desktop.png"
        app_command='nautilus --new-window'
        ;;
    waybar-firefox)
        capture_path="$output_dir/gnoblin-waybar-firefox.png"
        app_command="firefox --new-window '$docs_url/bring-your-own-shell.html'"
        ;;
    waybar-launcher)
        capture_path="$output_dir/gnoblin-waybar-launcher.png"
        app_command="firefox --new-window '$docs_url/guides/shortcuts.html'"
        post_app_command='fuzzel & sleep 3'
        ;;
    bingux-firefox)
        capture_path="$output_dir/gnoblin-bingux-firefox.png"
        app_command="gnoblin-quickshell -p '$bingux_config' & sleep 5; firefox --new-window '$docs_url/bring-your-own-shell.html'"
        ;;
esac

if [ "$example" != desktop ]; then
    command -v firefox >/dev/null || {
        echo "firefox is required for this scene" >&2
        exit 1
    }
fi
if [ "$example" = bingux-firefox ]; then
    command -v gnoblin-quickshell >/dev/null || {
        echo "gnoblin-quickshell is required for this scene" >&2
        exit 1
    }
    [ -d "$bingux_config" ] || {
        echo "Bingux shell config not found at $bingux_config" >&2
        exit 1
    }
    mkdir -p "$XDG_CONFIG_HOME/bingux"
    cat >"$XDG_CONFIG_HOME/bingux/settings.json" <<'JSON'
{
  "desktop": {
    "dockApps": {
      "pinnedApps": ["org.gnome.Nautilus.desktop", "org.mozilla.firefox.desktop", "foot.desktop"],
      "order": []
    }
  }
}
JSON
fi

pointer_position="${GNOBLIN_DOC_POINTER:-$(python3 -c 'import ctypes; x=ctypes.CDLL("libX11.so.6"); x.XOpenDisplay.restype=ctypes.c_void_p; x.XOpenDisplay.argtypes=[ctypes.c_char_p]; x.XDefaultScreen.argtypes=[ctypes.c_void_p]; x.XDefaultScreen.restype=ctypes.c_int; x.XDisplayWidth.argtypes=[ctypes.c_void_p,ctypes.c_int]; x.XDisplayWidth.restype=ctypes.c_int; x.XDisplayHeight.argtypes=[ctypes.c_void_p,ctypes.c_int]; x.XDisplayHeight.restype=ctypes.c_int; d=x.XOpenDisplay(None); s=x.XDefaultScreen(d); print(x.XDisplayWidth(d,s)//2, x.XDisplayHeight(d,s)//2)')}"
pointer_command="YDOTOOL_SOCKET='$ydotool_socket' ydotool mousemove --absolute $pointer_position"
post_app_command="${post_app_command:-:}"
desktop_command="swaybg -i /usr/share/backgrounds/fedora-workstation/flight_dark.webp -m fill & sleep 3; $app_command & sleep 9; $post_app_command; $pointer_command; sleep 2; grim -c '$capture_path'; DISPLAY='$host_xdisplay' GNOBLIN_DOC_VIEWPORT_X='${GNOBLIN_DOC_VIEWPORT_X:-}' GNOBLIN_DOC_VIEWPORT_Y='${GNOBLIN_DOC_VIEWPORT_Y:-}' python3 '$root/scripts/composite-doc-cursor.py' '$capture_path'"
export GNOME_DEVKIT_EXEC="$desktop_command"
bash "$root/scripts/run-gnome-devkit.sh"
