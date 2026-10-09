#!/usr/bin/env bash
# Guest: helper for run-login-leak.sh. Prints the number of leftover D-Bus-activated service processes.
#
# GNOME's session manager ends the accessibility registry (at-spi2-registryd --use-gnome-session) at logout. Gnoblin has no
# session manager, so the runtime stops the registry unit at the next login and at exit. Without that, every login would
# leave one more process in the user manager.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
echo "registry_processes=$(pgrep -c -f at-spi2-registryd)"
echo "registry_units=$(systemctl --user list-units --all --plain --no-legend 'dbus-*-org.a11y.atspi.Registry@*.service' | wc -l)"
