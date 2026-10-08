#!/usr/bin/env bash
# Guest: Super+Space and Shift+Super+Space switch the input source, as in a GNOME session.
#
# The starter config binds both keys with a Lua callback. With two configured layouts, a real key press through
# RemoteDesktop must select the next source, wrap around at the end, and Shift+Super+Space must go back.
#
# The guest keeps its own copy of the starter files, made by gnoblinctl init, so it never reads the embedded ones. The
# host script copies the repository's 40-shortcuts.lua to /tmp/40-shortcuts.lua. This script installs it in place of the
# guest copy and puts the old file back at the end.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-input-source-shortcut.lua"
STARTER="$HOME/.config/gnoblin/config/40-shortcuts.lua"
SAVED="$HOME/.config/gnoblin/config/40-shortcuts.lua.input-source-test-saved"
fail=0

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat > /tmp/press-keys.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

KEYS = {"shift": 42, "super": 125, "space": 57}
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
names = sys.argv[1].split("+")
for name in names:
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEYS[name], True)))
    time.sleep(0.1)
time.sleep(0.2)
for name in reversed(names):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEYS[name], False)))
    time.sleep(0.1)
time.sleep(0.5)
call(session, iface, "Stop", None)
PY

current() { "$G" input current 2>&1 | grep -o '"id":"[^"]*"' | head -1; }

cp -p "$STARTER" "$SAVED"
cp /tmp/40-shortcuts.lua "$STARTER"
printf 'gnoblin.configure {input_sources = {sources = {{type = "xkb", id = "us"}, {type = "xkb", id = "gb"}}}}\n' > "$F"
"$G" config reload >/dev/null 2>&1
sleep 3

first="$(current)"
check "the first configured source is current" "$first" '"id":"us"'
python3 /tmp/press-keys.py super+space; sleep 2
check "Super+Space selects the next source" "$(current)" '"id":"gb"'
python3 /tmp/press-keys.py super+space; sleep 2
check "Super+Space wraps to the first source" "$(current)" '"id":"us"'
python3 /tmp/press-keys.py shift+super+space; sleep 2
check "Shift+Super+Space wraps back to the last source" "$(current)" '"id":"gb"'
python3 /tmp/press-keys.py shift+super+space; sleep 2
check "Shift+Super+Space selects the previous source" "$(current)" '"id":"us"'

# With one source there is nothing to switch, and the key must not break anything.
printf 'gnoblin.configure {input_sources = {sources = {{type = "xkb", id = "us"}}}}\n' > "$F"
"$G" config reload >/dev/null 2>&1
sleep 3
python3 /tmp/press-keys.py super+space; sleep 2
check "Super+Space with one source changes nothing" "$(current)" '"id":"us"'
check "the compositor keeps running" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

rm -f "$F"
mv -f "$SAVED" "$STARTER"
"$G" config reload >/dev/null 2>&1
echo "failures: $fail"
exit "$fail"
