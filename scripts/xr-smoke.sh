#!/usr/bin/env bash
# Whole-stack XR smoke: the real compositor, the stock Lua shell, two Wayland clients, the gnoblin-xr extension and
# Monado (simulated headset, null compositor, lavapipe) together, with nothing faked. Prints PASS or FAIL for each
# assertion and exits non-zero when any fails. See docs/xr.md, "Testing without a headset".
#
# It is made for an Agent37 instance (Debian 12, no GPU, no seat, no headset), run as root. It is idempotent: each run
# stops what an earlier run left, starts everything afresh in its own runtime directory and removes it at the end.
#
#   scripts/xr-smoke.sh            run the smoke (builds nothing; installs and builds Monado once if it is missing)
#   scripts/xr-smoke.sh setup      only install the packages and build Monado
#
# Environment (all optional):
#   TARGET_DIR   where cargo put the binaries: gnoblin, gnoblin-xr, xr-smoke-client, xr-smoke-tree
#                (default /home/node/target/debug)
#   REPO         the repository checkout, for the stock Lua shell (default: this script's repository)
#   MONADO_DIR   the Monado checkout with its build/ directory (default /opt/monado-src/monado)
#   KEEP=1       leave the runtime directory and logs in place at the end (default: logs are kept in LOG_DIR, the
#                runtime directory is removed)
#   LOG_DIR      where the logs and dumps go (default /tmp/xr-smoke)
#   XR_SMOKE_RECORD=FILE
#                also record the owner's tree stream (the attach reply, the snapshot and every change batch) to FILE,
#                from before the extension starts until the run ends. extensions/gnoblin-xr/crates/xr-tree/tests/recorded/
#                holds such a recording, which the replay tests read; see docs/xr.md, "Recorded trees".
#
# Build on the instance first:
#   cargo build -p gnoblin
#   cargo build --manifest-path extensions/gnoblin-xr/Cargo.toml --bins
set -u

REPO="${REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
TARGET_DIR="${TARGET_DIR:-/home/node/target/debug}"
MONADO_DIR="${MONADO_DIR:-/opt/monado-src/monado}"
LOG_DIR="${LOG_DIR:-/tmp/xr-smoke}"
RUN=/run/xr-smoke
SERVICE="$MONADO_DIR/build/src/xrt/targets/service/monado-service"
ICD=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json

RED=ff0000
BLUE=0000ff

failures=0
checks=0

pass() {
    printf 'PASS  %s\n' "$1"
    checks=$((checks + 1))
}
fail() {
    printf 'FAIL  %s\n' "$1"
    [ -n "${2:-}" ] && printf '      %s\n' "$2"
    checks=$((checks + 1))
    failures=$((failures + 1))
}

# check NAME COMMAND...: PASS when the command succeeds.
check() {
    local name="$1"
    shift
    if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name"; fi
}

# wait_for SECONDS COMMAND...: true as soon as the command succeeds, false after SECONDS.
wait_for() {
    local limit="$1"
    shift
    local deadline=$((SECONDS + limit))
    while ! "$@" >/dev/null 2>&1; do
        [ "$SECONDS" -ge "$deadline" ] && return 1
        sleep 0.2
    done
}

in_file() { grep -q -- "$1" "$2" 2>/dev/null; }

