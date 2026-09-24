#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "!! starting the test system bus requires root" >&2
    exit 1
fi
command -v dbus-daemon >/dev/null

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
