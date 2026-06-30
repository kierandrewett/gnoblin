#!/usr/bin/env bash
# Run the gnoblin stack from its dev prefix (./install) inside an isolated
# dbus + GSettings + XDG sandbox. gnoblin's equivalent of the GNOME/mutter
# devkit: a nested compositor (gnoblin-shell on the patched libmutter) you can
# test without touching your real session.
#
# Build the stack first with `just dev` (builds + installs mutter incl. the
# Mutter Devkit viewer, gnoblin-shell, and the layer-shell clients into ./install).
#
# Modes:
#   verify   (default) headless — boot, print the protocols advertised, exit.
#   visible  windowed — launch the Mutter Devkit viewer running gnoblin-shell as
#            a window on your current session. Optional trailing COMMAND is run
#            inside the nested compositor, e.g.  run-devkit.sh visible foot
#
# Env: GNOBLIN_PREFIX (default ./install), MONITOR (default 1280x800),
#      CLIENTS=1 to autostart gnoblin's layer-shell clients.
set -uo pipefail

MODE="${1:-verify}"
[ "$#" -gt 0 ] && shift || true
CLIENT=("$@")

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnoblin-shell"
MONITOR="${MONITOR:-1280x800}"
LAST_LOG=/tmp/gnoblin-shell-last.log
HOST_HOME="${HOME:-$(getent passwd "$(id -u)" | cut -d: -f6)}"
# The whole point of the windowed devkit is to see the layer shell we built, so
# `visible` autostarts the bar/dock/wallpaper/notifyd by default (CLIENTS=0 to
# get a bare compositor). `verify` is a headless protocol probe — no clients.
case "$MODE" in
  visible) CLIENTS="${CLIENTS:-1}" ;;
  *)       CLIENTS="${CLIENTS:-0}" ;;
esac

[ -x "$SHELL_BIN" ] || { echo "stack not built — run: just dev" >&2; exit 1; }
[ -f "$PREFIX/lib64/libmutter-17.so.0" ] || { echo "no mutter in $PREFIX — run: just dev" >&2; exit 1; }

warn_if_stale_artifacts() {
  local newer_shell
  newer_shell="$(
    find "$ROOT/src/compositor" "$ROOT/src/config" "$ROOT/src/protocols" \
      -type f \( -name '*.[ch]' -o -name '*.cpp' -o -name '*.hpp' -o -name '*.xml' -o -name 'meson.build' \) \
      -newer "$SHELL_BIN" -print -quit 2>/dev/null || true
  )"
  if [ -n "$newer_shell" ]; then
    echo "WARN: installed gnoblin-shell is older than source changes." >&2
    echo "      newest source: $newer_shell" >&2
    echo "      run: just dev-shell   (or just dev)" >&2
  fi

  local client_bins=(
    gnoblin-topbar gnoblin-dock gnoblin-window-menu gnoblin-wallpaper
    gnoblin-notifyd gnoblin-osd gnoblin-launcher gnoblin-night-light
    gnoblin-power-menu
  )
  local stale_clients=()
  local bin artifact newer_client crate
  for bin in "${client_bins[@]}"; do
    artifact="$PREFIX/bin/$bin"
    [ -x "$artifact" ] || continue
    crate="${bin#gnoblin-}"
    newer_client="$(
      find "$ROOT/src/clients/Cargo.toml" \
        "$ROOT/src/clients/Cargo.lock" \
        "$ROOT/src/clients/crates" \
        "$ROOT/src/clients/$crate" \
        -type f \( -name '*.rs' -o -name '*.slint' -o -name 'Cargo.toml' -o -name 'Cargo.lock' -o -name 'build.rs' \) \
        -newer "$artifact" -print -quit 2>/dev/null || true
    )"
    [ -n "$newer_client" ] && stale_clients+=("$bin")
  done
  if [ "${#stale_clients[@]}" -gt 0 ]; then
    echo "WARN: installed Slint/userspace clients may be stale: ${stale_clients[*]}" >&2
    echo "      run: just dev-userspace   (or just dev)" >&2
  fi
}
warn_if_stale_artifacts

emit_default_wallpaper() {
  local wallpaper="${GNOBLIN_WALLPAPER:-$HOST_HOME/Documents/wallpaper_light.jpg}"
  [ -f "$wallpaper" ] || return 0
  printf 'wallpaper = %s\n' "$wallpaper"
  printf 'wallpaper-style = zoom\n'
}

