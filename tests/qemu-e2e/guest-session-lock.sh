#!/usr/bin/env bash
# Guest: the compositor enforces the session lock (ext-session-lock-v1).
#
# swaylock (a standard locker) takes the lock. While it holds it, the compositor must:
#   - report lock_state "locked" in gnoblinctl status;
#   - not leak the desktop through screen capture: a wlr-screencopy request (grim) must be refused or return only the
#     lock scene, never the desktop;
#   - stop normal input: a desktop shortcut pressed through RemoteDesktop must not fire.
# If the locker is killed without unlocking, the compositor must keep the lock in its documented failsafe state and
# keep refusing capture. A new locker must then be able to take the lock and unlock it (swaylock unlocks on SIGUSR1),
# after which the shortcut fires and capture shows the desktop again.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-session-lock.lua"
fail=0
trap 'pkill -9 -x swaylock 2>/dev/null; rm -f "$F" /tmp/lock-probe /tmp/lock-shot.png; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

python3 -c "import PIL" 2>/dev/null || sudo dnf install -y python3-pillow >/dev/null 2>&1

cat > "$F" <<'LUA'
gnoblin.configure {
    keybindings = {keyboard = {
        lock_probe = {binding = "<Super>F9", callback = function()
            gnoblin.commands.run({"sh", "-c", "echo x >> /tmp/lock-probe"})
        end},
    }},
}
LUA
"$G" config reload >/dev/null 2>&1
sleep 3

cat > /tmp/press-keys.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

KEYS = {"super": 125, "f9": 67}
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

# Prints how many of four sample points, two on each monitor, show the locker colour (51, 102, 153).
cat > /tmp/lock-pixels.py <<'PY'
from PIL import Image

im = Image.open("/tmp/lock-shot.png").convert("RGB")
width, height = im.size
points = [(100, 100), (width // 2 - 100, height // 2), (width // 2 + 100, height // 2), (width - 100, height - 100)]
hits = sum(1 for p in points if max(abs(a - b) for a, b in zip(im.getpixel(p), (51, 102, 153))) <= 6)
print(hits)
PY

lock_state() { "$G" status 2>&1 | head -1 | grep -o '"lock_state":"[a-z-]*"'; }
shot_hits() { grim /tmp/lock-shot.png 2>/dev/null && python3 /tmp/lock-pixels.py || echo "no-shot"; }
# "refused" or "lock-scene" are safe. "desktop" means capture returned something that is not the lock scene.
capture_while_locked() {
    local hits
    hits="$(shot_hits)"
    case "$hits" in
        no-shot) echo refused ;;
        4) echo lock-scene ;;
        *) echo desktop ;;
    esac
}
capture_is_safe() { case "$(capture_while_locked)" in refused|lock-scene) echo safe ;; *) echo leak ;; esac; }
probe_count() { wc -l < /tmp/lock-probe 2>/dev/null || echo 0; }

# A desktop window, so there is something to hide.
nohup foot -T lock-desktop >/dev/null 2>&1 < /dev/null &
sleep 3
rm -f /tmp/lock-probe

check "the session starts unlocked" "$(lock_state)" '"lock_state":"unlocked"'
check "no sample point shows the locker colour before the lock" "$(shot_hits)" "0"
python3 /tmp/press-keys.py super+f9; sleep 1
check "the desktop shortcut fires while unlocked" "$(probe_count)" "1"

swaylock -f -c 336699 >/dev/null 2>&1
sleep 3
check "the compositor reports the session locked" "$(lock_state)" '"lock_state":"locked"'
check "screen capture does not leak the desktop while locked" "$(capture_is_safe)" "safe"
python3 /tmp/press-keys.py super+f9; sleep 1
check "the desktop shortcut does not fire while locked" "$(probe_count)" "1"

pkill -9 -x swaylock
sleep 3
check "the session keeps the lock in the failsafe state after the locker is killed" "$(lock_state)" '"lock_state":"failsafe"'
check "screen capture still does not leak the desktop in the failsafe state" "$(capture_is_safe)" "safe"
echo "info capture while locked: $(capture_while_locked)"

swaylock -f -c 336699 >/dev/null 2>&1
sleep 3
check "a new locker can take over the lock" "$(lock_state)" '"lock_state":"locked"'
pkill -USR1 -x swaylock
sleep 3
check "SIGUSR1 unlocks the session" "$(lock_state)" '"lock_state":"unlocked"'
check "the desktop is visible again" "$(shot_hits)" "0"
python3 /tmp/press-keys.py super+f9; sleep 1
check "the desktop shortcut fires again after the unlock" "$(probe_count)" "2"
check "the compositor keeps running" "$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')" '"state":"running"'

pkill -f "foot -T lock-desktop"
echo "failures: $fail"
exit "$fail"
