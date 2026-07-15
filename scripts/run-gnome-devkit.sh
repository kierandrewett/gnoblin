#!/usr/bin/env bash
# gnoblin devkit — boot a VISIBLE nested gnoblin session (a window in your current
# Wayland session) and open a terminal already wired to it, so you can launch your
# OWN chrome (quickshell, waybar, …) against gnoblin.
#
# Nothing chrome-specific is vendored into this repo: the terminal just gets the
# right WAYLAND_DISPLAY (the nested session), the right D-Bus (so gnoblinctl /
# org.gnoblin.Shell work), and gnoblinctl on PATH. You then run e.g.
#   qs -p ~/dev/kobel-shell
# from that terminal and your bar appears inside the nested gnoblin.
#
# Usage: run-gnome-devkit.sh [TERMINAL]      (TERMINAL defaults to foot/kitty/alacritty)
#   env: MONITOR=1600x900   nested resolution
#        GNOME_DEVKIT_HEADLESS=1   boot headless (no window) — for plumbing tests
#        GNOME_DEVKIT_EXEC="cmd"   run cmd in the devkit env instead of a terminal
#        GNOME_DEVKIT_UNSAFE_MODE=1   explicitly enable privileged shell D-Bus APIs
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/gnoblin-state.sh"
LAST_LOG="$(gnoblin_state_dir)/devkit-last.log"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
MONITOR="${MONITOR:-1600x900}"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — run 'just dev' first" >&2; exit 1; }

if [ "${GNOME_DEVKIT_HEADLESS:-0}" != 1 ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  echo "run-gnome-devkit: no host WAYLAND_DISPLAY — the nested session needs a Wayland session to render into." >&2
  echo "  (log into Wayland, or use GNOME_DEVKIT_HEADLESS=1 for a non-visible boot)" >&2
  exit 1
fi

# Host WAYLAND_DISPLAY (where the --devkit viewer window is drawn) — captured BEFORE
# we scrub the environment below. The shell keeps it; the terminal renders here too;
# clients you launch are pointed at the NESTED display instead.
HOST_WAYLAND="${WAYLAND_DISPLAY:-}"
REAL_HOME="$HOME"

# --- runtime lookup paths for gnome-shell itself (shared with run-gnome-shell.sh,
# the installed login wrappers) -------------------------------------------
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"     # gnome-shell, gnoblinctl on PATH; session mode env

# --- ISOLATION from the host session (this is what makes the devkit a sandbox) ---
# 1. DISPLAY=:0 leaks in from the real session; with it set, GTK/Qt apps prefer X11
#    and connect to the HOST Xwayland — so they open on your real desktop, not the
#    nested one (and gnome-control-center then crashes on the mismatch). Kill it and
#    force the Wayland backends so every launched client uses the nested display.
unset DISPLAY
export GDK_BACKEND=wayland QT_QPA_PLATFORM=wayland CLUTTER_BACKEND=wayland
export MOZ_ENABLE_WAYLAND=1
# 2. No host accessibility bus (would connect back to the real session + spam errors).
export GTK_A11Y=none NO_AT_BRIDGE=1
# 3. No gvfs mounts from the real session.
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1

# --- terminal to spawn ---------------------------------------------------------
TERM_BIN="${1:-}"
if [ -z "$TERM_BIN" ]; then
  for t in foot kitty alacritty wezterm gnome-terminal konsole xterm; do
    command -v "$t" >/dev/null 2>&1 && { TERM_BIN="$t"; break; }
  done
fi

DISP="gnoblin-devkit-$$"

# --- isolated D-Bus, reachable by the shell AND the terminal AND your qs --------
# (dbus-daemon --print-address so all three can share it; not dbus-run-session,
#  which would wrap a single command.)
DK="$(mktemp -d /tmp/gnoblin-devkit.XXXXXX)"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
DBUS_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || { rm -rf "$DK"; exit 1; }
DBUS_PID_FILE="$DK/dbus.pid"
export DBUS_SESSION_BUS_ADDRESS="$(dbus-daemon --config-file="$DBUS_CONF" --print-address --fork --print-pid=3 3>"$DBUS_PID_FILE")"
DBUS_PID="$(cat "$DBUS_PID_FILE" 2>/dev/null || true)"

SHELL_PID=
_cleaned=
cleanup() {
  [ -n "$_cleaned" ] && return; _cleaned=1
  # Kill the shell's whole process group — takes the MDK viewer + anything it
  # spawned with it. Clients you launched live in the terminal's own tree and
  # die with the terminal.
  [ -n "$SHELL_PID" ] && kill -- "-$SHELL_PID" 2>/dev/null
  [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
  [ -n "$DBUS_PID" ] && kill "$DBUS_PID" 2>/dev/null
  [ -f "$DK/shell.log" ] && gnoblin_publish_log "$DK/shell.log" devkit-last.log 2>/dev/null || true
  rm -rf "$DK"
}
# EXIT is the single cleanup site; signals just exit into it (no double-run).
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

# --- boot the nested gnoblin session ------------------------------------------
# VISIBLE: --devkit opens mutter's development-kit viewer (a window in your current
#   session) — it does NOT take over the seat, so it coexists with your real GNOME.
#   (Plain --wayland would fall back to the native/KMS backend and fight for the
#   seat: "Failed to take control of the session: EBUSY".)
# HEADLESS (tests/CI): --headless + a virtual monitor, no window.
SECURITY_ARGS=()
case "${GNOME_DEVKIT_UNSAFE_MODE:-0}" in
  0) ;;
  1)
    SECURITY_ARGS=(--unsafe-mode)
    echo ">> WARNING: enabling unsafe mode for this isolated devkit shell"
    ;;
  *)
    echo "run-gnome-devkit: GNOME_DEVKIT_UNSAFE_MODE must be 0 or 1" >&2
    exit 1
    ;;
