#!/usr/bin/env bash
# Guest: prove that a layer panel's alpha and its compositor blur share one fade.
set -euo pipefail
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the installed prefix}/bin/gnoblinctl"
QS="${GNOBLIN_QS:-qs}"
CONFIG="$HOME/.config/gnoblin/conf.d/99-test-blur-fade.lua"
QML="/tmp/gnoblin-blur-fade.qml"
ARTIFACT_DIR="${GNOBLIN_BLUR_ARTIFACT_DIR:-$(mktemp -d /tmp/gnoblin-blur-fade.XXXXXX)}"
PID=

if [ -e "$CONFIG" ] || [ -e "$QML" ]; then
    echo "FAIL refusing to overwrite an existing blur-fade test file"
    exit 1
fi

cleanup() {
    if [ -n "$PID" ]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -f "$CONFIG" "$QML"
    if [ -z "${GNOBLIN_BLUR_ARTIFACT_DIR:-}" ]; then
        rm -rf "$ARTIFACT_DIR"
    fi
    "$G" config reload >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "$(dirname "$CONFIG")"
mkdir -p "$ARTIFACT_DIR"
cat >"$CONFIG" <<'EOF'
gnoblin.window_rule {
    match = {type = "window", layer = "^gnoblin-blur-fade-panel$"},
    blur = 24,
}
EOF
cat >"$QML" <<'EOF'
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

ShellRoot {
    IpcHandler {
        target: "fade"
        function set(value: real): void { panel.opacity = value; }
    }

    PanelWindow {
        anchors { top: true; bottom: true; left: true; right: true }
        WlrLayershell.layer: WlrLayer.Background
        WlrLayershell.namespace: "gnoblin-blur-fade-pattern"
        color: "black"
        Grid {
            columns: 160
            Repeater {
                model: 160 * 100
                Rectangle {
                    required property int index
                    width: 8
                    height: 8
                    color: (index % 160 + Math.floor(index / 160)) % 2 ? "#eeeeee" : "#222222"
                }
            }
        }
    }

    PanelWindow {
        anchors { top: true; left: true }
        implicitWidth: 1280
        implicitHeight: 800
        surfaceFormat.opaque: false
        WlrLayershell.layer: WlrLayer.Top
        WlrLayershell.namespace: "gnoblin-blur-fade-panel"
        color: "transparent"
        Rectangle {
            id: panel
            x: 64
            y: 64
            width: 192
            height: 96
            radius: 16
            color: "#80303030"
        }
    }
}
EOF

"$G" config reload >/dev/null
"$QS" -p "$QML" --no-color >/tmp/gnoblin-blur-fade.log 2>&1 &
PID=$!
sleep 2
for percent in 0 25 50 75 100; do
    alpha="$(awk -v value="$percent" 'BEGIN { printf "%.2f", value / 100 }')"
    "$QS" ipc -p "$QML" call fade set "$alpha"
    sleep 0.25
    grim "$ARTIFACT_DIR/$percent.png"
done

GNOBLIN_BLUR_ARTIFACT_DIR="$ARTIFACT_DIR" python3 - <<'PY'
import os
from PIL import Image, ImageChops, ImageStat

root = os.environ["GNOBLIN_BLUR_ARTIFACT_DIR"]
background = Image.open(os.path.join(root, "0.png")).convert("RGB")
full = Image.open(os.path.join(root, "100.png")).convert("RGB")
crop = (56, 56, 264, 168)
for percent in (25, 50, 75):
    actual = Image.open(os.path.join(root, f"{percent}.png")).convert("RGB")
    expected = Image.blend(background, full, percent / 100)
    delta = ImageStat.Stat(ImageChops.difference(actual, expected).crop(crop))
    if max(delta.mean) > 2 or max(high for low, high in delta.extrema) > 5:
        raise SystemExit(f"FAIL blur and panel alpha diverge at {percent}%: {delta.mean}, {delta.extrema}")
    print(f"pixel fade {percent}%: mean error {delta.mean}; max error {[high for low, high in delta.extrema]}")
print("PASS: layer blur and panel background follow the same 0-100% opacity fade")
print(f"pixel captures: {root}")
PY

kill "$PID"
wait "$PID" 2>/dev/null || true
PID=
