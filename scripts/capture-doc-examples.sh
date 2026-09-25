#!/usr/bin/env bash
# Capture a fresh Gnoblin desktop with Waybar and Files for the documentation.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
example="${1:-desktop}"
output_dir="${2:-$root/docs/images}"
case "$example" in
    desktop) ;;
    *)
        echo "Usage: $0 [desktop] [output-directory]" >&2
        exit 2
        ;;
esac
if [ "$(id -u)" -eq 0 ]; then
    echo "Run the capture as a regular user" >&2
    exit 1
fi
required=(grim swaybg waybar nautilus fuzzel mako foot)
for program in "${required[@]}"; do
    command -v "$program" >/dev/null || {
        echo "$program is required to capture the $example scene" >&2
        exit 1
    }
done
mkdir -p "$output_dir"

profile="$(mktemp -d /tmp/gnoblin-doc-example.XXXXXX)"
cleanup() {
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

host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
host_home="$HOME"
host_display="${WAYLAND_DISPLAY:-}"
case "$host_display" in /* | "") ;; *) host_display="$host_runtime/$host_display" ;; esac

unset GNOBLIN_CONFIG
export HOME="$profile/home"
export XDG_CONFIG_HOME="$profile/config" XDG_DATA_HOME="$profile/data"
export XDG_CACHE_HOME="$profile/cache" XDG_STATE_HOME="$profile/state"
export XDG_RUNTIME_DIR="$profile/runtime" WAYLAND_DISPLAY="$host_display"
export GNOBLIN_PREFIX="${GNOBLIN_DOC_PREFIX:-$root/install}"
GNOBLIN_MUTTER_API="$(python3 "$root/scripts/gnome-versions.py" get mutter api)"
export GNOBLIN_MUTTER_API
unset GNOBLIN_LIBDIR
export GNOBLIN_COMPOSITOR_SOCKET="$profile/runtime/gnoblin/compositor-v1.sock"

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

# Make an installed vector cursor theme visible inside the disposable profile.
cursor_theme="${GNOBLIN_DOC_CURSOR_THEME:-$root/install/share/icons/Adwaita-Hyprcursor}"
if [ ! -d "$cursor_theme/hyprcursors" ] && [ -d "$root/build/Adwaita-Hyprcursor/hyprcursors" ]; then
    cursor_theme="$root/build/Adwaita-Hyprcursor"
fi
if [ ! -d "$cursor_theme/hyprcursors" ] && [ -d "$host_home/.local/share/icons/Adwaita-Hyprcursor/hyprcursors" ]; then
    cursor_theme="$host_home/.local/share/icons/Adwaita-Hyprcursor"
fi
if [ ! -d "$cursor_theme/hyprcursors" ] && [ -d /usr/share/icons/Adwaita-Hyprcursor/hyprcursors ]; then
    cursor_theme=/usr/share/icons/Adwaita-Hyprcursor
fi
if [ ! -d "$cursor_theme/hyprcursors" ]; then
    echo "Adwaita-Hyprcursor is required; see docs/guides/cursors.md" >&2
    exit 1
fi
if [ -d "$cursor_theme/hyprcursors" ]; then
    mkdir -p "$XDG_DATA_HOME/icons"
    ln -s "$cursor_theme" "$XDG_DATA_HOME/icons/Adwaita-Hyprcursor"
    # Hyprcursor resolves user themes through ~/.local/share/icons.
    mkdir -p "$HOME/.local/share/icons"
    ln -s "$cursor_theme" "$HOME/.local/share/icons/Adwaita-Hyprcursor"
fi

cat >"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
    autostart = {
        bar = {command = {"waybar"}},
        notifications = {command = {"mako"}},
    },
    shortcuts = {
        launcher = {binding = "<Super>d", command = {"fuzzel"}},
        terminal = {binding = "<Super>Return", command = {"foot"}},
    },
}
LUA

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

waybar_log="$profile/waybar.log"
desktop_command="swaybg -c '#111520' & sleep 2; waybar > '$waybar_log' 2>&1 & for attempt in {1..40}; do if grep -q 'Bar configured' '$waybar_log'; then break; fi; sleep 0.25; done; grep -q 'Bar configured' '$waybar_log' || { cat '$waybar_log' >&2; exit 1; }; nautilus --new-window & sleep 6; grim -c '$output_dir/gnoblin-build-a-desktop.png'"
export GNOME_DEVKIT_EXEC="$desktop_command"
exec bash "$root/scripts/run-gnome-devkit.sh"
