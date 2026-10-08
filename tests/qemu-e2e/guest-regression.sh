#!/usr/bin/env bash
# Runs inside the Gnoblin guest. Re-checks the features added in this work and prints PASS or FAIL.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG_DIR="$HOME/.config/gnoblin/config"
coredump_before="$(coredumpctl list --no-pager 2>/dev/null | tail -1)"
fail=0

check() {
    # check NAME HAYSTACK NEEDLE
    if printf '%s' "$2" | grep -q -F -- "$3"; then
        echo "PASS $1"
    else
        echo "FAIL $1 (missing: $3)"
        fail=$((fail + 1))
    fi
}

install_config() { cp "/tmp/$1" "$CONFIG_DIR/$1"; "$G" config reload >/dev/null; }
disable_config() { printf -- '-- disabled by the regression script\n' > "$CONFIG_DIR/$1"; "$G" config reload >/dev/null; }

caps="$("$G" capabilities 2>&1 | python3 -c '
import json, sys
for c in json.load(sys.stdin):
    print(c["id"], c["available"])')"
check "capability auth-agent off by default" "$caps" "auth-agent False"
check "capability portal-grants" "$caps" "portal-grants True"
check "capability prompt-broker present" "$caps" "prompt-broker"
check "privacy state" "$("$G" privacy 2>&1)" "microphone_in_use"
check "grant list" "$("$G" grant list 2>&1)" '"grants"'
check "version build id" "$("$G" --version 2>&1)" "Build ID:"

echo "-- capture"
: > /tmp/capture-events.log
install_config 99-test-capture.lua
sleep 15
log="$(cat /tmp/capture-events.log)"
check "capture echo" "$log" "echo exit=0 stdout=[hello"
check "capture exit code" "$log" "exit3 exit=3"
check "capture stdin" "$log" "stdin exit=0 stdout=[from-stdin]"
check "capture timeout" "$log" "timeout error=command timed out"
check "capture output limit" "$log" "big error=command output exceeded"
check "capture missing program" "$log" "missing error="
check "capture signal" "$log" "signal exit=-15"
check "capture finished" "$log" "all steps done"
disable_config 99-test-capture.lua

echo "-- polkit"
: > /tmp/doc-events.log
install_config 99-test-doc.lua
auth_caps() {
    "$G" capabilities 2>&1 | python3 -c '
import json, sys
for c in json.load(sys.stdin):
    if c["id"] == "auth-agent":
        print(c["id"], c["available"])'
}
check "capability auth-agent on with polkit_agent" "$(auth_caps)" "auth-agent True"
sleep 12
check "pkexec result" "$(head -1 /tmp/pk3.out 2>&1)" "0"
check "auth events" "$(cat /tmp/doc-events.log)" "authorized"
disable_config 99-test-doc.lua
check "capability auth-agent off after disable" "$(auth_caps)" "auth-agent False"

echo "-- prompt broker"
install_config 99-test-prompts.lua
sleep 3
: > /tmp/prompt-events.log
check "broker owns names" "$(busctl --user list --no-pager 2>/dev/null | grep -i 'keyring.*SystemPrompter')" "gnoblin"
python3 - <<'PY' >/dev/null 2>&1
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
def call(method, args):
    return bus.call_sync("org.freedesktop.secrets", "/org/freedesktop/secrets",
                         "org.freedesktop.Secret.Service", method, args, None,
                         Gio.DBusCallFlags.NONE, 60000, None)
collection = call("ReadAlias", GLib.Variant("(s)", ("default",))).unpack()[0]
call("Lock", GLib.Variant("(ao)", ([collection],)))
PY
check "keyring unlock through broker" "$(timeout 40 secret-tool lookup app prompt-e2e 2>&1)" "secret-value"
export GNUPGHOME="$HOME/.gnupg-e2e"
gpgconf --kill gpg-agent 2>/dev/null
signed="$(echo hello | timeout 90 gpg --batch --yes --pinentry-mode ask --clearsign -u gnoblin-e2e@example.com 2>&1)"
check "gpg passphrase through broker" "$signed" "BEGIN PGP SIGNED MESSAGE"
check "prompt events" "$(cat /tmp/prompt-events.log)" "Unlock Keyring"
disable_config 99-test-prompts.lua
sleep 2
check "broker released names" "$(busctl --user list --no-pager 2>/dev/null | grep -i 'keyring.*SystemPrompter')" "activatable"

