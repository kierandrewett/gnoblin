#!/usr/bin/env bash
set -euo pipefail

gnoblinctl="${GNOBLIN_DEVKIT_CTL:-gnoblinctl}"
focus_client="${GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT:?missing input-source fixture client}"
title_a='Gnoblin per-window input A'
title_b='Gnoblin per-window input B'
window_log_a="$XDG_RUNTIME_DIR/per-window-window-a.log"
window_log_b="$XDG_RUNTIME_DIR/per-window-window-b.log"
windows_json="$XDG_RUNTIME_DIR/per-window-windows.json"
input_json="$XDG_RUNTIME_DIR/per-window-input.json"

source "$GNOBLIN_TEST_ROOT/scripts/gnoblin-test-ibus.sh"

cat >"$XDG_RUNTIME_DIR/per-window-enabled.lua" <<'LUA'
assert(gnoblin.settings.input_sources.per_window == true)
print("INPUT_SOURCE:per-window-config-enabled")
LUA
"$gnoblinctl" lua "$XDG_RUNTIME_DIR/per-window-enabled.lua"

"$focus_client" window "$title_a" >"$window_log_a" 2>&1 &
window_a_pid=$!
window_b_pid=""
cleanup_windows() {
    local result=$?
    if [[ -n $window_b_pid ]]; then
        kill -TERM "$window_a_pid" "$window_b_pid" 2>/dev/null || true
    else
        kill -TERM "$window_a_pid" 2>/dev/null || true
    fi
    wait "$window_a_pid" 2>/dev/null || true
    if [[ -n $window_b_pid ]]; then
        wait "$window_b_pid" 2>/dev/null || true
    fi
    if ((result != 0)); then
        printf '%s\n' 'Input-source fixture A log:' >&2
        cat "$window_log_a" >&2
        printf '%s\n' 'Input-source fixture B log:' >&2
        cat "$window_log_b" >&2
    fi
    return "$result"
}
trap cleanup_windows EXIT

window_matches() {
    local title="$1"
    local focused="$2"
    "$gnoblinctl" --json window list >"$windows_json" || return 1
    python3 - "$windows_json" "$title" "$focused" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    value = json.load(stream)
windows = value if isinstance(value, list) else value.get("windows", [])
expected_focus = None if sys.argv[3] == "any" else sys.argv[3] == "true"
for window in windows:
    if window.get("title") == sys.argv[2] and (
        expected_focus is None or bool(window.get("focused")) == expected_focus
    ):
        sys.exit(0)
sys.exit(1)
PY
}
wait_for_window() {
    local title="$1"
    for _ in {1..100}; do
        if window_matches "$title" any; then
            return 0
        fi
        sleep 0.05
    done
    cat "$windows_json" >&2
    echo "test window did not appear: $title" >&2
    return 1
}
wait_for_focus() {
    local title="$1"
    for _ in {1..100}; do
        if window_matches "$title" true; then
            return 0
        fi
        sleep 0.05
    done
    cat "$windows_json" >&2
    echo "test window did not receive focus: $title" >&2
    return 1
}
wait_for_source() {
    local source_type="$1"
    local source_id="$2"
    for _ in {1..100}; do
        "$gnoblinctl" --json input current >"$input_json" 2>/dev/null || true
        if python3 - "$input_json" "$source_type" "$source_id" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as stream:
        current = json.load(stream)
except (OSError, json.JSONDecodeError):
    sys.exit(1)
source = current.get("source")
if current.get("available") and source and source.get("type") == sys.argv[2] \
        and source.get("id") == sys.argv[3]:
    sys.exit(0)
sys.exit(1)
PY
        then
            return 0
        fi
        sleep 0.05
    done
    cat "$input_json" >&2
    echo "input source did not become active: $source_id" >&2
    return 1
}
select_source() {
    local source_id="$1"
    local result_file="$XDG_RUNTIME_DIR/select-$source_id.txt"
    for _ in {1..100}; do
        if "$gnoblinctl" --timeout 2 input select xkb "$source_id" >"$result_file" 2>&1; then
            return 0
        fi
        sleep 0.05
    done
    cat "$result_file" >&2
    echo "could not select XKB source: $source_id" >&2
    return 1
}
activate_window() {
    local title="$1"
    timeout 7 "$focus_client" activate "$title"
    wait_for_focus "$title"
}

wait_for_window "$title_a"
activate_window "$title_a"
select_source us
wait_for_source xkb us
"$focus_client" window "$title_b" >"$window_log_b" 2>&1 &
window_b_pid=$!
wait_for_window "$title_b"
activate_window "$title_b"
wait_for_source xkb us
printf 'INPUT_SOURCE:per-window-first-focus-inherits\n'
select_source gb
wait_for_source xkb gb
activate_window "$title_a"
wait_for_source xkb us
printf 'INPUT_SOURCE:per-window-restores-A\n'
activate_window "$title_b"
wait_for_source xkb gb
printf 'INPUT_SOURCE:per-window-restores-B\n'
gnoblin_test_ibus_select_source "$gnoblinctl" "$XDG_RUNTIME_DIR/per-window-ibus-select.txt"
wait_for_source ibus xkb:us::eng
activate_window "$title_a"
wait_for_source xkb us
printf 'INPUT_SOURCE:per-window-restores-XKB-from-IBus\n'
activate_window "$title_b"
wait_for_source ibus xkb:us::eng
printf 'INPUT_SOURCE:per-window-restores-IBus\n'
