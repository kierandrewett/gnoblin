#!/usr/bin/env bash
# The Vulkan renderer on the DRM backend: `compositor.renderer = "vulkan"`, a CPU Vulkan driver (lavapipe) in the guest, the
# card's LINEAR scan-out buffers written through a mapping.
#
#   scripts/ci/vm/vulkan-check.sh QMP_SOCKET SHELL_PATH OUTDIR
#
# QMP_SOCKET  the QEMU QMP socket of the guest, whose display is a 2D virtio-vga, which QEMU can capture
# SHELL_PATH  the shell library gnoblin loads, as a path inside the guest
# OUTDIR      where the logs, captures and results go (vulkan-results.txt: `check|pass|detail` per line)
# GUEST_CMD   (environment) the words that run one shell command line in the guest, such as `ssh -p 2242 user@127.0.0.1`
# GUEST_SESSION (environment) the guest's copy of guest-session.sh, `./guest-session.sh` by default
# CI_VM_MODE  (environment) `software` (default) takes the QMP captures; `gpu` has none, so the picture checks are skipped
#
# A foot window of one colour makes the picture exact: the share of pixels of that colour is what was drawn, with the desktop
# around it, so a renderer that drew nothing, drew black or scanned out a stale buffer fails.
#
#   vulkan-foot            a session on the Vulkan renderer serves, says which device it draws on, and foot runs
#   vulkan-foot-pixels     the QMP screendump of the card shows the red window and the desktop
#   vkms-vulkan            the same on the kernel's `vkms` display (GNOBLIN_DRM_DEVICE), which a client keeps redrawing
#   vkms-flips             120 frames completed on it
#   vkms-screenshot        a grim screenshot on the privileged socket, which only a Vulkan output can answer, shows the window
#
# A kernel without the `vkms` module skips the three vkms checks, with the reason in the result. Exit status is 1 when a check
# failed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
qmp=${1:?usage: vulkan-check.sh QMP_SOCKET SHELL_PATH OUTDIR}
shell_path=${2:?SHELL_PATH}
out=$(mkdir -p "${3:?OUTDIR}" && cd "$3" && pwd)
session=${GUEST_SESSION:-./guest-session.sh}
mode=${CI_VM_MODE:-software}
: "${GUEST_CMD:?GUEST_CMD names how a command reaches the guest}"
results=$out/vulkan-results.txt
: >"$results"
failed=0

guest() { $GUEST_CMD "$@"; }
# check NAME pass|fail DETAIL
check() {
    echo "$1|$2|$3" >>"$results"
    echo "[vulkan] $1: $2 $3"
    [ "$2" = pass ] || failed=1
}
# session_for NAME WAIT ENV CLIENT: a session with the renderer's environment and one client, and what they logged under
# $out/NAME.*, then a QMP capture of the card's display where there is one.
session_for() {
    local name=$1 wait=$2 env=$3 client=$4
    guest "$env $session $shell_path $(printf %q "$client")" >"$out/$name.session.txt" 2>&1 || true
    sleep "$wait"
    guest 'cat ~/session/gnoblin.log' >"$out/$name.gnoblin.log" 2>&1 || true
    guest 'cat ~/session/client.log' >"$out/$name.client.log" 2>&1 || true
    if [ "$mode" = software ]; then
        "$here/qmp.py" "$qmp" screendump "$out/$name.png" 2>"$out/$name.capture.err" || true
    fi
}
# serves NAME: the one reason a session is not a working Vulkan session, or nothing.
serves() {
    local name=$1 state
    state=$(guest 'pgrep -x gnoblin >/dev/null && echo serving || echo stopped; pgrep -x foot >/dev/null && echo running || echo exited' 2>/dev/null | tr '\n' ' ')
    if ! grep -q serving <<<"$state"; then
        echo "gnoblin stopped (see $name.gnoblin.log)"
    elif ! grep -q 'the shell .* serves' "$out/$name.gnoblin.log"; then
        echo "the shell did not start (see $name.gnoblin.log)"
    elif ! grep -q 'the Vulkan renderer is ready' "$out/$name.gnoblin.log"; then
        echo "no Vulkan renderer (see $name.gnoblin.log)"
    elif grep -q 'frame was refused' "$out/$name.gnoblin.log"; then
        echo "the kernel refused a frame (see $name.gnoblin.log)"
    elif ! grep -q running <<<"$state"; then
        echo "foot exited (see $name.client.log)"
    fi
}
device_of() { grep -m1 -oE 'device=.*kind=[A-Za-z]+' "$out/$1.gnoblin.log" || echo "device unknown"; }
# pixels NAME FILE: whether FILE shows the red window with the desktop around it.
pixels() {
    local name=$1 file=$2 share colours
    share=$(python3 "$here/pixels.py" "$file" share ff0000 2>&1) || {
        check "$name" fail "the picture cannot be read: $share"
        return
    }
    colours=$(python3 "$here/pixels.py" "$file" colours)
    if awk -v s="$share" 'BEGIN { exit !(s > 0.03 && s < 0.95) }' && [ "$colours" -gt 1 ]; then
        check "$name" pass "the red window is $share of the picture, $colours colours"
    else
        check "$name" fail "the red window is $share of the picture, $colours colours (see $(basename "$file"))"
    fi
}

