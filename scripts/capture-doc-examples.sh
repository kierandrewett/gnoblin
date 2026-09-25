#!/usr/bin/env bash
# Capture a clean Gnoblin shell example for the documentation.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
example="${1:-waybar-firefox}"
output_dir="${2:-$root/docs/images}"
case "$example" in
    waybar-firefox | waybar-launcher | bingux-firefox | quickshell-firefox | waybar-files | waybar-settings | waybar-notifications | waybar-mako-notification | bingux-files | quickshell-files | waybar-quickshell-dock | window-effects) ;;
    *)
        echo "Usage: $0 {waybar-firefox|waybar-launcher|bingux-firefox|quickshell-firefox|waybar-files|waybar-settings|waybar-notifications|waybar-mako-notification|bingux-files|quickshell-files|waybar-quickshell-dock|window-effects} [output-directory]" >&2
        exit 2
        ;;
esac
firefox_url="${GNOBLIN_DOC_FIREFOX_URL:-https://help.gnome.org/gnome-help/}"
bingux_root="${GNOBLIN_DOC_BINGUX_ROOT:-$root/../bingux}"
bingux_config="${GNOBLIN_DOC_BINGUX_PATH:-$bingux_root/shell/bingux}"
bingux_base_config="${GNOBLIN_DOC_BINGUX_BASE_CONFIG:-$bingux_root/packaging/gnoblin/bingux.lua}"
bingux_frame_dir="${GNOBLIN_DOC_BINGUX_FRAME_DIR:-$bingux_root/build}"
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
    if [ -n "$ydotoold_pid" ]; then
        kill "$ydotoold_pid" 2>/dev/null || true
        wait "$ydotoold_pid" 2>/dev/null || true
    fi
    for _ in 1 2 3; do
        rm -rf -- "$profile" 2>/dev/null || true
        [ ! -e "$profile" ] && return
        sleep 1
    done
    if [ -e "$profile" ]; then
        printf 'capture-doc-examples: could not remove disposable profile %s\n' "$profile" >&2
        return 1
    fi
}
trap 'cleanup || exit 1' EXIT
mkdir -m 700 "$profile/home" "$profile/config" "$profile/data" \
    "$profile/cache" "$profile/state" "$profile/runtime"
mkdir -p "$profile/config/dconf"
printf 'user-db:user\n' >"$profile/dconf.profile"
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
export XDG_CONFIG_DIRS=/etc/xdg
export GSETTINGS_BACKEND=dconf
export XDG_RUNTIME_DIR="$profile/runtime" WAYLAND_DISPLAY="$host_display"
export DCONF_PROFILE="$profile/dconf.profile"
export MESA_SHADER_CACHE_DISABLE=true
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
# Keep host-specific app and Gnoblin integration directories out of the scene.
# gnoblin_env_apply prepends this build's share directory to /usr/share.
export XDG_DATA_DIRS=/usr/share
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

if [ "$example" = bingux-firefox ] || [ "$example" = bingux-files ]; then
    command -v gnoblin-quickshell >/dev/null || {
        echo "gnoblin-quickshell is required for this scene" >&2
        exit 1
    }
    [ -d "$bingux_config" ] || {
        echo "Bingux shell config not found at $bingux_config" >&2
        exit 1
    }
    [ -f "$bingux_base_config" ] || {
        echo "Bingux's packaged Gnoblin defaults not found at $bingux_base_config" >&2
        exit 1
    }
    if [ -x "$bingux_frame_dir/bingux-frame" ]; then
        PATH="$bingux_frame_dir:$PATH"
        export PATH
    fi
    command -v bingux-frame >/dev/null || {
        echo "bingux-frame is required to show Bingux's installed window-frame defaults" >&2
        exit 1
    }
    mkdir -p "$XDG_CONFIG_HOME/gnoblin/conf.d"
    cp -- "$bingux_base_config" "$XDG_CONFIG_HOME/gnoblin/conf.d/bingux.lua"
    cat >"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
local gnoblin = require("gnoblin")
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
}
gnoblin.load("conf.d/**/*.lua")
LUA
else
    cat >"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
}
LUA
fi
if [ "$example" = window-effects ]; then
    cat >>"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.window_rule {
    match = {type = "window"},
    corners = {
        radius = 20,
        smoothing = 0.55,
        mode = "force",
        shadow = {x = 0, y = 12, blur = 32, spread = 0, opacity = 0.28},
    },
}
LUA
fi
if [ "$example" = waybar-mako-notification ]; then
    cat >>"$XDG_CONFIG_HOME/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    shell = {notifications = false},
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
anchor=bottom-right
margin=24
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
lines=3
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