setup() {
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq mesa-vulkan-drivers vulkan-tools libvulkan-dev glslang-tools ninja-build cmake \
        pkg-config libeigen3-dev libudev-dev libusb-1.0-0-dev libx11-dev libxcb1-dev libwayland-dev libxrandr-dev \
        libxcb-randr0-dev libopenxr-dev libhidapi-dev libsystemd-dev libcjson-dev libbsd-dev libglvnd-dev libgl-dev \
        libegl-dev libgles-dev wayland-protocols libdrm-dev libgbm-dev libsdl2-dev libv4l-dev libjpeg-dev \
        libopencv-dev git >/dev/null || return 1
    if [ ! -x "$SERVICE" ]; then
        mkdir -p "$(dirname "$MONADO_DIR")"
        [ -d "$MONADO_DIR" ] || git clone -q --depth 1 https://gitlab.freedesktop.org/monado/monado.git "$MONADO_DIR" || return 1
        cmake -S "$MONADO_DIR" -B "$MONADO_DIR/build" -G Ninja -DXRT_FEATURE_SERVICE=ON -DXRT_BUILD_DRIVER_SIMULATED=ON \
            -DXRT_FEATURE_OPENXR=ON -DXRT_FEATURE_SLAM=OFF -DXRT_FEATURE_DEBUG_GUI=OFF -DXRT_FEATURE_WINDOW_PEEK=OFF \
            -DXRT_FEATURE_STEAMVR_PLUGIN=OFF -DXRT_HAVE_OPENCV=OFF -DBUILD_TESTING=OFF >/dev/null || return 1
        ninja -C "$MONADO_DIR/build" -j2 >/dev/null || return 1
    fi
}

if [ "${1:-}" = setup ]; then
    setup
    exit $?
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "xr-smoke: run as root on the Agent37 instance" >&2
    exit 2
fi
if [ ! -x "$SERVICE" ] || [ ! -f "$ICD" ]; then
    echo "xr-smoke: Monado or lavapipe is missing; running setup" >&2
    setup || {
        echo "xr-smoke: setup failed" >&2
        exit 2
    }
fi
for binary in gnoblin gnoblin-xr xr-smoke-client xr-smoke-tree; do
    [ -x "$TARGET_DIR/$binary" ] || {
        echo "xr-smoke: $TARGET_DIR/$binary is missing; build it first" >&2
        exit 2
    }
done

# ---- A fresh stage -----------------------------------------------------------------------------------------------

# Stop what an earlier run left. Matching on the exact program names keeps this to our own processes.
stop_all() {
    # The recorder's retry loop first, or it would start the recorder again over the recording.
    [ -n "${RECORD_LOOP:-}" ] && kill "$RECORD_LOOP" 2>/dev/null
    for name in xr-smoke-client xr-smoke-tree gnoblin-xr monado-service; do
        pkill -x "$name" 2>/dev/null
    done
    pkill -x gnoblin 2>/dev/null
    sleep 0.5
    for name in xr-smoke-client xr-smoke-tree gnoblin-xr monado-service gnoblin; do
        pkill -9 -x "$name" 2>/dev/null
    done
    pkill -f "dbus-daemon --session --nofork --nopidfile --address=unix:path=$RUN/bus" 2>/dev/null
    return 0
}
stop_all
rm -rf "$RUN" "$LOG_DIR"
mkdir -p "$RUN" "$LOG_DIR/dump" "$LOG_DIR/home"
chmod 700 "$RUN"

cleanup() {
    stop_all
    [ "${KEEP:-0}" = 1 ] || rm -rf "$RUN"
}
trap cleanup EXIT

export XDG_RUNTIME_DIR="$RUN"
export XDG_STATE_HOME="$LOG_DIR/home/state"
export XDG_CONFIG_HOME="$LOG_DIR/home/config"
export HOME="$LOG_DIR/home"
export XR_RUNTIME_JSON="$MONADO_DIR/build/openxr_monado-dev.json"
export VK_ICD_FILENAMES="$ICD"

# ---- A session bus: the guardian starts the portal backend, which needs one before it reports ready ------------------

# A private bus at the path the portal derives from XDG_RUNTIME_DIR. Without it the portal restarts five times, the
# guardian never reports the session ready and never starts the extension.
export DBUS_SESSION_BUS_ADDRESS="unix:path=$RUN/bus"
setsid dbus-daemon --session --nofork --nopidfile --address="$DBUS_SESSION_BUS_ADDRESS" >"$LOG_DIR/dbus.log" 2>&1 &
wait_for 10 test -S "$RUN/bus" || {
    echo "xr-smoke: the session bus did not start (see $LOG_DIR/dbus.log)" >&2
    exit 2
}