# foot 1.26 renamed the `colors` section to `colors-dark`, so `-o colors.background=` is an error on Arch and Tumbleweed (foot 1.28)
# and `-o colors-dark.background=` one on Debian 13 (foot 1.21). The terminal's own escape sequence for the default background
# (OSC 11) works on every version, and paints the whole window.
paint='printf "\033]11;rgb:ff/00/00\007"'
red="foot sh -c '$paint; sleep 3600'"
session_for vulkan-foot 8 'GNOBLIN_CI_RENDERER=vulkan' "$red"
reason=$(serves vulkan-foot)
if [ -n "$reason" ]; then
    check vulkan-foot fail "$reason"
else
    check vulkan-foot pass "$(device_of vulkan-foot)"
fi
if [ "$mode" = software ] && [ -s "$out/vulkan-foot.png" ]; then
    pixels vulkan-foot-pixels "$out/vulkan-foot.png"
elif [ "$mode" = software ]; then
    check vulkan-foot-pixels fail "QEMU could not capture the display (see vulkan-foot.capture.err)"
fi

# vkms: a display with no GPU behind it. The client redraws for as long as the check waits, so the flips are a count of frames
# that kept completing and not of one.
vkms_card=$(guest 'sudo modprobe vkms >/dev/null 2>&1; sleep 1; for c in /sys/class/drm/card[0-9]*; do case "$c" in *-*) continue ;; esac; [ "$(basename "$(readlink -f "$c/device")")" = vkms ] && echo "/dev/dri/$(basename "$c")"; done | head -1' 2>/dev/null || true)
if [ -z "$vkms_card" ]; then
    why=$(guest 'modinfo vkms >/dev/null 2>&1 && echo "vkms is built but did not load" || echo "this kernel has no vkms module"' 2>/dev/null || true)
    for name in vkms-vulkan vkms-flips vkms-screenshot; do
        check "$name" pass "skipped: ${why:-vkms could not be checked}"
    done
else
    session_for vkms-vulkan 30 "GNOBLIN_CI_RENDERER=vulkan GNOBLIN_DRM_DEVICE=$vkms_card GNOBLIN_LOG=backend:debug" \
        "foot sh -c '$paint; while :; do echo \$RANDOM; sleep 0.02; done'"
    reason=$(serves vkms-vulkan)
    if [ -n "$reason" ]; then
        check vkms-vulkan fail "$reason"
    else
        check vkms-vulkan pass "$vkms_card: $(device_of vkms-vulkan)"
    fi
    flips=$(grep -oE 'flips=[0-9]+' "$out/vkms-vulkan.gnoblin.log" | tail -1 | cut -d= -f2 || true)
    if [ "${flips:-0}" -ge 120 ]; then
        check vkms-flips pass "$flips frames flipped on $vkms_card"
    else
        check vkms-flips fail "${flips:-0} frames flipped on $vkms_card, 120 are expected (see vkms-vulkan.gnoblin.log)"
    fi
    # The ordinary socket has no screencopy on purpose; grim goes through the privileged one.
    guest 'WAYLAND_DISPLAY=gnoblin-wayland-privileged.sock XDG_RUNTIME_DIR=/run/user/$(id -u) grim ~/vkms.png' >"$out/vkms-grim.txt" 2>&1 || true
    guest 'cat ~/vkms.png' >"$out/vkms-grim.png" 2>/dev/null || true
    if [ -s "$out/vkms-grim.png" ]; then
        pixels vkms-screenshot "$out/vkms-grim.png"
    else
        check vkms-screenshot fail "grim wrote no picture (see vkms-grim.txt)"
    fi
fi
exit "$failed"