if [ "$example" = quickshell-firefox ] || [ "$example" = quickshell-files ] || [ "$example" = waybar-quickshell-dock ]; then
    command -v quickshell >/dev/null || {
        echo "quickshell is required for this scene" >&2
        exit 1
    }
    mkdir -p "$XDG_CONFIG_HOME/quickshell"
    cat >"$XDG_CONFIG_HOME/quickshell/shell.qml" <<'QML'
import Quickshell
import QtQuick

PanelWindow {
    anchors { top: true; left: true; right: true }
    implicitHeight: 42
    color: "#171b27"

    Text {
        id: clock
        anchors.centerIn: parent
        text: Qt.formatDateTime(new Date(), "ddd, dd MMM  ·  HH:mm")
        color: "#e8eaf1"
        font.family: "Adwaita Sans"
        font.pixelSize: 15
    }

    Timer {
        interval: 30000
        running: true
        repeat: true
        onTriggered: clock.text = Qt.formatDateTime(new Date(), "ddd, dd MMM  ·  HH:mm")
    }
}
QML
fi
if [ "$example" = waybar-quickshell-dock ]; then
    cat >"$XDG_CONFIG_HOME/quickshell/shell.qml" <<'QML'
import Quickshell
import Quickshell.Widgets
import QtQuick

PanelWindow {
    anchors { bottom: true; left: true; right: true }
    margins.bottom: 18
    implicitHeight: 88
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
        anchors.centerIn: parent
        width: 280
        height: 72
        radius: 24
        color: "#e8171b27"
        border.color: "#657080"
        border.width: 1

        Row {
            anchors.centerIn: parent
            spacing: 18

            Repeater {
                model: [
                    {icon: "org.gnome.Nautilus", command: ["nautilus"], running: true},
                    {icon: "firefox", command: ["firefox"], running: false},
                    {icon: "utilities-terminal", command: ["foot"], running: false},
                    {icon: "org.gnome.Settings", command: ["gnome-control-center"], running: false}
                ]

                delegate: Item {
                    required property var modelData
                    width: 42
                    height: 50

                    IconImage {
                        anchors.top: parent.top
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 40
                        height: 40
                        source: Quickshell.iconPath(modelData.icon)
                    }

                    Rectangle {
                        anchors.bottom: parent.bottom
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 5
                        height: 5
                        radius: 3
                        color: "#9ccfd8"
                        visible: modelData.running
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: Quickshell.execDetached(modelData.command)
                    }
                }
            }
        }
    }
}
QML
fi

firefox_profile="$HOME/.mozilla/firefox/gnoblin-docs"
firefox_command="firefox --no-remote --profile '$firefox_profile'"
case "$example" in
    waybar-firefox)
        capture_path="$output_dir/gnoblin-waybar-firefox.png"
        app_command="waybar & mako & sleep 2; $firefox_command --new-window '$firefox_url'"
        ;;
    waybar-launcher)
        capture_path="$output_dir/gnoblin-waybar-launcher.png"
        app_command="waybar & mako & sleep 2; $firefox_command --new-window '$firefox_url' & sleep 5; fuzzel & sleep 3; YDOTOOL_SOCKET='$ydotool_socket' ydotool type Firefox; sleep 2"
        ;;
    bingux-firefox)
        capture_path="$output_dir/gnoblin-bingux-firefox.png"
        app_command="gnoblin-quickshell -p '$bingux_config' & sleep 5; $firefox_command --new-window '$firefox_url'"
        ;;
    quickshell-firefox)
        capture_path="$output_dir/gnoblin-quickshell-firefox.png"
        app_command="quickshell -p '$XDG_CONFIG_HOME/quickshell/shell.qml' & sleep 3; $firefox_command --new-window '$firefox_url'"
        ;;
    waybar-files)
        capture_path="$output_dir/gnoblin-waybar-files.png"
        app_command='waybar & mako & sleep 2; nautilus --new-window'
        ;;
    waybar-settings)
        capture_path="$output_dir/gnoblin-waybar-settings.png"
        app_command='waybar & mako & sleep 2; gnome-control-center multitasking'
        ;;
    waybar-notifications)
        capture_path="$output_dir/gnoblin-waybar-notifications.png"
        app_command='waybar & mako & sleep 2; gnome-control-center notifications'
        ;;
    waybar-mako-notification)
        capture_path="$output_dir/gnoblin-waybar-mako-notification.png"
        app_command="waybar & mako & sleep 2; $firefox_command --new-window '$firefox_url' & sleep 6; notify-send --expire-time=30000 --app-name='Calendar' --icon=appointment-soon 'Project review' 'Starts in 10 minutes'"
        ;;
    window-effects)
        capture_path="$output_dir/gnoblin-window-effects.png"
        app_command="waybar & mako & sleep 2; $firefox_command --new-window '$firefox_url'"
        post_app_command='gnoblinctl window unmaximize active 2>/dev/null || true; gnoblinctl window resize active 1000 680; gnoblinctl window move active 140 60'
        ;;
    bingux-files)
        capture_path="$output_dir/gnoblin-bingux-files.png"
        app_command="gnoblin-quickshell -p '$bingux_config' & sleep 5; nautilus --new-window"
        ;;
    quickshell-files)
        capture_path="$output_dir/gnoblin-quickshell-files.png"
        app_command="quickshell -p '$XDG_CONFIG_HOME/quickshell/shell.qml' & sleep 3; nautilus --new-window"
        ;;
    waybar-quickshell-dock)
        capture_path="$output_dir/gnoblin-waybar-quickshell-dock.png"
        app_command="waybar & mako & sleep 2; quickshell -p '$XDG_CONFIG_HOME/quickshell/shell.qml' & sleep 3; nautilus --new-window"
        ;;