esac

if [ "${GNOME_DEVKIT_HEADLESS:-0}" = 1 ]; then
  BACKEND=(--headless --virtual-monitor "$MONITOR")
  echo ">> booting gnoblin (mode=gnoblin, headless, display=$DISP) ..."
else
  BACKEND=(--devkit)
  echo ">> booting gnoblin (mode=gnoblin, devkit viewer window, display=$DISP) ..."
fi
# Gnoblin has no stock chrome. Keep user extensions out of the nested shell so
# dock/panel extensions from the normal GNOME session cannot leak into devkit.
GNOME_SHELL_DISABLE_EXTENSIONS=1 WAYLAND_DISPLAY="$HOST_WAYLAND" setsid \
  "$SHELL_BIN" --wayland --no-x11 --mode=gnoblin \
  "${SECURITY_ARGS[@]}" \
  "${BACKEND[@]}" --wayland-display "$DISP" \
  >"$DK/shell.log" 2>&1 &
SHELL_PID=$!

# wait for the control protocol (implies the shell is up + serving $DISP)
if ! gdbus wait --session --timeout 30 org.gnoblin.Shell; then
  echo "!! gnoblin did not come up:" >&2; tail -n 30 "$DK/shell.log" >&2
  exit 1
fi
echo ">> gnoblin up. org.gnoblin.Shell owned; nested wayland display = $DISP"

# Push the nested display + Wayland backends into the isolated bus's activation
# environment, so D-Bus-activated apps (gnome-control-center, nautilus, …) spawn
# INSIDE the devkit rather than inheriting the host's WAYLAND_DISPLAY. No --systemd:
# this is a standalone bus, and we must not touch the real user's systemd env.
WAYLAND_DISPLAY="$DISP" dbus-update-activation-environment \
  WAYLAND_DISPLAY GDK_BACKEND QT_QPA_PLATFORM CLUTTER_BACKEND \
  XDG_CURRENT_DESKTOP GTK_A11Y NO_AT_BRIDGE GIO_USE_VFS 2>/dev/null || true

# --- env for anything you launch from the devkit shell ------------------------
# DBUS_SESSION_BUS_ADDRESS + PATH are already exported. WAYLAND_DISPLAY is handled
# per-case below: the terminal is a HOST window (renders in your real session), but
# its shell points children (qs/foot) at the NESTED gnoblin display.
export GDK_BACKEND=wayland QT_QPA_PLATFORM=wayland

MOTD='cat <<EOF
── gnoblin devkit ──────────────────────────────────────────────
This is a host terminal; commands you run here target the nested
gnoblin session ('"$DISP"'). Launch your chrome, e.g.:

    qs -p ~/dev/kobel-shell        # Quickshell (kobel-shell)
    # or: waybar / your own layer-shell client

Drive gnoblin:  gnoblinctl ping | version | reload | features
Close this terminal to end the devkit.
────────────────────────────────────────────────────────────────
EOF'

# Plumbing/test hook: run a command with children pointed at the nested session.
if [ -n "${GNOME_DEVKIT_EXEC:-}" ]; then
  echo ">> GNOME_DEVKIT_EXEC: $GNOME_DEVKIT_EXEC"
  WAYLAND_DISPLAY="$DISP" bash -c "$GNOME_DEVKIT_EXEC"
  exit $?
fi

[ -n "$TERM_BIN" ] || { echo "no terminal found (install foot/kitty/alacritty, or pass one)"; exit 1; }
echo ">> opening $TERM_BIN as a host window (its shell targets the nested session) ..."

# The terminal renders on the HOST (WAYLAND_DISPLAY=$HOST_WAYLAND); its interactive
# shell exports WAYLAND_DISPLAY=$DISP so qs etc. draw in the nested gnoblin.
# NB: each statement on its own line — the MOTD ends with a heredoc terminator (EOF),
# which must stand alone, so ';' would fold the next command into the heredoc body.
INNER="export WAYLAND_DISPLAY='$DISP'
$MOTD
exec bash -i"
case "$TERM_BIN" in
  alacritty|wezterm|konsole|xterm) WAYLAND_DISPLAY="$HOST_WAYLAND" "$TERM_BIN" -e bash -c "$INNER" ;;
  gnome-terminal)                  WAYLAND_DISPLAY="$HOST_WAYLAND" "$TERM_BIN" -- bash -c "$INNER" ;;
  *)                               WAYLAND_DISPLAY="$HOST_WAYLAND" "$TERM_BIN" bash -c "$INNER" ;;   # foot, kitty
esac

# Terminal closed — tear the session down.
echo ">> terminal closed; shutting down the nested gnoblin."