# ---- Monado: the simulated headset, the null compositor, lavapipe ------------------------------------------------

# The service needs a pipe on stdin that stays open (with /dev/null it fails at epoll_ctl).
# No simulated hands by default, so the extension's pointing ray is the gaze: the head's forward axis. XR_SMOKE_HANDS=1
# adds a simulated simple controller on each hand, whose aim pose points wherever the driver puts it.
HANDS=""
[ "${XR_SMOKE_HANDS:-0}" = 1 ] && HANDS="SIMULATED_LEFT=simple SIMULATED_RIGHT=simple"
setsid bash -c "sleep 3000 | SIMULATED_ENABLE=1 $HANDS XRT_COMPOSITOR_NULL=1 \
    XRT_COMPOSITOR_LOG=debug XRT_LOG=debug exec -a monado-service '$SERVICE'" >"$LOG_DIR/monado.log" 2>&1 &
wait_for 20 test -S "$RUN/monado_comp_ipc"
check "Monado service is listening" test -S "$RUN/monado_comp_ipc"

# ---- Gnoblin: guardian, compositor (headless, software renderer), runtime (stock Lua shell), xr extension ----------

export GNOBLIN_XR=1
export GNOBLIN_XR_PROGRAM="$TARGET_DIR/gnoblin-xr"
export GNOBLIN_XR_DUMP_DIR="$LOG_DIR/dump"
export GNOBLIN_SHELL_VM="$REPO/data/lua/gnoblin"
export GNOBLIN_LOG="xr=debug,info"
export GNOBLIN_IPC_SOCKET="$RUN/gnoblin/ipc.sock"
export WAYLAND_DISPLAY="$RUN/gnoblin-wayland.sock"

SESSION="$LOG_DIR/session.log"
setsid "$TARGET_DIR/gnoblin" >"$SESSION" 2>&1 &
if [ -n "${XR_SMOKE_RECORD:-}" ]; then
    # Attach as soon as the owner socket takes a client, so the recording holds the room being built. A failed attach
    # (the socket is not serving yet) is tried again; a recorder that attached runs until the run ends.
    setsid bash -c "until '$TARGET_DIR/xr-smoke-tree' --settle-ms 600000 --record '$XR_SMOKE_RECORD' >/dev/null 2>'$LOG_DIR/record.err'; do sleep 0.1; done" &
    RECORD_LOOP=$!
fi
wait_for 60 in_file "session ready" "$SESSION"
check "the guardian reports the session ready (compositor, services, runtime)" in_file "session ready" "$SESSION"
check "the compositor serves a headless output" in_file 'backend="headless"' "$SESSION"
check "the stock shell attached to the tree (the runtime committed its configuration)" \
    in_file "configuration revision 1 accepted" "$SESSION"
check "the guardian started the xr extension" in_file "started xr (generation 1" "$SESSION"

tree() { "$TARGET_DIR/xr-smoke-tree" --settle-ms "${1:-600}" 2>"$LOG_DIR/tree.err"; }

# ---- (a) the xr seat exists -------------------------------------------------------------------------------------

tree_has() { tree 300 | grep -q -- "$1"; }
wait_for 60 tree_has "^seats: .*xr"
if tree 300 | grep -q "^seats: .*xr"; then
    pass "(a) the xr seat exists in the owner's tree snapshot"
else
    fail "(a) the xr seat exists in the owner's tree snapshot" "$(tree 300 | head -3 | tr '\n' ' ')"
fi
check "(a) the xr extension stayed up (no 'refused' and no restart)" \
    bash -c "! grep -q 'restarting xr' '$SESSION' && ! grep -q 'refused tree attach' '$SESSION'"

# ---- Two Wayland clients, one solid colour each -----------------------------------------------------------------