esac

case "$example" in
    waybar-files | waybar-settings | waybar-notifications | bingux-files | quickshell-files | waybar-quickshell-dock) ;;
    *)
        command -v firefox >/dev/null || {
            echo "firefox is required for this scene" >&2
            exit 1
        }
        mkdir -p "$firefox_profile"
        cat >"$firefox_profile/user.js" <<'PREFS'
// Keep first-run pages out of screenshots while retaining a genuinely fresh profile.
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("browser.startup.homepage_override.buildID", "ignore");
user_pref("browser.rights.3.shown", true);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);
user_pref("datareporting.policy.dataSubmissionPolicyBypassNotification", true);
PREFS
        ;;
esac
if [ "$example" = waybar-settings ] || [ "$example" = waybar-notifications ]; then
    command -v gnome-control-center >/dev/null || {
        echo "gnome-control-center is required for this scene" >&2
        exit 1
    }
fi
if [ "$example" = waybar-mako-notification ]; then
    command -v notify-send >/dev/null || {
        echo "notify-send is required for this scene" >&2
        exit 1
    }
fi
if [ "$example" = bingux-firefox ] || [ "$example" = bingux-files ]; then
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

pointer_position="${GNOBLIN_DOC_POINTER:-1160 700}"
if [ -z "${GNOBLIN_DOC_POINTER:-}" ]; then
    case "$example" in
        waybar-launcher) pointer_position="1100 700" ;;
        waybar-settings) pointer_position="900 450" ;;
        waybar-files) pointer_position="1000 560" ;;
        bingux-firefox) pointer_position="1120 650" ;;
        quickshell-files | bingux-files) pointer_position="800 400" ;;
        waybar-quickshell-dock) pointer_position="790 668" ;;
        waybar-firefox | quickshell-firefox | waybar-mako-notification | window-effects) pointer_position="1100 700" ;;
        *) pointer_position="900 700" ;;
    esac
fi
pointer_command="YDOTOOL_SOCKET='$ydotool_socket' ydotool mousemove --absolute $pointer_position"
post_app_command="${post_app_command:-:}"
desktop_command="set -e; gsettings set org.gnome.desktop.interface color-scheme prefer-dark; gsettings set org.gnome.desktop.interface gtk-theme Adwaita-dark; swaybg -i /usr/share/backgrounds/fedora-workstation/flight_dark.webp -m fill & sleep 3; $app_command & sleep 9; $post_app_command; $pointer_command; sleep 2; grim '$capture_path'; DISPLAY='$host_xdisplay' GNOBLIN_DOC_POINTER='$pointer_position' GNOBLIN_DOC_VIEWPORT_X='${GNOBLIN_DOC_VIEWPORT_X:-}' GNOBLIN_DOC_VIEWPORT_Y='${GNOBLIN_DOC_VIEWPORT_Y:-}' python3 '$root/scripts/composite-doc-cursor.py' '$capture_path'"
export GNOME_DEVKIT_EXEC="$desktop_command"
bash "$root/scripts/run-gnome-devkit.sh"
