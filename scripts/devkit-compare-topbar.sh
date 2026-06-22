#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:-restart}"
RENDERER="${GNOBLIN_DEVKIT_GSK_RENDERER:-}"
FORCE_TOPBAR_HOVER="${GNOBLIN_TOPBAR_COMPARE_FORCE_HOVER:-${GNOBLIN_TOPBAR_FORCE_HOVER:-}}"

shell_pid() {
  ps -eo pid=,args= |
    awk '/gnoblin-shell/ && /--devkit/ { print $1; exit }'
}

prefix_from_pid() {
  local pid="$1" exe
  exe="$(readlink "/proc/$pid/exe" 2>/dev/null || true)"
  if [ -n "$exe" ]; then
    dirname "$(dirname "$exe")"
  else
    printf '%s\n' "$ROOT/install"
  fi
}

env_from_pid() {
  local pid="$1" key="$2"
  tr '\0' '\n' <"/proc/$pid/environ" |
    awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; exit }'
}

wayland_display_from_pid() {
  local pid="$1"
  tr '\0' ' ' <"/proc/$pid/cmdline" |
    sed -n 's/.*--wayland-display \([^ ]*\).*/\1/p'
}

run_component() {
  local unit="$1"
  local description="$2"
  local command="$3"
  shift 3

  systemd-run --user --collect --unit="$unit" \
    --description="$description" \
    "$@" \
    /usr/bin/env -u DISPLAY "$command"
}

stop_devkit_layer_clients() {
  local pid cmd env

  for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    [ "$pid" = "$$" ] && continue
    cmd="$({ tr '\0' ' ' < "$proc/cmdline"; } 2>/dev/null || true)"
    case "$cmd" in
      *gnoblin-topbar*|*gnoblin-dock*) ;;
      *) continue ;;
    esac
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$env" in
      *"WAYLAND_DISPLAY=$nested_display"*)
        kill "$pid" 2>/dev/null || true
        ;;
    esac
  done
}

pid="$(shell_pid)"
if [ -z "$pid" ]; then
  echo "devkit-compare-topbar: no live gnoblin-shell --devkit process found" >&2
  exit 1
fi

prefix="$(prefix_from_pid "$pid")"
nested_display="$(wayland_display_from_pid "$pid")"
bus="$(env_from_pid "$pid" DBUS_SESSION_BUS_ADDRESS)"
runtime_dir="$(env_from_pid "$pid" XDG_RUNTIME_DIR)"

if [ -z "$nested_display" ] || [ -z "$bus" ] || [ -z "$runtime_dir" ]; then
  echo "devkit-compare-topbar: missing nested shell environment" >&2
  exit 1
fi

common_env=(
  "--setenv=WAYLAND_DISPLAY=$nested_display"
  "--setenv=GDK_BACKEND=wayland"
  "--setenv=DBUS_SESSION_BUS_ADDRESS=$bus"
  "--setenv=XDG_RUNTIME_DIR=$runtime_dir"
  "--setenv=GTK_A11Y=none"
)

for key in \
  XDG_DATA_HOME \
  XDG_CONFIG_HOME \
  XDG_CACHE_HOME \
  XDG_DATA_DIRS \
  GSETTINGS_SCHEMA_DIR \
  GI_TYPELIB_PATH \
  LD_LIBRARY_PATH \
  PATH
do
  value="$(env_from_pid "$pid" "$key")"
  if [ -n "$value" ]; then
    common_env+=("--setenv=$key=$value")
  fi
done

if [ -n "$RENDERER" ]; then
  common_env+=("--setenv=GSK_RENDERER=$RENDERER")
fi
if [ -n "$FORCE_TOPBAR_HOVER" ]; then
  common_env+=("--setenv=GNOBLIN_TOPBAR_FORCE_HOVER=$FORCE_TOPBAR_HOVER")
fi

systemctl --user stop gnoblin-topbar-live.service gnoblin-dock-live.service \
  >/dev/null 2>&1 || true
stop_devkit_layer_clients
sleep 0.5

topbar="$prefix/bin/gnoblin-topbar"
dock="$prefix/bin/gnoblin-dock"
[ -x "$topbar" ] || { echo "devkit-compare-topbar: missing $topbar; run just dev-userspace" >&2; exit 1; }
[ -x "$dock" ] || { echo "devkit-compare-topbar: missing $dock; run just dev-userspace" >&2; exit 1; }

case "$MODE" in
  off)
    echo "devkit-compare-topbar: stopped live topbar/dock on $nested_display"
    exit 0
    ;;
  on|restart)
    ;;
  *)
    echo "usage: $0 [on|off|restart]" >&2
    exit 2
    ;;
esac

run_component gnoblin-topbar-live "Gnoblin topbar live" \
  "$topbar" \
  "${common_env[@]}"

run_component gnoblin-dock-live "Gnoblin dock live" \
  "$dock" \
  "${common_env[@]}"

echo "devkit-compare-topbar: $MODE on $nested_display"
