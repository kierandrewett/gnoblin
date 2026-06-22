#!/usr/bin/env bash
# Regression: a layer surface bound to an explicit wl_output must receive
# zwlr_layer_surface_v1.closed when that output disappears.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SRC="$ROOT/scripts/layer-shell-probe.c"
LS_XML="$ROOT/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
XDG_XML="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v wayland-scanner >/dev/null || { echo "SKIP: no wayland-scanner"; exit 0; }
command -v grim >/dev/null || { echo "SKIP: no grim"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0"; exit 0; }

BUILD="$(mktemp -d /tmp/lsout.XXXXXX)"
trap 'rm -rf "$BUILD"' EXIT

wayland-scanner client-header "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-client-protocol.h" || exit 1
wayland-scanner private-code "$LS_XML" "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" || exit 1
wayland-scanner client-header "$XDG_XML" "$BUILD/xdg-shell-client-protocol.h" || exit 1
wayland-scanner private-code "$XDG_XML" "$BUILD/xdg-shell-protocol.c" || exit 1

cc "$SRC" \
   "$BUILD/wlr-layer-shell-unstable-v1-protocol.c" \
   "$BUILD/xdg-shell-protocol.c" \
   -I"$BUILD" $(pkg-config --cflags --libs wayland-client) \
   -o "$BUILD/layer-shell-probe" || {
  echo "FAIL: could not compile layer-shell protocol probe"; exit 1; }

echo "== layer-shell surfaces must close when their requested output disappears =="
run_case() {
  local mode="$1"
  local marker="$2"
  local pass_message="$3"

  PROBE="$BUILD/layer-shell-probe" \
  PROBE_MODE="$mode" \
  PROBE_MAPPED_MARKER="$marker" \
  PROBE_PASS_MESSAGE="$pass_message" \
  GNOBLIN_PREFIX="$PREFIX" \
  python3 - <<'PY'
import importlib.util
import os
import pathlib
import select
import subprocess
import sys
import time

ROOT = pathlib.Path.cwd()
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


MODE = os.environ["PROBE_MODE"]
MAPPED_MARKER = os.environ["PROBE_MAPPED_MARKER"]
PASS_MESSAGE = os.environ["PROBE_PASS_MESSAGE"]


def drain_until_mapped(proc, deadline, lines):
    while time.time() < deadline:
        ready, _, _ = select.select([proc.stdout], [], [], 0.1)
        if not ready:
            if proc.poll() is not None:
                break
            continue

        line = proc.stdout.readline()
        if not line:
            break

        print(line, end="")
        lines.append(line)
        if MAPPED_MARKER in line:
            return True

    return any(MAPPED_MARKER in line for line in lines)


old_clients = dh.CLIENTS
dh.CLIENTS = False
dk = dh.Devkit()
proc = None
try:
    dk.boot(with_monitor=False, per_output=False)
    if not dk.add_monitor_late(1280, 800):
        print("SKIP: virtual monitor never materialized")
        sys.exit(0)

    env = dk._env()
    env["WAYLAND_DISPLAY"] = dk.disp
    env["PROBE_WAIT_CLOSED_SECONDS"] = "20"
    proc = subprocess.Popen(
        [os.environ["PROBE"], MODE],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )

    lines = []
    if not drain_until_mapped(proc, time.time() + 10, lines):
        if proc.poll() is None:
            proc.terminate()
        print(f"FAIL: layer-shell probe '{MODE}' did not map before output removal")
        sys.exit(1)

    for consumer in dk._consumers:
        consumer.terminate()
    for consumer in dk._consumers:
        try:
            consumer.wait(timeout=2)
        except Exception:
            consumer.kill()
    dk._consumers.clear()
    dk._sc_session.call_sync("Stop", None, dh.Gio.DBusCallFlags.NONE, -1, None)
    print("  stopped late virtual output session")

    try:
        out, _ = proc.communicate(timeout=25)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, _ = proc.communicate()
        print(out, end="")
        print(f"FAIL: layer-shell probe '{MODE}' hung waiting for closed")
        sys.exit(1)

    print(out, end="")
    if dk.crashed():
        print(f"FAIL: compositor crashed: {dk.crashed()}")
        sys.exit(1)
    if proc.returncode != 0:
        print(f"FAIL: layer-shell probe '{MODE}' exited {proc.returncode}")
        sys.exit(1)

    print(PASS_MESSAGE)
finally:
    if proc and proc.poll() is None:
        proc.terminate()
    dk.teardown()
    dh.CLIENTS = old_clients
PY
}

run_case \
  "wait-closed" \
  "wait-closed: mapped" \
  "PASS: requested-output layer surface received closed on output removal" || exit 1

run_case \
  "closed-ignores-requests" \
  "closed-ignore: mapped" \
  "PASS: requested-output layer surface ignored requests after closed" || exit 1
