#!/usr/bin/env bash
# Install and remove a package only in a disposable container with stock GNOME.
set -euo pipefail
export PATH="$PATH"
if [ "$(id -u)" -ne 0 ] || { [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; }; then
    echo 'Run this test as root in a disposable Debian/Ubuntu container.' >&2
    exit 2
fi
package="$(realpath -- "${1:?usage: test-deb.sh PACKAGE.deb}")"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends gnome-shell foot dbus-x11 xauth wayland-utils procps build-essential pkg-config libwayland-dev
stock_checksum="$(sha256sum /usr/bin/gnome-shell)"
stock_version="$(/usr/bin/gnome-shell --version)"
apt-get install -y --no-install-recommends "$package"
printf '%s\n' "$stock_checksum" | sha256sum --check
id tester >/dev/null 2>&1 || useradd -m tester
install -d -m700 -o tester -g tester /tmp/gnoblin-package-runtime
mkdir -p /run/dbus
test -S /run/dbus/system_bus_socket || dbus-daemon --system --fork
# Package tmpfiles create this empty directory even without a running logind.
# Let GNOME select its supported no-logind backend for this headless test.
if [ "$(cat /proc/1/comm)" != systemd ] && [ -d /run/systemd/seats ]; then
    rmdir /run/systemd/seats
fi
runuser -u tester -- env XDG_RUNTIME_DIR=/tmp/gnoblin-package-runtime GNOBLIN_PREFIX=/usr/lib/gnoblin \
    GNOBLIN_TEST_DBUS_CLIENT="$root/tests/deb-runtime.py" \
    bash "$root/scripts/run-gnome-shell.sh"
runuser -u tester -- env XDG_RUNTIME_DIR=/tmp/gnoblin-package-runtime GNOBLIN_PREFIX=/usr/lib/gnoblin GNOBLIN_TEST_MODE=user \
    bash "$root/scripts/run-gnome-shell.sh"
apt-get remove -y gnoblin
printf '%s\n' "$stock_checksum" | sha256sum --check
test "$(/usr/bin/gnome-shell --version)" = "$stock_version"
test ! -e /usr/share/wayland-sessions/gnoblin.desktop
test ! -e /usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
printf '%s\n' 'PASS: package install, runtime, GNOME isolation and removal'
