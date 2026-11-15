#!/usr/bin/env bash
# Runs the nested backend against a real host compositor, headless: Sway or Weston instead of this repository's own host.
#
#   scripts/real-host-conformance.sh sway
#   scripts/real-host-conformance.sh weston
#
# The scenarios are `crates/gnoblin-wayland/tests/real_host.rs` (ignored unless a host is named). The host runs on a
# virtual output with the software renderer, so the machine needs no GPU and no display. What that host can and cannot
# show is in docs/backends.md.
#
# Needs: sway (1.7 or later) or weston (10 or later), wl-clipboard for the clipboard scenarios, and the Rust toolchain.
#   Debian: apt-get install sway weston wl-clipboard
set -euo pipefail

host="${1:?usage: real-host-conformance.sh sway|weston}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

runtime="$(mktemp -d)"
chmod 700 "$runtime"
export XDG_RUNTIME_DIR="$runtime"
log="$runtime/host.log"
host_pid=""
cleanup() {
    if [ -n "$host_pid" ]; then
        kill "$host_pid" 2>/dev/null || true
        wait "$host_pid" 2>/dev/null || true
    fi
    rm -rf "$runtime"
}
trap cleanup EXIT

case "$host" in
    sway)
        command -v sway >/dev/null || {
            echo "real-host-conformance.sh: sway is not installed" >&2
            exit 2
        }
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway --unsupported-gpu >"$log" 2>&1 &
        host_pid=$!
        socket=wayland-1
        ;;
    weston)
        command -v weston >/dev/null || {
            echo "real-host-conformance.sh: weston is not installed" >&2
            exit 2
        }
        weston --backend=headless-backend.so --use-pixman --no-config --socket=wayland-weston >"$log" 2>&1 &
        host_pid=$!
        socket=wayland-weston
        ;;
    *)
        echo "real-host-conformance.sh: the host is sway or weston" >&2
        exit 2
        ;;
esac

# The host is ready when its socket exists.
for _ in $(seq 1 100); do
    [ -S "$runtime/$socket" ] && break
    kill -0 "$host_pid" 2>/dev/null || {
        echo "real-host-conformance.sh: $host exited:" >&2
        cat "$log" >&2
        exit 1
    }
    sleep 0.1
done
[ -S "$runtime/$socket" ] || {
    echo "real-host-conformance.sh: $host never made $socket:" >&2
    cat "$log" >&2
    exit 1
}

if [ "$host" = sway ]; then
    SWAYSOCK="$(ls "$runtime"/sway-ipc.*.sock | head -1)"
    export SWAYSOCK
fi

echo "host: $host on $socket"
GNOBLIN_HOST_SOCKET="$socket" cargo test -p gnoblin-wayland --test real_host -- --ignored --nocapture --test-threads=1
