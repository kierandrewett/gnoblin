#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "!! starting the test system bus requires root" >&2
    exit 1
fi
command -v dbus-daemon >/dev/null

# Headless CI exercises compositor and window lifecycles without a logged-in
# seat. Some CI containers expose /run/systemd/seats even though logind cannot
# be activated (its D-Bus service exits with /bin/false). In that test mode,
# hide the marker so Shell selects GNOME's supported dummy login manager.
if [[ "${GNOBLIN_TEST_NO_LOGIND:-0}" == 1 && -e /run/systemd/seats ]]; then
    echo "INFO: clearing /run/systemd/seats for headless compositor tests"
    rm -rf -- /run/systemd/seats
fi

socket=/run/dbus/system_bus_socket
if [[ ! -S "$socket" ]]; then
    install -d -m 0755 /run/dbus
    dbus-daemon --system --fork --nopidfile
fi

for _ in {1..50}; do
    if [[ -S "$socket" ]]; then
        echo "PASS: private test system bus is ready"
        exit 0
    fi
    sleep 0.1
done

echo "!! system D-Bus did not create $socket" >&2
exit 1