"$TARGET_DIR/xr-smoke-client" --app-id smoke-red --colour "$RED" --size 400x300 >"$LOG_DIR/client-red.log" 2>&1 &
RED_PID=$!
wait_for 20 in_file "^mapped " "$LOG_DIR/client-red.log"
"$TARGET_DIR/xr-smoke-client" --app-id smoke-blue --colour "$BLUE" --size 400x300 >"$LOG_DIR/client-blue.log" 2>&1 &
BLUE_PID=$!
wait_for 20 in_file "^mapped " "$LOG_DIR/client-blue.log"
check "two Wayland clients mapped on the compositor" bash -c \
    "grep -q '^mapped ' '$LOG_DIR/client-red.log' && grep -q '^mapped ' '$LOG_DIR/client-blue.log'"

# ---- (b) the shell built the room ---------------------------------------------------------------------------------

views_are() { tree 300 | grep -q "^views: $1\$"; }
wait_for 30 views_are 2
TREE_OUT="$(tree 300)"
printf '%s\n' "$TREE_OUT" >"$LOG_DIR/tree.txt"
check "(b) the tree has a node named room" grep -q '^room: present' "$LOG_DIR/tree.txt"
check "(b) the room holds a camera named head with a stereo projection" grep -q '^head: camera stereo' "$LOG_DIR/tree.txt"
check "(b) the room holds one View per client window (2)" grep -q '^views: 2$' "$LOG_DIR/tree.txt"

XR_LOG="$LOG_DIR/xr.log"
grep 'gnoblin-xr' "$SESSION" >"$XR_LOG"
refresh_xr() { grep 'gnoblin-xr' "$SESSION" >"$XR_LOG"; }

# ---- (c) the extension planned and submitted layers; Monado accepted frames ---------------------------------------

xr_has() {
    refresh_xr
    grep -q -- "$1" "$XR_LOG"
}
wait_for 30 xr_has "plan: 2 layers, 0 skipped"
check "(c) the extension planned layers=2, skipped=0" xr_has "plan: 2 layers, 0 skipped"
wait_for 30 xr_has "submitted 2 layers"
check "(c) the extension ended a frame with 2 submitted layers" xr_has "submitted 2 layers"
frames_with_two() {
    refresh_xr
    [ "$(grep -c 'debug: frame .*: planned 2, submitted 2' "$XR_LOG")" -ge 10 ]
}
wait_for 30 frames_with_two
check "(c) at least 10 frames submitted 2 layers each (debug log)" frames_with_two
check "(c) the extension logged no failure (swapchain, copy, capture, refused input)" \
    bash -c "! grep -E 'failed|refused|cannot|no graphics session|headless session|view capture disabled' '$XR_LOG'"
check "(c) Monado's null compositor ran with a session from the extension" \
    bash -c "grep -qiE 'null compositor|null_compositor|Null compositor' '$LOG_DIR/monado.log'"
check "(c) Monado reported no error for the session" \
    bash -c "! grep -E '(^| )ERROR' '$LOG_DIR/monado.log'"

# ---- (d) pixel check -------------------------------------------------------------------------------------------

# The centre pixel of a binary PPM as RRGGBB.
centre_pixel() {
    local file="$1" width height header offset
    read -r width height < <(sed -n '2p' "$file")
    header=$(printf 'P6\n%s %s\n255\n' "$width" "$height" | wc -c)
    offset=$((header + ((height / 2) * width + width / 2) * 3))
    od -An -tx1 -j "$offset" -N3 "$file" | tr -d ' \n'
}

dumps_exist() { [ "$(ls "$LOG_DIR"/dump/view-*-first.ppm 2>/dev/null | wc -l)" -ge 2 ]; }
wait_for 30 dumps_exist
check "(d) the extension dumped the first uploaded picture of each View" dumps_exist
colours=""
for file in "$LOG_DIR"/dump/view-*-first.ppm; do
    [ -f "$file" ] && colours="$colours $(centre_pixel "$file")"
