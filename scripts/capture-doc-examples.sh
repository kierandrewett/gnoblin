#!/usr/bin/env bash
# Capture the real Gnoblin desktop and developer console for the documentation.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
example="${1:-all}"
output_dir="${2:-$root/docs/images}"
case "$example" in
    desktop | console | all) ;;
    *) echo "Usage: $0 [desktop|console|all] [output-directory]" >&2; exit 2 ;;
esac
required=(grim swaybg gdbus)
if [ "$example" != console ]; then
    required+=(waybar fuzzel mako notify-send foot)
fi
for program in "${required[@]}"; do
    command -v "$program" >/dev/null || {
        echo "$program is required to capture the $example scene" >&2
        exit 1
    }
done
mkdir -p "$output_dir"

profile="$(mktemp -d /tmp/gnoblin-doc-example.XXXXXX)"
cleanup() {
    for attempt in 1 2 3; do
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
if [ -S "$host_runtime/pipewire-0" ]; then
    ln -s "$host_runtime/pipewire-0" "$profile/runtime/pipewire-0"
fi

unset GNOBLIN_CONFIG
export HOME="$profile/home"
export XDG_CONFIG_HOME="$profile/config" XDG_DATA_HOME="$profile/data"
export XDG_CACHE_HOME="$profile/cache" XDG_STATE_HOME="$profile/state"
export XDG_RUNTIME_DIR="$profile/runtime" WAYLAND_DISPLAY="$host_display"
export GNOBLIN_PREFIX="${GNOBLIN_PREFIX:-$root/install}"
export GNOBLIN_COMPOSITOR_SOCKET="$profile/runtime/gnoblin/compositor-v1.sock"

mkdir -p "$XDG_CONFIG_HOME/gnoblin" "$XDG_CONFIG_HOME/waybar" \
    "$XDG_CONFIG_HOME/mako" "$XDG_CONFIG_HOME/foot" "$XDG_CONFIG_HOME/fuzzel"

# Make an installed vector cursor theme visible inside the disposable profile.
cursor_theme="${GNOBLIN_DOC_CURSOR_THEME:-$root/build/Adwaita-Hyprcursor}"
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

cat > "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
}
LUA
if [ "$example" != console ]; then
    cat >> "$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.autostart {name = "bar", command = {"waybar"}}
gnoblin.autostart {name = "notifications", command = {"mako"}}
gnoblin.shortcut {name = "launcher", binding = "<Super>d", command = {"fuzzel"}}
gnoblin.shortcut {name = "terminal", binding = "<Super>Return", command = {"foot"}}
LUA
fi

cat > "$XDG_CONFIG_HOME/waybar/config.jsonc" <<'JSON'
{
  "layer": "top", "position": "top", "height": 42,
  "modules-left": ["custom/brand", "clock"],
  "modules-right": ["custom/session"],
  "custom/brand": {"format": "󰣇  GNOBLIN", "tooltip": false},
  "clock": {"format": "  %A, %d %B   ·   %H:%M", "tooltip": false},
  "custom/session": {"format": "Super  D  ·  launcher", "tooltip": false}
}
JSON
cat > "$XDG_CONFIG_HOME/waybar/style.css" <<'CSS'
* { border: 0; border-radius: 0; font-family: "Adwaita Sans", sans-serif; font-size: 14px; }
window#waybar { background: #171b27; color: #e8eaf1; }
#custom-brand { color: #9ccfd8; font-weight: 700; padding: 0 18px; }
#clock { color: #c8cbd6; }
#custom-session { color: #a6accd; padding: 0 18px; }
CSS
cat > "$XDG_CONFIG_HOME/mako/config" <<'MAKO'
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
cat > "$XDG_CONFIG_HOME/foot/foot.ini" <<'FOOT'
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
cat > "$XDG_CONFIG_HOME/fuzzel/fuzzel.ini" <<'FUZZEL'
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

export GNOBLIN_MUTTER_API="${GNOBLIN_MUTTER_API:-17}"
export GNOBLIN_LIBDIR="${GNOBLIN_LIBDIR:-lib64}"
export GNOBLIN_PREFIX

desktop_command="swaybg -c '#111520' & sleep 2; waybar & sleep 4; notify-send 'Downloads' 'Archive ready'; sleep 1; grim -c '$output_dir/gnoblin-mako-notification.png'; fuzzel --prompt='Search apps  ›  ' --search=Firefox & sleep 5; grim -c '$output_dir/gnoblin-build-a-desktop.png'"
console_command="swaybg -c '#111520' & sleep 2; gdbus call --session --dest org.gnoblin.ConsolePreview --object-path /org/gnoblin/ConsolePreview --method org.gnoblin.ConsolePreview.Open; sleep 3; grim '$output_dir/gnoblin-developer-console.png'"

if [ "$example" = desktop ]; then
    export GNOME_DEVKIT_EXEC="$desktop_command"
fi

if [ "$example" = console ] || [ "$example" = all ]; then
    mkdir -p "$XDG_CONFIG_HOME/gnoblin/scripts"
    cat > "$XDG_CONFIG_HOME/gnoblin/scripts/console-preview.js" <<'JS'
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
const iface = `<node><interface name="org.gnoblin.ConsolePreview"><method name="Open"><arg type="b" direction="out"/></method></interface></node>`;
const object = Gio.DBusExportedObject.wrapJSObject(iface, {
    Open() {
        const opened = Main.openDevConsole();
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            Main.devConsole._hideCompletions();
            return GLib.SOURCE_REMOVE;
        });
        return opened;
    },
});
object.export(Gio.DBus.session, "/org/gnoblin/ConsolePreview");
Gio.bus_own_name(Gio.BusType.SESSION, "org.gnoblin.ConsolePreview", Gio.BusNameOwnerFlags.NONE, null, null, null);
export default function () {}
JS
    export GNOME_DEVKIT_EXEC="$console_command"
fi

if [ "$example" = all ]; then
    # Capture both states in one real session. The console capture is run first
    # in a separate session below, so each image starts from a clean profile.
    export GNOME_DEVKIT_EXEC="$console_command"
    bash "$root/scripts/run-gnome-devkit.sh"
    export GNOBLIN_COMPOSITOR_SOCKET="$profile/runtime/gnoblin/compositor-v1.sock"
    # The same disposable profile is safe to reuse after its process tree exits.
    export GNOME_DEVKIT_EXEC="$desktop_command"
fi

exec bash "$root/scripts/run-gnome-devkit.sh"
