#!/usr/bin/env bash
# Guest: apps follow dark mode and high contrast live, through the desktop portal.
#
# GTK and Qt apps read org.freedesktop.appearance from the portal. The test changes the GSettings keys that GNOME Settings
# changes, checks the value the portal reports, and checks that the portal emits SettingChanged for each. The original values
# are restored afterwards.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export DBUS_SESSION_BUS_ADDRESS
fail=0
orig_scheme="$(gsettings get org.gnome.desktop.interface color-scheme)"
orig_contrast="$(gsettings get org.gnome.desktop.a11y.interface high-contrast)"
trap 'gsettings set org.gnome.desktop.interface color-scheme "$orig_scheme"; gsettings set org.gnome.desktop.a11y.interface high-contrast "$orig_contrast"; rm -f /tmp/appearance-signals.txt' EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

read_setting() {
    busctl --user call org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.Settings \
        ReadOne ss org.freedesktop.appearance "$1" 2>&1 | head -1
}
wait_for() {
    # wait_for KEY VALUE: wait up to 5 seconds for the portal to report the value
    for _ in $(seq 1 20); do
        if [ "$(read_setting "$1")" = "v u $2" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}

gsettings set org.gnome.desktop.interface color-scheme default
gsettings set org.gnome.desktop.a11y.interface high-contrast false
wait_for color-scheme 0
wait_for contrast 0
check "the portal reports the default colour scheme" "$(read_setting color-scheme)" "v u 0"
check "the portal reports normal contrast" "$(read_setting contrast)" "v u 0"

timeout 25 busctl --user monitor --match "type='signal',interface='org.freedesktop.portal.Settings',member='SettingChanged'" \
    >/tmp/appearance-signals.txt 2>&1 &
sleep 1
gsettings set org.gnome.desktop.interface color-scheme prefer-dark
check "the portal reports dark mode after it is switched on" "$(wait_for color-scheme 1 && echo 'v u 1')" "v u 1"
gsettings set org.gnome.desktop.interface color-scheme prefer-light
check "the portal reports light mode after prefer-light" "$(wait_for color-scheme 2 && echo 'v u 2')" "v u 2"
gsettings set org.gnome.desktop.a11y.interface high-contrast true
check "the portal reports more contrast when high contrast is on" "$(wait_for contrast 1 && echo 'v u 1')" "v u 1"
sleep 2
check "the portal emitted SettingChanged for color-scheme" "$([ "$(grep -c 'STRING "color-scheme"' /tmp/appearance-signals.txt)" -ge 2 ] && echo yes)" "yes"
check "the portal emitted SettingChanged for contrast" "$([ "$(grep -c 'STRING "contrast"' /tmp/appearance-signals.txt)" -ge 1 ] && echo yes)" "yes"

echo "failures: $fail"
exit "$fail"
