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

# The previous client is stopped by the process group it was started in (recorded below), never by matching command
# lines: a pattern such as `/firefox` also matches this script's own arguments when the next client is Firefox.
client_group=$HOME/.gnoblin-ci-client.pgid
if [ -s "$client_group" ]; then
    kill -TERM -- "-$(cat "$client_group")" 2>/dev/null || true
    rm -f "$client_group"
fi
pkill -x gnoblin 2>/dev/null || true
bus_pid=$HOME/.gnoblin-ci-bus.pid
if [ -s "$bus_pid" ]; then
    kill -TERM "$(cat "$bus_pid")" 2>/dev/null || true
    rm -f "$bus_pid"
fi
sleep 1
sudo systemctl stop getty@tty1 2>/dev/null || true
rm -rf "$logs"
mkdir -p "$logs"

XDG_RUNTIME_DIR=/run/user/$(id -u)
export XDG_RUNTIME_DIR
export LIBSEAT_BACKEND=seatd SEATD_SOCK=/run/seatd.sock
export GNOBLIN_LOG="${GNOBLIN_LOG:-info}" GNOBLIN_LOG_FORMAT=plain
dbus-daemon --session --address="unix:path=$logs/bus" --fork --print-pid >"$bus_pid"
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
    # setsid makes the client the leader of a session and a process group of its own, whose id is its pid.
    WAYLAND_DISPLAY=gnoblin-wayland.sock DISPLAY=:0 setsid bash -c "$client" >"$logs/client.log" 2>&1 </dev/null &
    echo $! >"$client_group"
    echo "[guest] client started: $client"
fi
