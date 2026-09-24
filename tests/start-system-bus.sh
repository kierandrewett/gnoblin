#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "!! starting the test system bus requires root" >&2
    exit 1
fi
command -v dbus-daemon >/dev/null

# GitHub-hosted container jobs can inherit /run/systemd/seats from the host
# without running systemd as PID 1 in the job container. GNOME Shell treats
# that marker as proof that logind is available, then system-bus activation of
# org.freedesktop.login1 fails because its real systemd service cannot run in
# this container. Remove only this stale marker in a non-systemd container so
# Shell selects GNOME's supported dummy login manager for the headless test.
pid1_name="$(cat /proc/1/comm 2>/dev/null || true)"
if [[ "$pid1_name" != systemd && -e /run/systemd/seats ]]; then
    echo "INFO: PID 1 is '$pid1_name'; clearing stale /run/systemd/seats test marker"
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
