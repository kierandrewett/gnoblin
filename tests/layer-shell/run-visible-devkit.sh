#!/usr/bin/env bash
# Regression for the exact `just devkit` startup path: Mutter Devkit viewer,
# portal negotiation, layer-shell autostart, live log publication, and cleanup
# after an interrupted visible run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNS="${VISIBLE_DEVKIT_RUNS:-3}"
TIMEOUT="${VISIBLE_DEVKIT_TIMEOUT:-45s}"
VISIBLE_INPUT_CYCLES="${VISIBLE_INPUT_CYCLES:-4}"
VISIBLE_INPUT_IDLE_SECONDS="${VISIBLE_INPUT_IDLE_SECONDS:-6}"
LAST_LOG=/tmp/gnoblin-shell-last.log

if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  echo "SKIP: visible devkit smoke needs a host Wayland/X session"
  exit 0
fi
command -v rg >/dev/null 2>&1 || {
  echo "SKIP: visible devkit smoke needs ripgrep"
  exit 0
}

bad_log_pattern="autostart client 'gnoblin-[^']+' exited|keeps exiting right after launch|status 139|cannot attach a buffer before ack_configure|Protocol error 0 on object zwlr_layer_surface|[[:alnum:]_-]+-CRITICAL|runtime check failed|no monitors available|gnoblin-devkit: timed out waiting for virtual monitor|fuse init failed|Document portal fuse mount point unknown|Failed to execute program org\\.a11y\\.Bus|Failed to create secret proxy|Failed to get application states|Could not get window list|Choosing .*\\.portal .* deprecated UseIn key|The preferred method to match portal implementations|Timeout was reached|Permission denied|Lost connection to Wayland compositor|Error reading events from display|Broken pipe"

display_processes() {
  local display="$1"
  for proc in /proc/[0-9]*; do
    local pid="${proc##*/}"
    [ "$pid" = "$$" ] && continue
    local cmd env
    cmd="$({ tr '\0' ' ' < "$proc/cmdline"; } 2>/dev/null || true)"
    env="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    if printf '%s\n' "$cmd" | grep -Fq -- "--wayland-display $display" ||
       printf '%s\n' "$env" | grep -Fxq -- "WAYLAND_DISPLAY=$display"; then
      printf '%s %s\n' "$pid" "$cmd"
    fi
  done | sort -n
}

display_temp_dirs() {
  local display="$1"
  for dir in /tmp/gnoblin-devkit.*; do
    [ -f "$dir/shell.log" ] || continue
    if grep -Fq "$display" "$dir/shell.log" 2>/dev/null; then
      printf '%s\n' "$dir"
    fi
  done | sort
}

assert_default_wallpaper_config() {
  local display="$1"
  local expected="${GNOBLIN_WALLPAPER:-$HOME/Documents/wallpaper_light.jpg}"
  [ -f "$expected" ] || return 0

  local dirs dir config
  dirs="$(display_temp_dirs "$display")"
  if [ -z "$dirs" ]; then
    echo "FAIL: could not find live devkit temp dir for $display to verify wallpaper config"
    return 1
  fi
  while IFS= read -r dir; do
    config="$dir/config/gnoblin/gnoblin.conf"
    [ -f "$config" ] || continue
    if grep -Fxq "wallpaper = $expected" "$config" &&
       grep -Fxq "wallpaper-style = zoom" "$config"; then
      echo "  ok  visible devkit config uses wallpaper $expected"
      return 0
    fi
  done <<< "$dirs"

  echo "FAIL: visible devkit generated config did not include wallpaper $expected"
  while IFS= read -r dir; do
    config="$dir/config/gnoblin/gnoblin.conf"
    [ -f "$config" ] && sed -n '1,80p' "$config"
  done <<< "$dirs"
  return 1
}