echo "-- input sources"
check "input sources list" "$("$G" input list 2>&1)" "English (US)"
check "select xkb source" "$("$G" input select xkb us 2>&1)" '"current":true'
check "current xkb source" "$("$G" input current 2>&1)" '"id":"us"'
pgrep -x ibus-daemon >/dev/null || ibus-daemon -drx >/dev/null 2>&1
sleep 4
check "select ibus source" "$("$G" input select ibus m17n:ru:translit 2>&1)" '"current":true'
check "current ibus source" "$("$G" input current 2>&1)" '"type":"ibus"'
check "restore xkb source" "$("$G" input select xkb us 2>&1)" '"current":true'

echo "-- control surface"
check "ping" "$("$G" ping 2>&1)" "pong"
check "status running" "$("$G" status 2>&1)" '"state":"running"'
check "workspace list" "$("$G" workspace list 2>&1)" "Workspace 1"
check "monitor list" "$("$G" monitor list 2>&1)" "refresh_rate"
check "focus policy" "$("$G" focus policy 2>&1)" "focus_mode"
nohup swaybg -c "#223344" >/dev/null 2>&1 < /dev/null &
nohup foot -T regression-window >/dev/null 2>&1 < /dev/null &
sleep 4
wallpaper_layers() {
    # Report whether swaybg's wallpaper layer surfaces exist (one for each monitor). Other layer clients (a notification daemon, for example) may come and go.
    "$G" layer list 2>&1 | python3 -c '
import json, sys
data = json.load(sys.stdin)
layers = data.get("layers", data)
print("wallpaper_layers=%s" % ("present" if any(layer.get("namespace") == "wallpaper" for layer in layers) else "none"))'
}
check "layer list sees the wallpaper layer surfaces" "$(wallpaper_layers)" "wallpaper_layers=present"
check "window list sees a window" "$("$G" window list 2>&1)" "regression-window"
pkill -x swaybg
pkill -f "foot -T regression-window"
sleep 2
check "wallpaper layers removed after exit" "$(wallpaper_layers)" "wallpaper_layers=none"
check "window removed after exit" "$("$G" window list 2>&1)" '"windows":[]'

echo "-- window rules"
install_config 99-test-rules.lua
nohup foot -T rule-test >/dev/null 2>&1 < /dev/null &
nohup foot -T rule-control >/dev/null 2>&1 < /dev/null &
sleep 4
rule_windows="$("$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    print(w["title"], "workspace", w["workspace_number"])')"
check "rule places the matching window on workspace 2" "$rule_windows" "rule-test workspace 2"
check "rule leaves other windows alone" "$rule_windows" "rule-control workspace 1"
pkill -f "foot -T rule-"
sleep 1
disable_config 99-test-rules.lua

echo "-- opacity rule"
window_pixel() {
    # window_pixel: print the red channel at the centre of the opacity-test window
    local geometry
    geometry="$("$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "opacity-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + f["height"] // 2)')"
    grim -t ppm /tmp/opacity.ppm
    python3 - $geometry <<'PY'
import sys
x, y = int(sys.argv[1]), int(sys.argv[2])
data = open("/tmp/opacity.ppm", "rb").read()
parts = data.split(b"\n", 3)
width = int(parts[1].split()[0])
print(parts[3][(y * width + x) * 3])
PY
}
nohup swaybg -c "#ffffff" >/dev/null 2>&1 < /dev/null &
nohup foot -T opacity-test >/dev/null 2>&1 < /dev/null &
sleep 4
before="$(window_pixel)"
install_config 99-test-opacity.lua
sleep 3
after="$(window_pixel)"
if [ "$before" -lt 80 ] && [ "$after" -gt 120 ]; then
    echo "PASS opacity rule lightens the window over a white wallpaper ($before -> $after)"
else
    echo "FAIL opacity rule lightens the window over a white wallpaper ($before -> $after)"
    fail=$((fail + 1))
fi
pkill -x swaybg
pkill -f "foot -T opacity-test"
sleep 1
disable_config 99-test-opacity.lua

