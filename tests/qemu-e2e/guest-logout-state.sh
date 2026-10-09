#!/usr/bin/env bash
# Guest: report core dump count, shutdown errors since the last logout request, and compositor state.
set -u
since="$(cat /tmp/logout-mark 2>/dev/null || date -d '-1 min' '+%Y-%m-%d %H:%M:%S')"
echo "cores=$(coredumpctl list --no-pager 2>/dev/null | grep -c -E 'gnoblin|gnome-shell' || true)"
bad="$(sudo journalctl -b --no-pager --since "$since" 2>/dev/null |
    grep -v 'pkla-check-authorization' |
    grep -c -E 'double free|corruption|SIGABRT|Aborted|Gjs-CRITICAL|libmutter-CRITICAL|GLib-GObject-CRITICAL' || true)"
# pkla-check-authorization is a polkit helper that logs its own GLib criticals; it is not the compositor.
echo "journal-bad=$bad"
if pgrep -f "${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX}/bin/gnoblin --wayland" >/dev/null; then
    echo "compositor=running"
else
    echo "compositor=absent"
fi
