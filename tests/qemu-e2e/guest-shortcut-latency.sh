#!/usr/bin/env bash
# Guest: a single tap of a media key reaches its command as fast as a plain shortcut.
#
# GitHub #106 reported delayed single-tap media shortcuts. This overrides the starter volume_up binding and a plain
# Super+F9 binding with callbacks that start a command which records the time. Each key is tapped five times through
# RemoteDesktop. Every tap must fire, and the time from the key event to the running command must stay under a generous
# bound, so that QEMU jitter does not fail the test but a delay of the size the issue describes does.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
F="$HOME/.config/gnoblin/config/99-test-shortcut-latency.lua"
BOUND_MS=150
fail=0
trap 'rm -f "$F" /tmp/latency.log /tmp/latency.sent; "$G" config reload >/dev/null 2>&1' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

cat > "$F" <<'LUA'
local function stamp(name)
    return function()
        gnoblin.commands.run({"sh", "-c", "echo " .. name .. " $(date +%s%N) >> /tmp/latency.log"})
    end
end
gnoblin.configure {
    keybindings = {keyboard = {
        volume_up = {binding = "XF86AudioRaiseVolume", callback = stamp("media")},
        latency_plain = {binding = "<Super>F9", callback = stamp("plain")},
    }},
}
LUA
"$G" config reload >/dev/null 2>&1
sleep 3

cat > /tmp/press-timed.py <<'PY'
import sys
import time

from gi.repository import Gio, GLib

CODES = {"media": [115], "plain": [125, 67]}
name = sys.argv[1]
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
started = time.time_ns()
for code in CODES[name]:
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, True)))
for code in reversed(CODES[name]):
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (code, False)))
print(name, started)
time.sleep(0.5)
call(session, iface, "Stop", None)
PY

rm -f /tmp/latency.log /tmp/latency.sent
for i in 1 2 3 4 5; do
    for kind in media plain; do
        python3 /tmp/press-timed.py "$kind" >> /tmp/latency.sent
        sleep 1.5
    done
done

report="$(python3 - "$BOUND_MS" <<'PY'
import collections
import sys

bound = int(sys.argv[1])
sent = collections.defaultdict(list)
for line in open("/tmp/latency.sent"):
    name, stamp = line.split()
    sent[name].append(int(stamp))
fired = collections.defaultdict(list)
try:
    for line in open("/tmp/latency.log"):
        name, stamp = line.split()
        fired[name].append(int(stamp))
except FileNotFoundError:
    pass
for name in ("media", "plain"):
    ms = [round((b - a) / 1e6) for a, b in zip(sent[name], fired[name])]
    print("%s_fired=%d" % (name, len(fired[name])))
    print("%s_ms=%s" % (name, ",".join(map(str, ms))))
    print("%s_within=%s" % (name, "yes" if ms and len(ms) == 5 and max(ms) <= bound else "no"))
PY
)"
echo "$report"
check "all five media key taps fire" "$(echo "$report" | grep -o '^media_fired=[0-9]*')" "media_fired=5"
check "all five plain key taps fire" "$(echo "$report" | grep -o '^plain_fired=[0-9]*')" "plain_fired=5"
check "every media key tap reaches its command within ${BOUND_MS} ms" "$(echo "$report" | grep -o '^media_within=[a-z]*')" "media_within=yes"
check "every plain key tap reaches its command within ${BOUND_MS} ms" "$(echo "$report" | grep -o '^plain_within=[a-z]*')" "plain_within=yes"
echo "failures: $fail"
exit "$fail"