echo "-- GNOME interchangeability"
polluted=""
for path in /org/gnome/desktop/wm/keybindings/ /org/gnome/mutter/keybindings/ /org/gnome/shell/keybindings/ /org/gnome/mutter/; do
    dumped="$(dconf dump "$path" 2>/dev/null)"
    [ -z "$dumped" ] || polluted="$polluted $path"
done
if [ -z "$polluted" ]; then
    echo "PASS Gnoblin sessions leave GNOME keybinding and mutter settings unset in dconf"
else
    echo "FAIL Gnoblin sessions wrote GNOME settings into dconf:$polluted"
    fail=$((fail + 1))
fi

echo "-- protocol reload"
screencopy_globals() { wayland-info 2>/dev/null | grep -c 'zwlr_screencopy_manager_v1'; }
printf 'gnoblin.configure {protocols = {wlr_screencopy = false}}\n' > "$CONFIG_DIR/99-test-protocols.lua"
"$G" config reload >/dev/null
sleep 2
check "one reload hides a disabled protocol" "screencopy=$(screencopy_globals)" "screencopy=0"
disable_config 99-test-protocols.lua
sleep 2
check "one reload advertises the protocol again" "screencopy=$(screencopy_globals)" "screencopy=1"

echo "-- xwayland reload"
xauth_file="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
xft_dpi() { XAUTHORITY="$xauth_file" DISPLAY=:0 timeout 10 xrdb -query 2>&1 | grep -i 'Xft.dpi' | tr -d '\t'; }
xwayland_pid() { pgrep -u "$(id -u)" -x Xwayland | head -1; }
printf 'gnoblin.configure {xwayland = {scaling_factor = 2}}\n' > "$CONFIG_DIR/99-test-xwayland.lua"
pid_before="$(xwayland_pid)"
"$G" config reload >/dev/null
sleep 4
check "scaling_factor updates Xft.dpi live" "$(xft_dpi)" "Xft.dpi:192"
check "scaling_factor does not restart Xwayland" "pid=$(xwayland_pid)" "pid=$pid_before"
printf 'gnoblin.configure {xwayland = {disable_extensions = {"xtest"}}}\n' > "$CONFIG_DIR/99-test-xwayland.lua"
"$G" config reload >/dev/null
sleep 6
check "disable_extensions restarts Xwayland without ending the session" \
    "pid_changed=$([ "$(xwayland_pid)" != "$pid_before" ] && echo yes || echo no) compositor=$(pgrep -x gnoblin | wc -l)" \
    "pid_changed=yes compositor=4"
disable_config 99-test-xwayland.lua
sleep 6

echo "-- xsettings"
cat > /tmp/xsettings-byte-order.py <<'PY'
import ctypes
import ctypes.util

x = ctypes.CDLL(ctypes.util.find_library("X11"))
for name in ("XOpenDisplay", "XDefaultRootWindow"):
    getattr(x, name).restype = ctypes.c_void_p
x.XInternAtom.restype = ctypes.c_ulong
x.XGetSelectionOwner.restype = ctypes.c_ulong
display = ctypes.c_void_p(x.XOpenDisplay(None))
selection = x.XInternAtom(display, b"_XSETTINGS_S0", 0)
setting = x.XInternAtom(display, b"_XSETTINGS_SETTINGS", 0)
owner = x.XGetSelectionOwner(display, ctypes.c_ulong(selection))
atom_type, fmt, count, after = ctypes.c_ulong(), ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
data = ctypes.POINTER(ctypes.c_ubyte)()
x.XGetWindowProperty(display, ctypes.c_ulong(owner), ctypes.c_ulong(setting), 0, 64, 0, 0,
                     ctypes.byref(atom_type), ctypes.byref(fmt), ctypes.byref(count), ctypes.byref(after),
                     ctypes.byref(data))
print("byte_order=%s" % (data[0] if count.value else "missing"))
PY
xs_auth="$(ls /run/user/"$(id -u)"/.mutter-Xwaylandauth.* 2>/dev/null | head -1)"
check "XSETTINGS byte order is LSBFirst (0), not a letter" \
    "$(XAUTHORITY="$xs_auth" DISPLAY=:0 python3 /tmp/xsettings-byte-order.py 2>&1 | tail -1)" "byte_order=0"
