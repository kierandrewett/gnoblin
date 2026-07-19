#!/usr/bin/env bash
# Performance smoke test for the patched shell. Boots headless in gnoblin mode
# and checks three regression-prone metrics against thresholds:
#   1. idle memory: private dirty after settle, and growth over a 60 s window
#   2. soft-reload memory: growth across 10 org.gnoblin.Shell.Reload calls
#   3. window churn: shell RSS growth across 15 client open/close cycles
# Thresholds are generous (headless llvmpipe numbers are noisy) — this catches
# reintroduced leaks, not small drifts. See TODO.md "Performance" for the
# measured baselines behind them.
#
# Env: GNOBLIN_PREFIX (default ./install), PERF_CLIENT (default foot).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
source "$ROOT/scripts/gnoblin-state.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
PERF_CLIENT="${PERF_CLIENT:-foot}"
[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

# Thresholds (kB).
IDLE_PRIVATE_DIRTY_MAX=150000     # measured ~101 MB
IDLE_GROWTH_MAX=4000              # measured ~0 (shrinks)
RELOAD_GROWTH_MAX=8000            # measured ~0; was +38 MB per 10 before fixes
CHURN_GROWTH_MAX=8000             # measured ~0 (shrinks)

source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland

DK="$(mktemp -d /tmp/gnoblin-perf.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GSETTINGS_BACKEND=memory GTK_A11Y=none NO_AT_BRIDGE=1
export DISP="gnoblin-perf-$$" SHELL_LOG="$DK/shell.log"

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  pkill -f "$DK/" 2>/dev/null
  [ -f "$SHELL_LOG" ] && gnoblin_publish_log "$SHELL_LOG" perf-smoke-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT TERM HUP INT

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

export PERF_CLIENT IDLE_PRIVATE_DIRTY_MAX IDLE_GROWTH_MAX RELOAD_GROWTH_MAX CHURN_GROWTH_MAX
dbus-run-session --config-file="$CONF" -- bash -uo pipefail -c '
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor 1280x800 --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  gdbus wait --session --timeout 45 org.gnoblin.Shell || { echo "[perf] FAIL: shell never up"; exit 1; }
  sleep 10

  pd() { awk "/^Private_Dirty/{print \$2}" /proc/$SHELL_PID/smaps_rollup; }
  rss() { awk "/VmRSS/{print \$2}" /proc/$SHELL_PID/status; }
  gnoblin() { gdbus call --session --dest org.gnoblin.Shell --object-path /org/gnoblin/Shell \
               --method "org.gnoblin.Shell.$1" >/dev/null 2>&1; }
  rc=0

  # 1. idle memory
  pd0=$(pd)
  echo "[perf] idle private dirty after settle: $pd0 kB (max $IDLE_PRIVATE_DIRTY_MAX)"
  [ "$pd0" -le "$IDLE_PRIVATE_DIRTY_MAX" ] || { echo "[perf] FAIL: idle private dirty over budget"; rc=1; }
  sleep 60
  pd1=$(pd)
  growth=$((pd1 - pd0))
  echo "[perf] idle growth over 60 s: $growth kB (max $IDLE_GROWTH_MAX)"
  [ "$growth" -le "$IDLE_GROWTH_MAX" ] || { echo "[perf] FAIL: idle memory grows"; rc=1; }

  # 2. soft reloads
  pd0=$(pd)
  for _ in $(seq 1 10); do
    gnoblin Reload || { echo "[perf] FAIL: Reload errored"; rc=1; break; }
  done
  sleep 5
  pd1=$(pd)
  growth=$((pd1 - pd0))
  echo "[perf] growth across 10 soft reloads: $growth kB (max $RELOAD_GROWTH_MAX)"
  [ "$growth" -le "$RELOAD_GROWTH_MAX" ] || { echo "[perf] FAIL: soft reloads leak"; rc=1; }

  # 3. window churn
  if command -v "$PERF_CLIENT" >/dev/null; then
    r0=$(rss)
    for _ in $(seq 1 15); do
      WAYLAND_DISPLAY="$DISP" "$PERF_CLIENT" -e true >/dev/null 2>&1 &
      CPID=$!
      sleep 1
      kill $CPID 2>/dev/null; wait $CPID 2>/dev/null
      sleep 0.5
    done
    sleep 5
    r1=$(rss)
    growth=$((r1 - r0))
    echo "[perf] rss growth across 15 window cycles: $growth kB (max $CHURN_GROWTH_MAX)"
    [ "$growth" -le "$CHURN_GROWTH_MAX" ] || { echo "[perf] FAIL: window churn leaks"; rc=1; }
  else
    echo "[perf] skip window churn: $PERF_CLIENT not installed"
  fi

  kill $SHELL_PID 2>/dev/null
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (perf smoke within budgets)" \
              || echo ">> RESULT: FAIL (perf smoke, rc=$rc). log -> $GNOBLIN_STATE_DIR/perf-smoke-last.log" >&2
exit "$rc"