# Point the runtime at the dev prefix (patched mutter + its Devkit viewer).
export LD_LIBRARY_PATH="$PREFIX/lib64:$PREFIX/lib64/mutter-17${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GI_TYPELIB_PATH="$PREFIX/lib64/mutter-17${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
# All shell clients install to $PREFIX/bin.
export PATH="$PREFIX/bin:$PATH"
export GSETTINGS_SCHEMA_DIR="$PREFIX/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
# Our layer-shell clients (gtk4-layer-shell) MUST use the Wayland GDK backend.
# When a devkit runs alongside an existing session, DISPLAY=:0 leaks in and GTK
# would otherwise pick X11 — then gtk_layer_init_for_window() fails with "not on
# Wayland" and the bar/dock never map. Force Wayland for all spawned clients.
export GDK_BACKEND=wayland

# Isolated, throwaway state so dconf/cache never touch the real session.
DK="$(mktemp -d /tmp/gnoblin-devkit.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home"
export XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
export GSETTINGS_BACKEND=memory
export GTK_A11Y=none NO_AT_BRIDGE=1
export XDG_CURRENT_DESKTOP=GNOME:Gnoblin

# Keep the Mutter Devkit viewer's ScreenCast portal path, but do not let the
# private bus inherit the host desktop's Secret portal preference. There is no
# secret service in this throwaway session, and probing it costs a 25s timeout.
mkdir -p "$DK/config/xdg-desktop-portal"
cat > "$DK/config/xdg-desktop-portal/gnoblin-portals.conf" <<'EOF'
[preferred]
default=gtk
org.freedesktop.impl.portal.ScreenCast=gnome
org.freedesktop.impl.portal.RemoteDesktop=gnome
org.freedesktop.impl.portal.Screenshot=gnome
org.freedesktop.impl.portal.GlobalShortcuts=gnome
org.freedesktop.impl.portal.Background=none
org.freedesktop.impl.portal.Clipboard=none
org.freedesktop.impl.portal.InputCapture=none
org.freedesktop.impl.portal.Lockdown=none
org.freedesktop.impl.portal.Secret=none
org.freedesktop.impl.portal.Usb=none
org.freedesktop.impl.portal.Wallpaper=none
EOF

# Do not let the private bus auto-discover the host session's full service
# directory. xdg-desktop-portal is needed for Mutter Devkit ScreenCast, but the
# real document portal tries to mount FUSE at /run/user/$UID/doc and can fail
# noisily when another session already owns that mount.
DBUS_SESSION_CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

# Seed a gnoblin.conf in the sandbox (the compositor reads it, not GSettings).
# CLIENTS=1 autostarts the gnoblin topbar/dock via the config's `autostart`.
mkdir -p "$DK/config/gnoblin"
{
  echo "# gnoblin devkit config (generated by run-devkit.sh)"
  echo "[startup]"
  if [ "$CLIENTS" = 1 ]; then
    # The layer shell we built — these autostart inside the devkit window.
    echo "exec_per_output = gnoblin-topbar"
    echo "exec_per_output = gnoblin-dock"
    echo "exec = gnoblin-notifyd"
    echo "exec_per_output = gnoblin-wallpaper"
  fi
  echo "[appearance]"
  echo "background = \"#1d1f21\""
  emit_default_wallpaper
  echo "rounding = 12"          # rounded window corners, so the effect is visible
  echo 'shadow = "0 20px 48px -20px rgba(0,0,0,.22), 0 4px 12px -6px rgba(0,0,0,.14)"'
  # The compositor-managed window effects (these mirror gnoblin.defaults.conf,
  # spelled out here so the devkit visibly exercises them and they're easy to
  # tweak in one place). The two-layer ring border hugs each window's surface.
  echo "[effects]"
  echo "rounding = 12"
  echo "border-style = ring"
  echo "[animations]"
  echo "enabled = on"
  echo "[bind]"
  echo "Super+Q = close"
  echo "Super+Up = maximize"
  echo "Super+M = minimize"
  echo "Super+Left = snap left"
  echo "Super+Right = snap right"
  echo "Super+Return = snap center"
  echo "Super+Space = spawn gnoblin-launcher"
  echo "Super+L = lock"           # test the lock screen (PAM) inside the devkit
  echo "Super+Escape = window-menu" # pop the window menu for the focused window
  echo "[roles]"
  echo "window-menu = gnoblin-window-menu"  # the Slint window menu (right-click titlebar)
  echo "[features]"
  echo "appmenu = on"              # the topbar global appmenu API
  echo "[topbar]"
  echo "left = workspaces, focused-app, appmenu, spring"
  echo "center = clock"
  echo "right = launcher, tray, status"
  echo "appmenu-backend = auto"
} > "$DK/config/gnoblin/gnoblin.conf"

DISP="gnoblin-devkit-$$"
SHELL_PID=
WATCHDOG_PID=
cleanup_done=0
publish_log() {
  [ -f "$DK/shell.log" ] && cp "$DK/shell.log" "$LAST_LOG" 2>/dev/null || true
}
start_cleanup_watchdog() {
  setsid sh -c '
    parent=$1
    dir=$2
    disp=$3
    conf=$4
    while kill -0 "$parent" 2>/dev/null; do sleep 1; done
    for sig in TERM KILL; do
      for proc in /proc/[0-9]*; do
        pid="${proc##*/}"
        cmd="$(tr "\0" " " < "$proc/cmdline" 2>/dev/null || true)"
        env="$(tr "\0" "\n" < "$proc/environ" 2>/dev/null || true)"
        case "$cmd
$env" in
          *"--wayland-display $disp"*|*"$conf"*|*"WAYLAND_DISPLAY=$disp"*|*"XDG_CONFIG_HOME=$dir/config"*|*"XDG_DATA_HOME=$dir/data"*)
            kill "-$sig" "$pid" 2>/dev/null || true
            ;;
        esac
      done
      [ "$sig" = TERM ] && sleep 1
    done
    sleep 2
    rm -rf "$dir"
  ' sh "$$" "$DK" "$DISP" "$DBUS_SESSION_CONF" >/dev/null 2>&1 &
  WATCHDOG_PID=$!
  disown "$WATCHDOG_PID" 2>/dev/null || true
}
kill_stale_devkit_processes() {
  signal="$1"
  protected_pids=" $$ "
  cur="$$"
  while [ -n "$cur" ] && [ "$cur" != 0 ]; do
    protected_pids="$protected_pids$cur "
    cur="$(ps -o ppid= -p "$cur" 2>/dev/null | tr -d '[:space:]' || true)"
  done
  for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    case "$protected_pids" in
      *" $pid "*) continue ;;
    esac
    cmd="$({ tr '\0' ' ' < "$proc/cmdline"; } 2>/dev/null || true)"
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    text="$cmd
$env"
    case "$text" in
      *"$DK"*|*"$DBUS_SESSION_CONF"*|*"WAYLAND_DISPLAY=$DISP"*|*"--wayland-display $DISP"*)
        continue
        ;;
    esac
    case "$text" in
      *"gnoblin-devkit-"*|*"XDG_CONFIG_HOME=/tmp/gnoblin-devkit."*)
        case "$text" in
          *"$PREFIX/"*|*"XDG_CONFIG_HOME=/tmp/gnoblin-devkit."*)
            kill "-$signal" "$pid" 2>/dev/null || true
            ;;
        esac
        ;;
    esac
  done
}
cleanup_stale_devkit_dirs() {
  refs="$DK/live-devkit-refs"
  : > "$refs"
  for proc in /proc/[0-9]*; do
    {
      tr '\0' ' ' < "$proc/cmdline"
      printf '\n'
      tr '\0' '\n' < "$proc/environ"
      printf '\n'
    } >> "$refs" 2>/dev/null || true
  done
  for dir in /tmp/gnoblin-devkit.*; do
    [ -d "$dir" ] || continue
    [ "$dir" = "$DK" ] && continue
    grep -Fq -- "$dir" "$refs" 2>/dev/null && continue
    rm -rf "$dir"
  done
  rm -f "$refs"
}
kill_devkit_processes() {
  signal="$1"
  for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    [ "$pid" = "$$" ] && continue
    cmd="$({ tr '\0' ' ' < "$proc/cmdline"; } 2>/dev/null || true)"
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$cmd
$env" in
      *"--wayland-display $DISP"*|*"$DBUS_SESSION_CONF"*|*"WAYLAND_DISPLAY=$DISP"*)
        kill "-$signal" "$pid" 2>/dev/null || true
        ;;
    esac
  done
}
kill_devkit_clients() {
  signal="$1"
  for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    [ "$pid" = "$$" ] && continue
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$env" in
      *"WAYLAND_DISPLAY=$DISP"*)
        kill "-$signal" "$pid" 2>/dev/null || true
        ;;
    esac
  done
}
cleanup() {
  [ "$cleanup_done" = 1 ] && return
  cleanup_done=1
  # Let nested clients disconnect before the compositor goes away; otherwise
  # they report Wayland Broken pipe errors during intentional devkit teardown.
  kill_devkit_clients TERM
  sleep 0.5
  kill_devkit_clients KILL
  [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
  kill_devkit_processes TERM
  sleep 0.5
  kill_devkit_processes KILL
  [ -n "$SHELL_PID" ] && wait "$SHELL_PID" 2>/dev/null || true
  publish_log
  rm -rf "$DK"
  # Portal helpers can briefly outlive dbus-run-session on interrupted visible
  # runs and recreate files under the sandbox after the first removal.
  ( sleep 2; rm -rf "$DK" ) >/dev/null 2>&1 &
}
on_signal() {
  cleanup
  exit "$1"
}
trap cleanup EXIT
trap 'on_signal 130' INT
trap 'on_signal 143' TERM
trap 'on_signal 129' HUP
start_cleanup_watchdog

case "$MODE" in
  shot)
    # Headless boot + a real wlr-screencopy capture (grim). grim forces a full
    # output composite, so toplevel windows render with their rounded corners +
    # drop shadows in the PNG — unlike the old clutter self-screenshot, which
    # never drove a paint and only caught the background. Set SHOT_APP to a GUI
    # app to get a window in frame (e.g. SHOT_APP=foot or =gnome-calculator).
    # NB: pure-headless has no frame consumer, so gtk4 layer-shell bars may not
    # paint their first frame here — use `visible`/`just devkit` to see those.
    command -v grim >/dev/null 2>&1 || { echo "shot mode needs 'grim'" >&2; exit 1; }
	    {
	      echo "[appearance]"
	      echo "background = \"#202434\""
	      emit_default_wallpaper
	      echo "rounding = 14"
	      echo 'shadow = "0 20px 48px -20px rgba(0,0,0,.22), 0 4px 12px -6px rgba(0,0,0,.14)"'
      echo "[roles]"
      echo "window-menu = gnoblin-window-menu"
      echo "[startup]"
      if [ "${CLIENTS:-1}" = 1 ]; then
        echo "exec = gnoblin-topbar"
        echo "exec = gnoblin-dock"
      fi
      [ -n "${SHOT_APP:-}" ] && echo "exec = ${SHOT_APP}"
    } > "$DK/config/gnoblin/gnoblin.conf"
    out="${SHOT_OUT:-/tmp/gnoblin-shot.png}"
    rm -f "$out"
    echo ">> booting gnoblin-shell headless for a grim screenshot ..."
    dbus-run-session --config-file="$DBUS_SESSION_CONF" -- "$SHELL_BIN" --headless --no-x11 \
      --virtual-monitor "$MONITOR" --wayland-display "$DISP" \
      >"$DK/shell.log" 2>&1 &
    SHELL_PID=$!
    for _ in $(seq 1 40); do
      sleep 0.5
      [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break
      kill -0 "$SHELL_PID" 2>/dev/null || break
    done
    if [ ! -S "$XDG_RUNTIME_DIR/$DISP" ]; then
      echo "!! gnoblin-shell did not come up:" >&2; tail -n 20 "$DK/shell.log" >&2; exit 1
    fi
    sleep "${SHOT_SETTLE:-7}"   # let clients connect + map + paint a frame
    WAYLAND_DISPLAY="$DISP" grim "$out" 2>>"$DK/shell.log"
    publish_log
    if [ -s "$out" ]; then echo ">> saved $out"; else
      echo "!! no screenshot (log -> $LAST_LOG):" >&2
      tail -n 15 "$DK/shell.log" >&2; exit 1
    fi
    ;;

  verify)
    echo ">> booting gnoblin-shell headless from $PREFIX (isolated dbus/gsettings) ..."
    dbus-run-session --config-file="$DBUS_SESSION_CONF" -- "$SHELL_BIN" --headless --no-x11 \
      --virtual-monitor "$MONITOR" --wayland-display "$DISP" \
      >"$DK/shell.log" 2>&1 &
    SHELL_PID=$!
    for _ in $(seq 1 40); do
      sleep 0.5
      [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break
      kill -0 "$SHELL_PID" 2>/dev/null || break
    done
    if [ ! -S "$XDG_RUNTIME_DIR/$DISP" ]; then
      echo "!! gnoblin-shell did not come up:" >&2; tail -n 20 "$DK/shell.log" >&2; exit 1
    fi
    sleep 1
    probe=/tmp/gnoblin-wl-globals
    if [ ! -x "$probe" ] || [ "$ROOT/scripts/wl-globals.c" -nt "$probe" ]; then
      cc "$ROOT/scripts/wl-globals.c" $(pkg-config --cflags --libs wayland-client) -o "$probe" || exit 1
    fi
    echo "== protocols advertised by gnoblin-shell =="
    WAYLAND_DISPLAY="$DISP" "$probe" | grep -iE "wlr_|ext_" | sort | sed 's/^/   /'
    echo "   (+ $(WAYLAND_DISPLAY="$DISP" "$probe" | wc -l) globals total)"
    ;;

  visible)
    if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
      echo "visible mode needs a host Wayland/X session (WAYLAND_DISPLAY/DISPLAY)" >&2
      exit 1
    fi
    if [ "${GNOBLIN_DEVKIT_KEEP_OLD:-0}" != 1 ]; then
      kill_stale_devkit_processes TERM
      sleep 0.5
      kill_stale_devkit_processes KILL
      cleanup_stale_devkit_dirs
    fi
    command -v pipewire >/dev/null 2>&1 || pgrep -x pipewire >/dev/null 2>&1 || \
      echo "!! warning: PipeWire not detected — the Mutter Devkit viewer needs it to show the window." >&2
    echo ">> opening the gnoblin Mutter Devkit window (isolated dbus/gsettings) ..."
    echo "   nested display: $DISP"
    [ "$CLIENTS" = 1 ] && echo "   autostarting layer shell: topbar, dock, notifyd, wallpaper"
    [ "${#CLIENT[@]}" -gt 0 ] && echo "   + running: ${CLIENT[*]}"
    echo "   NOTE: the Mutter Devkit window can take a few seconds to appear while"
    echo "         it negotiates a PipeWire screencast."
    echo "   close the window to exit. Full log -> $LAST_LOG"
    # --devkit implies a wayland compositor on a headless backend + the MDK viewer
    # window (mutter-devkit) on the host session; clients connect to $DISP.
    #
    # The devkit is software-rendered, so disable animations through the STANDARD
    # desktop setting (org.gnome.desktop.interface enable-animations) inside this
    # isolated session — the compositor (meta_prefs) and every client (gnoblin's
    # and any third-party layer-shell app) all honour it, so the whole devkit is
    # snappy. XDG_CONFIG_HOME is isolated, so this never touches the real session.
    : > "$LAST_LOG"
    : > "$DK/shell.log"
    dbus-run-session --config-file="$DBUS_SESSION_CONF" -- bash -c '
      gsettings set org.gnome.desktop.interface enable-animations false 2>/dev/null || true
      shell_bin=$1
      disp=$2
      log=$3
      shift 3
      "$shell_bin" --devkit --wayland-display "$disp" &
      shell_pid=$!
      if [ "$#" -gt 0 ]; then
        (
          for _ in $(seq 1 80); do
            grep -Fq "Added virtual monitor" "$log" 2>/dev/null && break
            kill -0 "$shell_pid" 2>/dev/null || exit 0
            sleep 0.25
          done
          if grep -Fq "Added virtual monitor" "$log" 2>/dev/null &&
             kill -0 "$shell_pid" 2>/dev/null; then
            WAYLAND_DISPLAY="$disp" "$@" &
          else
            echo "gnoblin-devkit: timed out waiting for virtual monitor before launching: $*" >&2
          fi
        ) &
      fi
      wait "$shell_pid"
    ' _ "$SHELL_BIN" "$DISP" "$DK/shell.log" "${CLIENT[@]}" \
      > >(tee "$DK/shell.log" "$LAST_LOG") 2>&1 &
    SHELL_PID=$!
    wait "$SHELL_PID"
    rc=$?
    SHELL_PID=
    exit "$rc"
    ;;

  *)
    echo "usage: $0 [verify|visible|shot] [-- CLIENT…]" >&2
    exit 2
    ;;
esac