check "no Invalid XSETTINGS warning from GTK X11 clients" \
    "invalid=$(sudo journalctl -b --no-pager --since '-2min' 2>/dev/null | grep -c 'Invalid XSETTINGS')" "invalid=0"

cat > /tmp/gtk-scale.py <<'PY'
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window()
window.present()
loop = GLib.MainLoop()


def report():
    surface = window.get_surface()
    print("scale_factor=%s" % (surface.get_scale_factor() if surface else None))
    loop.quit()
    return False


GLib.timeout_add(1500, report)
loop.run()
PY
x11_scale() { XAUTHORITY="$xs_auth" DISPLAY=:0 GDK_BACKEND=x11 timeout 20 python3 /tmp/gtk-scale.py 2>&1 | grep '^scale_factor=' | head -1; }
printf 'gnoblin.configure {xwayland = {scaling_factor = 1}}\n' > "$CONFIG_DIR/99-test-xwayland.lua"
"$G" config reload >/dev/null
sleep 4
check "a GTK X11 app sees scale factor 1 at xwayland.scaling_factor 1" "$(x11_scale)" "scale_factor=1"
printf 'gnoblin.configure {xwayland = {scaling_factor = 2}}\n' > "$CONFIG_DIR/99-test-xwayland.lua"
"$G" config reload >/dev/null
sleep 4
check "a GTK X11 app sees scale factor 2 at xwayland.scaling_factor 2" "$(x11_scale)" "scale_factor=2"
disable_config 99-test-xwayland.lua
sleep 3

echo "-- failed reload"
active_cursor_size() { "$G" config show 2>&1 | grep -o '"size":[0-9]*' | head -1; }
printf 'gnoblin.configure {cursor = {size = 48}}\n' > "$CONFIG_DIR/99-test-badreload.lua"
"$G" config reload >/dev/null
check "a valid reload applies cursor.size" "$(active_cursor_size)" '"size":48'
printf 'gnoblin.configure {cursor = {size = 48}, window_management = {focus_mode = "bogus"}}\n' > "$CONFIG_DIR/99-test-badreload.lua"
bad_output="$("$G" config reload 2>&1)"
bad_status=$?
check "an invalid reload exits with an error" "status=$bad_status" "status=1"
check "the error names the failing setting" "$bad_output" 'window-management "focus-mode"'
check "the previous config stays active after a failed reload" "$(active_cursor_size)" '"size":48'
check "the compositor keeps running after a failed reload" "$("$G" status 2>&1)" '"state":"running"'
# Wrong-typed input values used to abort the Lua worker (g_variant_iter_init on a non-container). Each one must be
# rejected with a message that names the setting.
reject_input() {
    # reject_input NAME LUA EXPECTED_MESSAGE
    printf 'gnoblin.configure {input = %s}\n' "$2" > "$CONFIG_DIR/99-test-badreload.lua"
    check "$1" "$("$G" config reload 2>&1)" "$3"
}
reject_input "input with a number where a group table belongs" "{keyboard = 5}" "input.keyboard must be a table"
reject_input "input.keyboard.xkb_options that is not a list" "{keyboard = {xkb_options = 5}}" "input.keyboard.xkb-options"
reject_input "input.keyboard.xkb_options with non-string items" "{keyboard = {xkb_options = {1, 2}}}" "input.keyboard.xkb-options"
reject_input "input.mouse.accel_curve that is not a table" "{mouse = {accel_curve = 7}}" "input.mouse.accel-curve"
reject_input "input.tablets that is not a table" "{tablets = 3}" "input.tablets must be a table"
reject_input "an unknown input group" "{repeat_delay = -5}" "unknown input group: repeat-delay"
check "the compositor keeps running after wrong-typed input" "$("$G" status 2>&1)" '"state":"running"'
disable_config 99-test-badreload.lua

echo "-- stability"
check "compositor alive" "$(pgrep -x gnoblin | wc -l)" "4"
coredump_after="$(coredumpctl list --no-pager 2>/dev/null | tail -1)"
if [ "$coredump_before" = "$coredump_after" ]; then
    echo "PASS no new coredump"
else
    echo "FAIL no new coredump (before: $coredump_before; after: $coredump_after)"
    fail=$((fail + 1))
fi

echo "RESULT failures=$fail"