assert_default_topbar_config() {
  local display="$1"
  local dirs dir config
  dirs="$(display_temp_dirs "$display")"
  if [ -z "$dirs" ]; then
    echo "FAIL: could not find live devkit temp dir for $display to verify topbar config"
    return 1
  fi
  while IFS= read -r dir; do
    config="$dir/config/gnoblin/gnoblin.conf"
    [ -f "$config" ] || continue
    if grep -Fxq "left = workspaces, focused-app, appmenu, spring" "$config" &&
       grep -Fxq "center = clock" "$config" &&
       grep -Fxq "right = launcher, tray, status" "$config" &&
       grep -Fxq "appmenu-backend = auto" "$config"; then
      echo "  ok  visible devkit config keeps workspaces/focused-app/appmenu and launcher"
      return 0
    fi
  done <<< "$dirs"

  echo "FAIL: visible devkit generated config did not include the default topbar layout"
  while IFS= read -r dir; do
    config="$dir/config/gnoblin/gnoblin.conf"
    [ -f "$config" ] && sed -n '1,110p' "$config"
  done <<< "$dirs"
  return 1
}

assert_default_wallpaper_rendered() {
  local display="$1"
  local expected="${GNOBLIN_WALLPAPER:-$HOME/Documents/wallpaper_light.jpg}"
  [ -f "$expected" ] || return 0
  command -v grim >/dev/null 2>&1 || return 0
  python3 -c "from PIL import Image" >/dev/null 2>&1 || return 0

  local shot
  shot="$(mktemp /tmp/gnoblin-visible-wallpaper.XXXXXX.png)"
  local deadline=$((SECONDS + 12))
  local matched=0
  while [ "$SECONDS" -lt "$deadline" ]; do
    if WAYLAND_DISPLAY="$display" grim "$shot" >/dev/null 2>&1 &&
       python3 - "$expected" "$shot" <<'PY'
import math
import sys
from PIL import Image

wallpaper = Image.open(sys.argv[1]).convert("RGB")
shot = Image.open(sys.argv[2]).convert("RGB")
out_w, out_h = shot.size
iw, ih = wallpaper.size
if out_w <= 0 or out_h <= 0 or iw <= 0 or ih <= 0:
    sys.exit(1)

scale = max(out_w / iw, out_h / ih)
sw = max(1, math.ceil(iw * scale))
sh = max(1, math.ceil(ih * scale))
resized = wallpaper.resize((sw, sh), Image.Resampling.BILINEAR)
# Rust integer division truncates toward zero for the crop offset.
offset_x = int((out_w - sw) / 2)
offset_y = int((out_h - sh) / 2)
rx = min(max(out_w // 2 - offset_x, 0), sw - 1)
ry = min(max(out_h // 2 - offset_y, 0), sh - 1)

def avg(im, x, y):
    total = [0, 0, 0]
    count = 0
    for yy in range(max(0, y - 2), min(im.size[1], y + 3)):
        for xx in range(max(0, x - 2), min(im.size[0], x + 3)):
            p = im.getpixel((xx, yy))
            total[0] += p[0]
            total[1] += p[1]
            total[2] += p[2]
            count += 1
    return tuple(round(v / count) for v in total)

expected = avg(resized, rx, ry)
actual = avg(shot, out_w // 2, out_h // 2)
if max(abs(a - b) for a, b in zip(actual, expected)) > 64:
    print(f"expected centre wallpaper pixel around {expected}, got {actual}")
    sys.exit(1)
print(f"  ok  visible devkit rendered wallpaper centre pixel {actual}")
PY
    then
      matched=1
      break
    fi
    sleep 0.5
  done
  rm -f "$shot"

  if [ "$matched" -ne 1 ]; then
    echo "FAIL: visible devkit did not render the configured wallpaper at screen centre"
    return 1
  fi
}

RUNNER_PID=
RUN_LOG=
SHORT_MARKER=
DESKTOP_MARKER=
INPUT_MARKER=
FIREFOX_MARKER=
cleanup_runner() {
  if [ -n "${RUNNER_PID:-}" ] && kill -0 "$RUNNER_PID" 2>/dev/null; then
    kill "$RUNNER_PID" 2>/dev/null || true
    wait "$RUNNER_PID" 2>/dev/null || true
  fi
  [ -n "${RUN_LOG:-}" ] && rm -f "$RUN_LOG"
  [ -n "${SHORT_MARKER:-}" ] && rm -f "$SHORT_MARKER"
  [ -n "${DESKTOP_MARKER:-}" ] && rm -f "$DESKTOP_MARKER"
  [ -n "${INPUT_MARKER:-}" ] && rm -f "$INPUT_MARKER"
  [ -n "${FIREFOX_MARKER:-}" ] && rm -f "$FIREFOX_MARKER"
  true
}
trap cleanup_runner EXIT

timeout_seconds() {
  case "$TIMEOUT" in
    *s) printf '%s\n' "${TIMEOUT%s}" ;;
    *) printf '%s\n' "$TIMEOUT" ;;
  esac
}

wait_for_line() {
  local pattern="$1"
  local label="$2"
  local seconds
  seconds="$(timeout_seconds)"
  local deadline=$((SECONDS + seconds))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if rg -q "$bad_log_pattern" "$RUN_LOG"; then
      echo "FAIL: visible devkit log contains a known-bad startup signature while waiting for $label"
      rg -n "$bad_log_pattern" "$RUN_LOG" || true
      return 1
    fi
    if rg -q "$pattern" "$RUN_LOG"; then
      return 0
    fi
    if ! kill -0 "$RUNNER_PID" 2>/dev/null; then
      echo "FAIL: visible devkit exited before $label"
      cat "$RUN_LOG"
      return 1
    fi
    sleep 0.25
  done
  echo "FAIL: timed out waiting for $label"
  cat "$RUN_LOG"
  return 1
}

echo "== visible devkit smoke: startup log, layer-shell clients, cleanup =="
for i in $(seq 1 "$RUNS"); do
  echo "-- visible run $i/$RUNS"
  RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
  "$ROOT/scripts/run-devkit.sh" visible >"$RUN_LOG" 2>&1 &
  RUNNER_PID=$!

  wait_for_line "^   nested display: " "nested display"
  display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
  if [ -z "$display" ]; then
    echo "FAIL: visible devkit did not print its nested display"
    cat "$RUN_LOG"
    exit 1
  fi
  wait_for_line "Added virtual monitor" "virtual monitor"
  assert_default_wallpaper_config "$display"
  assert_default_topbar_config "$display"

  sleep 2
  live="$(display_processes "$display")"
  missing=()
  for needle in gnoblin-topbar gnoblin-dock gnoblin-notifyd gnoblin-wallpaper; do
    if printf '%s\n' "$live" | grep -Fq "$needle"; then
      echo "  ok  $needle is alive on $display"
    else
      missing+=("$needle")
    fi
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "FAIL: visible devkit did not keep autostart clients alive: ${missing[*]}"
    printf '%s\n' "$live"
    cat "$RUN_LOG"
    exit 1
  fi
  assert_default_wallpaper_rendered "$display"

  kill "$RUNNER_PID" 2>/dev/null || true
  wait "$RUNNER_PID" 2>/dev/null || true
  RUNNER_PID=
  cat "$RUN_LOG"

  if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: $LAST_LOG was not updated for $display"
    exit 1
  fi
  if ! grep -Fq "Added virtual monitor" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: visible devkit did not materialize a virtual monitor"
    exit 1
  fi
  if rg -n "$bad_log_pattern" "$LAST_LOG"; then
    echo "FAIL: visible devkit log contains a known-bad startup/teardown signature"
    exit 1
  fi

  sleep 5
  leftovers="$(display_processes "$display")"
  if [ -n "$leftovers" ]; then
    echo "FAIL: visible devkit left processes for $display:"
    printf '%s\n' "$leftovers"
    exit 1
  fi
  leaked_dirs="$(display_temp_dirs "$display")"
  if [ -n "$leaked_dirs" ]; then
    echo "FAIL: visible devkit left temp dirs for $display:"
    printf '%s\n' "$leaked_dirs"
    exit 1
  fi
  rm -f "$RUN_LOG"
  RUN_LOG=
done

if command -v foot >/dev/null 2>&1; then
  echo "-- visible command path: foot"
  RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
  "$ROOT/scripts/run-devkit.sh" visible foot >"$RUN_LOG" 2>&1 &
  RUNNER_PID=$!

  wait_for_line "^   nested display: " "nested display"
  display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
  if [ -z "$display" ]; then
    echo "FAIL: visible devkit command run did not print its nested display"
    cat "$RUN_LOG"
    exit 1
  fi
  wait_for_line "\\+ running: foot" "requested command announcement"
  wait_for_line "Added virtual monitor" "virtual monitor"

  sleep 2
  live="$(display_processes "$display")"
  if ! printf '%s\n' "$live" | grep -Fq "foot"; then
    echo "FAIL: visible devkit command did not keep foot alive on $display"
    printf '%s\n' "$live"
    cat "$RUN_LOG"
    exit 1
  fi
  echo "  ok  foot is alive on $display"

  kill "$RUNNER_PID" 2>/dev/null || true
  wait "$RUNNER_PID" 2>/dev/null || true
  RUNNER_PID=
  cat "$RUN_LOG"

  if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: $LAST_LOG was not updated for command display $display"
    exit 1
  fi
  if rg -n "$bad_log_pattern" "$LAST_LOG"; then
    echo "FAIL: visible devkit command log contains a known-bad signature"
    exit 1
  fi

  sleep 5
  leftovers="$(display_processes "$display")"
  if [ -n "$leftovers" ]; then
    echo "FAIL: visible devkit command left processes for $display:"
    printf '%s\n' "$leftovers"
    exit 1
  fi
  leaked_dirs="$(display_temp_dirs "$display")"
  if [ -n "$leaked_dirs" ]; then
    echo "FAIL: visible devkit command left temp dirs for $display:"
    printf '%s\n' "$leaked_dirs"
    exit 1
  fi
  rm -f "$RUN_LOG"
  RUN_LOG=

  echo "-- visible desktop activation path: dock launch hook"
  RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
  DESKTOP_MARKER="$(mktemp /tmp/gnoblin-visible-desktop-activation.XXXXXX.marker)"
  rm -f "$DESKTOP_MARKER"
  CLIENTS=0 "$ROOT/scripts/run-devkit.sh" visible sh -c \
    'mkdir -p "$XDG_DATA_HOME/applications"
     printf "%s\n" \
       "[Desktop Entry]" \
       "Type=Application" \
       "Name=Foot" \
       "Exec=foot" \
       "Icon=foot" \
       "Terminal=false" \
       > "$XDG_DATA_HOME/applications/foot.desktop"
     GNOBLIN_DOCK_LAUNCH=foot gnoblin-dock &
     dock_pid=$!
     deadline=$((SECONDS + 20))
     while [ "$SECONDS" -lt "$deadline" ]; do
       if gdbus call --session --dest dev.gnoblin.Shell \
            --object-path /dev/gnoblin/Shell \
            --method dev.gnoblin.Shell.ListWindows 2>/dev/null |
          grep -Fq "'\''foot'\''"; then
         printf ok > '"$DESKTOP_MARKER"'
         wait "$dock_pid"
         exit $?
       fi
       kill -0 "$dock_pid" 2>/dev/null || exit 1
       sleep 0.25
     done
     echo "visible desktop activation: foot never appeared in ListWindows" >&2
     exit 1' >"$RUN_LOG" 2>&1 &
  RUNNER_PID=$!

  wait_for_line "^   nested display: " "nested display"
  display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
  if [ -z "$display" ]; then
    echo "FAIL: visible desktop activation run did not print its nested display"
    cat "$RUN_LOG"
    exit 1
  fi
  wait_for_line "\\+ running: sh -c" "requested desktop activation command announcement"
  wait_for_line "Added virtual monitor" "virtual monitor"

  deadline=$((SECONDS + $(timeout_seconds)))
  while [ "$SECONDS" -lt "$deadline" ] && [ ! -f "$DESKTOP_MARKER" ]; do
    if rg -q "$bad_log_pattern" "$RUN_LOG"; then
      echo "FAIL: visible desktop activation log contains a known-bad signature"
      rg -n "$bad_log_pattern" "$RUN_LOG" || true
      exit 1
    fi
    live="$(display_processes "$display")"
    if ! kill -0 "$RUNNER_PID" 2>/dev/null; then
      echo "FAIL: visible desktop activation run exited before launching foot"
      cat "$RUN_LOG"
      exit 1
    fi
    sleep 0.25
  done
  if [ "$(cat "$DESKTOP_MARKER" 2>/dev/null || true)" != "ok" ]; then
    echo "FAIL: visible desktop activation did not map a foot toplevel on $display"
    printf '%s\n' "$live"
    cat "$RUN_LOG"
    exit 1
  fi
  echo "  ok  dock desktop activation mapped foot on $display"

  kill "$RUNNER_PID" 2>/dev/null || true
  wait "$RUNNER_PID" 2>/dev/null || true
  RUNNER_PID=
  cat "$RUN_LOG"

  if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: $LAST_LOG was not updated for desktop activation display $display"
    exit 1
  fi
  if rg -n "$bad_log_pattern" "$LAST_LOG"; then
    echo "FAIL: visible desktop activation log contains a known-bad signature"
    exit 1
  fi

  sleep 5
  leftovers="$(display_processes "$display")"
  if [ -n "$leftovers" ]; then
    echo "FAIL: visible desktop activation left processes for $display:"
    printf '%s\n' "$leftovers"
    exit 1
  fi
  leaked_dirs="$(display_temp_dirs "$display")"
  if [ -n "$leaked_dirs" ]; then
    echo "FAIL: visible desktop activation left temp dirs for $display:"
    printf '%s\n' "$leaked_dirs"
    exit 1
  fi
  rm -f "$RUN_LOG" "$DESKTOP_MARKER"
  RUN_LOG=
  DESKTOP_MARKER=

  echo "-- visible keyboard input path: launcher activation"
  RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
  INPUT_MARKER="$(mktemp /tmp/gnoblin-visible-input.XXXXXX.marker)"
  rm -f "$INPUT_MARKER"
  GNOBLIN_VISIBLE_INPUT_MARKER="$INPUT_MARKER" \
  GNOBLIN_VISIBLE_INPUT_CYCLES="$VISIBLE_INPUT_CYCLES" \
  GNOBLIN_VISIBLE_INPUT_IDLE_SECONDS="$VISIBLE_INPUT_IDLE_SECONDS" \
  CLIENTS=0 \
    "$ROOT/scripts/run-devkit.sh" visible python3 "$ROOT/tests/layer-shell/visible-input-probe.py" \
    >"$RUN_LOG" 2>&1 &
  RUNNER_PID=$!

  wait_for_line "^   nested display: " "nested display"
  display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
  if [ -z "$display" ]; then
    echo "FAIL: visible input run did not print its nested display"
    cat "$RUN_LOG"
    exit 1
  fi
  wait_for_line "\\+ running: python3" "requested visible input probe announcement"
  wait_for_line "Added virtual monitor" "virtual monitor"

  deadline=$((SECONDS + $(timeout_seconds)))
  while [ "$SECONDS" -lt "$deadline" ] && [ ! -f "$INPUT_MARKER" ]; do
    if rg -q "$bad_log_pattern" "$RUN_LOG"; then
      echo "FAIL: visible input log contains a known-bad signature"
      rg -n "$bad_log_pattern" "$RUN_LOG" || true
      exit 1
    fi
    if ! kill -0 "$RUNNER_PID" 2>/dev/null; then
      echo "FAIL: visible input run exited before launcher activation completed"
      cat "$RUN_LOG"
      exit 1
    fi
    sleep 0.25
  done
  if [ "$(cat "$INPUT_MARKER" 2>/dev/null || true)" != "ok" ]; then
    echo "FAIL: visible input did not map foot through launcher keyboard activation on $display"
    cat "$RUN_LOG"
    exit 1
  fi
  echo "  ok  visible keyboard input mapped foot via launcher on $display after ${VISIBLE_INPUT_CYCLES} idle cycle(s)"

  kill "$RUNNER_PID" 2>/dev/null || true
  wait "$RUNNER_PID" 2>/dev/null || true
  RUNNER_PID=
  cat "$RUN_LOG"

  if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: $LAST_LOG was not updated for visible input display $display"
    exit 1
  fi
  if rg -n "$bad_log_pattern" "$LAST_LOG"; then
    echo "FAIL: visible input log contains a known-bad signature"
    exit 1
  fi

  sleep 5
  leftovers="$(display_processes "$display")"
  if [ -n "$leftovers" ]; then
    echo "FAIL: visible input left processes for $display:"
    printf '%s\n' "$leftovers"
    exit 1
  fi
  leaked_dirs="$(display_temp_dirs "$display")"
  if [ -n "$leaked_dirs" ]; then
    echo "FAIL: visible input left temp dirs for $display:"
    printf '%s\n' "$leaked_dirs"
    exit 1
  fi
  rm -f "$RUN_LOG" "$INPUT_MARKER"
  RUN_LOG=
  INPUT_MARKER=
else
  echo "-- visible command path: SKIP no foot"
fi

if command -v firefox >/dev/null 2>&1; then
  echo "-- visible command path: firefox"
  RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
  FIREFOX_MARKER="$(mktemp /tmp/gnoblin-visible-firefox.XXXXXX.marker)"
  rm -f "$FIREFOX_MARKER"
  CLIENTS=0 "$ROOT/scripts/run-devkit.sh" visible sh -c \
    'profile="$XDG_CACHE_HOME/firefox-visible-profile"
     mkdir -p "$profile"
     MOZ_ENABLE_WAYLAND=1 MOZ_DBUS_REMOTE=0 firefox --new-instance --profile "$profile" about:blank &
     firefox_pid=$!
     deadline=$((SECONDS + 35))
     while [ "$SECONDS" -lt "$deadline" ]; do
       if gdbus call --session --dest dev.gnoblin.Shell \
            --object-path /dev/gnoblin/Shell \
            --method dev.gnoblin.Shell.ListWindows 2>/dev/null |
          grep -Eqi "firefox|mozilla"; then
         printf ok > '"$FIREFOX_MARKER"'
         wait "$firefox_pid"
         exit $?
       fi
       kill -0 "$firefox_pid" 2>/dev/null || exit 1
       sleep 0.5
     done
     echo "visible firefox: Firefox never appeared in ListWindows" >&2
     exit 1' >"$RUN_LOG" 2>&1 &
  RUNNER_PID=$!

  wait_for_line "^   nested display: " "nested display"
  display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
  if [ -z "$display" ]; then
    echo "FAIL: visible Firefox run did not print its nested display"
    cat "$RUN_LOG"
    exit 1
  fi
  wait_for_line "\\+ running: sh -c" "requested Firefox command announcement"
  wait_for_line "Added virtual monitor" "virtual monitor"

  deadline=$((SECONDS + 45))
  while [ "$SECONDS" -lt "$deadline" ] && [ ! -f "$FIREFOX_MARKER" ]; do
    if rg -q "$bad_log_pattern" "$RUN_LOG"; then
      echo "FAIL: visible Firefox log contains a known-bad signature"
      rg -n "$bad_log_pattern" "$RUN_LOG" || true
      exit 1
    fi
    if ! kill -0 "$RUNNER_PID" 2>/dev/null; then
      echo "FAIL: visible Firefox run exited before Firefox mapped"
      cat "$RUN_LOG"
      exit 1
    fi
    sleep 0.5
  done
  if [ "$(cat "$FIREFOX_MARKER" 2>/dev/null || true)" != "ok" ]; then
    echo "FAIL: visible Firefox did not map a toplevel on $display"
    live="$(display_processes "$display")"
    printf '%s\n' "$live"
    cat "$RUN_LOG"
    exit 1
  fi
  echo "  ok  Firefox mapped inside visible devkit on $display"

  kill "$RUNNER_PID" 2>/dev/null || true
  wait "$RUNNER_PID" 2>/dev/null || true
  RUNNER_PID=
  cat "$RUN_LOG"

  if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
    echo "FAIL: $LAST_LOG was not updated for Firefox display $display"
    exit 1
  fi
  if rg -n "$bad_log_pattern" "$LAST_LOG"; then
    echo "FAIL: visible Firefox log contains a known-bad signature"
    exit 1
  fi

  sleep 5
  leftovers="$(display_processes "$display")"
  if [ -n "$leftovers" ]; then
    echo "FAIL: visible Firefox left processes for $display:"
    printf '%s\n' "$leftovers"
    exit 1
  fi
  leaked_dirs="$(display_temp_dirs "$display")"
  if [ -n "$leaked_dirs" ]; then
    echo "FAIL: visible Firefox left temp dirs for $display:"
    printf '%s\n' "$leaked_dirs"
    exit 1
  fi
  rm -f "$RUN_LOG" "$FIREFOX_MARKER"
  RUN_LOG=
  FIREFOX_MARKER=
else
  echo "-- visible command path: SKIP no firefox"
fi

echo "-- visible short-lived command path"
RUN_LOG="$(mktemp /tmp/gnoblin-visible-devkit.XXXXXX.log)"
SHORT_MARKER="$(mktemp /tmp/gnoblin-visible-short-command.XXXXXX.marker)"
rm -f "$SHORT_MARKER"
"$ROOT/scripts/run-devkit.sh" visible sh -c "printf ok > '$SHORT_MARKER'; exit 0" >"$RUN_LOG" 2>&1 &
RUNNER_PID=$!

wait_for_line "^   nested display: " "nested display"
display="$(sed -n 's/^   nested display: //p' "$RUN_LOG" | tail -n 1)"
if [ -z "$display" ]; then
  echo "FAIL: visible devkit short-lived command run did not print its nested display"
  cat "$RUN_LOG"
  exit 1
fi
wait_for_line "\\+ running: sh -c" "requested short-lived command announcement"
wait_for_line "Added virtual monitor" "virtual monitor"

deadline=$((SECONDS + $(timeout_seconds)))
while [ "$SECONDS" -lt "$deadline" ] && [ ! -f "$SHORT_MARKER" ]; do
  if rg -q "$bad_log_pattern" "$RUN_LOG"; then
    echo "FAIL: visible short-lived command log contains a known-bad signature"
    rg -n "$bad_log_pattern" "$RUN_LOG" || true
    exit 1
  fi
  sleep 0.25
done
if [ "$(cat "$SHORT_MARKER" 2>/dev/null || true)" != "ok" ]; then
  echo "FAIL: visible short-lived command did not run after monitor creation"
  cat "$RUN_LOG"
  exit 1
fi
echo "  ok  short-lived command ran on $display"

kill "$RUNNER_PID" 2>/dev/null || true
wait "$RUNNER_PID" 2>/dev/null || true
RUNNER_PID=
cat "$RUN_LOG"

if ! grep -Fq "$display" "$LAST_LOG" 2>/dev/null; then
  echo "FAIL: $LAST_LOG was not updated for short-lived command display $display"
  exit 1
fi
if rg -n "$bad_log_pattern" "$LAST_LOG"; then
  echo "FAIL: visible short-lived command log contains a known-bad signature"
  exit 1
fi

sleep 5
leftovers="$(display_processes "$display")"
if [ -n "$leftovers" ]; then
  echo "FAIL: visible short-lived command left processes for $display:"
  printf '%s\n' "$leftovers"
  exit 1
fi
leaked_dirs="$(display_temp_dirs "$display")"
if [ -n "$leaked_dirs" ]; then
  echo "FAIL: visible short-lived command left temp dirs for $display:"
  printf '%s\n' "$leaked_dirs"
  exit 1
fi
rm -f "$RUN_LOG" "$SHORT_MARKER"
RUN_LOG=
SHORT_MARKER=

echo "PASS: visible devkit startup/log/cleanup smoke passed ($RUNS runs + command paths + desktop activation)"
