#!/usr/bin/env bash
# Guest: compositor.audible_bell plays the bell sound. Counts new PipeWire sink-input streams while a
# GTK client rings the system bell ten times, with audible_bell off and then on.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-audible.lua"
fail=0

command -v pactl >/dev/null || { echo "FAIL pactl is not installed in the guest"; exit 1; }

cat > /tmp/beep-audible.py <<'PY'
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk

Gtk.init()
display = Gdk.Display.get_default()
loop = GLib.MainLoop()
rings = 0


def ring():
    global rings
    display.beep()
    rings += 1
    if rings >= 10:
        loop.quit()
        return False
    return True


GLib.timeout_add(250, ring)
loop.run()
PY

streams() {
    printf 'gnoblin.configure {compositor = {visual_bell = false, audible_bell = %s}}\n' "$1" > "$F"
    "$G" config reload >/dev/null
    sleep 3
    rm -f /tmp/pactl-events
    timeout 10 pactl subscribe > /tmp/pactl-events 2>&1 &
    subscriber=$!
    sleep 1
    python3 /tmp/beep-audible.py
    sleep 2
    kill "$subscriber" 2>/dev/null
    wait "$subscriber" 2>/dev/null
    grep -c "'new' on sink-input" /tmp/pactl-events
}

off="$(streams false)"
on="$(streams true)"
printf -- '-- disabled\n' > "$F"
"$G" config reload >/dev/null

if [ "$off" -eq 0 ]; then
    echo "PASS audible_bell=false plays no sound (new streams: $off)"
else
    echo "FAIL audible_bell=false still plays sound (new streams: $off)"
    fail=$((fail + 1))
fi
if [ "$on" -gt 0 ]; then
    echo "PASS audible_bell=true plays the bell sound (new streams: $on)"
else
    echo "FAIL audible_bell=true plays no sound (new streams: $on)"
    fail=$((fail + 1))
fi
echo "failures: $fail"
exit "$fail"