done
# Gnoblin draws the window, so an unfocused one can be a little dimmer than the client's colour: allow 0x10 a channel.
near() {
    local a="$1" b="$2" i
    for i in 0 2 4; do
        local d=$((0x${a:$i:2} - 0x${b:$i:2}))
        [ "${d#-}" -le 16 ] || return 1
    done
}
set -- $colours 000000 000000
if { near "$1" "$RED" && near "$2" "$BLUE"; } || { near "$1" "$BLUE" && near "$2" "$RED"; }; then
    pass "(d) the centre pixel of the two Views is the clients' colours ($RED and $BLUE)"
else
    fail "(d) the centre pixel of the two Views is the clients' colours ($RED and $BLUE)" "got: $colours"
fi

# ---- (f) input: the gaze ray reaches the front panel -------------------------------------------------------------

# The simulated head looks straight ahead and the ring puts the focused panel front and centre, so the ray hits it. The
# window mapped last takes the focus, so it is the front panel and the gaze must enter it on the xr seat; the first
# client was in front until then, so it hears an enter too.
entered_by() { grep -q '^pointer-enter seat=xr' "$LOG_DIR/client-$1.log"; }
wait_for 30 entered_by blue
check "(f) the front client (the last window mapped, the focused one) got wl_pointer.enter on the xr seat from the gaze ray" \
    entered_by blue
for name in red blue; do
    if entered_by "$name"; then
        echo "      client $name got the xr pointer: $(grep -c '^pointer-enter seat=xr' "$LOG_DIR/client-$name.log") enter(s)"
    fi
done
# Every enter is at a point in the window's own 400 by 300 pixels.
enters_inside() {
    local log
    for log in "$LOG_DIR"/client-red.log "$LOG_DIR"/client-blue.log; do
        grep '^pointer-enter seat=xr' "$log" | sed -E 's/.* x=(-?[0-9.]+) y=(-?[0-9.]+)/\1 \2/' |
            awk 'NF != 2 || $1 < 0 || $1 > 400 || $2 < 0 || $2 > 300 { bad = 1 } END { exit bad }' || return 1
    done
}
check "(f) every xr enter is at a point inside its 400x300 window" enters_inside

# ---- (e) closing one client removes a layer ----------------------------------------------------------------------

kill "$BLUE_PID" 2>/dev/null
wait "$BLUE_PID" 2>/dev/null
views_is_one() { tree 300 | grep -q '^views: 1$'; }
wait_for 15 views_is_one
check "(e) closing a client removes its View from the room" views_is_one
# The last plan and the last submission the extension logged, not an earlier one from while the clients were mapping.
xr_one_layer() {
    refresh_xr
    [ "$(grep 'gnoblin-xr: plan:' "$XR_LOG" | tail -1 | grep -c 'plan: 1 layers')" = 1 ] &&
        [ "$(grep 'gnoblin-xr: frame .*submitted' "$XR_LOG" | tail -1 | grep -c 'submitted 1 layers')" = 1 ]
}
wait_for 15 xr_one_layer
check "(e) the extension planned and submitted 1 layer after the close" xr_one_layer

# ---- The extension goes: its seat and the room go with it --------------------------------------------------------

kill "$RED_PID" 2>/dev/null
wait "$RED_PID" 2>/dev/null
XR_PID="$(pgrep -x gnoblin-xr | head -1)"
[ -n "$XR_PID" ] && kill -KILL "$XR_PID" 2>/dev/null
no_xr_seat() { ! tree 300 | grep -q '^seats: .*xr'; }
wait_for 15 no_xr_seat
check "the xr seat is destroyed when the extension's connection closes" no_xr_seat
no_room() { tree 300 | grep -q '^room: absent'; }
wait_for 15 no_room
check "the stock shell removes the room when the seat goes" no_room

echo
if [ "$failures" -eq 0 ]; then
    echo "xr-smoke: $checks checks, all PASS (logs in $LOG_DIR)"
    exit 0
fi
echo "xr-smoke: $failures of $checks checks FAILED (logs in $LOG_DIR)"
exit 1
