#!/usr/bin/env bash
# Guest: the compositor's IBus input method gives Wayland-only apps composition.
#
# A GTK4 entry on the Wayland backend, with no GTK_IM_MODULE override, so it uses text-input-v3. With ibus-daemon
# stopped, typing "privet" gives Latin text: the input method passes keys through. With ibus-daemon running and
# the m17n Russian transliteration engine selected, the same keys give the Cyrillic word.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
unset GTK_IM_MODULE QT_IM_MODULE XMODIFIERS
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CLICK="${GNOBLIN_CLICK_SCRIPT:-/tmp/guest-click.py}"
fail=0

check() {
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: [$2], expected: [$3])"
        fail=$((fail + 1))
    fi
}

type_privet() {
    rm -f /tmp/ime-entry.txt
    GDK_BACKEND=wayland nohup python3 /tmp/guest-ime-entry-app.py >/dev/null 2>&1 < /dev/null &
    sleep 5
    # A window started from ssh does not begin focused, so click into the entry first.
    read -r cx cy <<<"$("$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "ime-test":
        f = w["frame"]
        print(f["x"] + f["width"] // 2, f["y"] + 70)')"
    python3 "$CLICK" "$cx" "$cy" >/dev/null 2>&1
    sleep 2
    python3 /tmp/guest-type-keys.py p r i v e t
    sleep 4
    cat /tmp/ime-entry.txt 2>/dev/null
    pkill -f guest-ime-entry-app.py
    sleep 1
}

pkill -x ibus-daemon
sleep 2
check "no ibus-daemon: the keys pass through as Latin text" "$(type_privet)" "privet"

nohup ibus-daemon -d -r -x >/tmp/ibus-daemon.log 2>&1 < /dev/null
sleep 4
ibus engine m17n:ru:translit >/dev/null 2>&1
sleep 2
check "the Russian transliteration engine is active" "$(ibus engine 2>&1 | head -1)" "m17n:ru:translit"
check "ibus-daemon running: privet becomes Cyrillic text" "$(type_privet)" "привет"

pkill -x ibus-daemon
echo "failures: $fail"
exit "$fail"
