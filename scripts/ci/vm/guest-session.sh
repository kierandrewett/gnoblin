#!/usr/bin/env bash
# Run inside the test guest: start the installed gnoblin on the DRM backend through seatd, then one client.
#
#   guest-session.sh SHELL [CLIENT_COMMAND]
#
# SHELL is the shell gnoblin loads (a path); CLIENT_COMMAND is one shell command line started on gnoblin's socket.
# Logs go to ~/session: gnoblin.log and client.log. gnoblin is started with setsid so it outlives the ssh session that
# ran this. The text console is stopped first, because the seat refuses gnoblin while another DRM master holds it.
set -euo pipefail
export PATH=/opt/gnoblin/bin:/usr/local/bin:/usr/bin:/bin
shell=${1:?usage: guest-session.sh SHELL [CLIENT_COMMAND]}
client=${2:-}
logs=$HOME/session

pkill -x gnoblin 2>/dev/null || true
pkill -f "^(foot|gtk4-widget-factory|gtk3-widget-factory|xterm|xeyes|wev)( |$)|/firefox" 2>/dev/null || true
pkill -f "dbus-daemon --session --address=unix:path=$logs/" 2>/dev/null || true
sleep 1
sudo systemctl stop getty@tty1 2>/dev/null || true
rm -rf "$logs"
mkdir -p "$logs"

XDG_RUNTIME_DIR=/run/user/$(id -u)
export XDG_RUNTIME_DIR
export LIBSEAT_BACKEND=seatd SEATD_SOCK=/run/seatd.sock
export GNOBLIN_LOG="${GNOBLIN_LOG:-info}" GNOBLIN_LOG_FORMAT=plain
dbus-daemon --session --address="unix:path=$logs/bus" --fork >/dev/null
export DBUS_SESSION_BUS_ADDRESS="unix:path=$logs/bus"
rm -f "$XDG_RUNTIME_DIR"/gnoblin-wayland*.sock*
GNOBLIN_BACKEND=drm setsid gnoblin "$shell" >"$logs/gnoblin.log" 2>&1 </dev/null &
started=$!
for _ in $(seq 1 120); do
    [ -S "$XDG_RUNTIME_DIR/gnoblin-wayland.sock" ] && break
    kill -0 "$started" 2>/dev/null || break
    sleep 0.5
done
if ! kill -0 "$started" 2>/dev/null; then
    echo "[guest] gnoblin exited during start" >&2
    tail -30 "$logs/gnoblin.log" >&2
    exit 1
fi
echo "[guest] gnoblin is serving"
if [ -n "$client" ]; then
    WAYLAND_DISPLAY=gnoblin-wayland.sock DISPLAY=:0 setsid bash -c "$client" >"$logs/client.log" 2>&1 </dev/null &
    echo "[guest] client started: $client"
fi
