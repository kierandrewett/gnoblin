#!/usr/bin/env bash
# Capture the framebuffer inside a live gnoblin-shell Mutter Devkit session.
set -uo pipefail

OUT="${1:-/tmp/gnoblin-devkit-live/nested.png}"
DISPLAY_FILTER="${NESTED_DISPLAY:-}"

usage() {
    echo "usage: $0 [output.png]" >&2
    echo "       NESTED_DISPLAY=gnoblin-devkit-123 $0 [output.png]" >&2
}

find_shell() {
    local pid args display

    while read -r pid args; do
        [ -n "$pid" ] || continue
        [[ "$args" == *"gnoblin-shell"* ]] || continue
        [[ "$args" == *"--devkit"* ]] || continue
        [[ "$args" != *"dbus-run-session"* ]] || continue

        if [[ "$args" =~ --wayland-display[[:space:]]+([^[:space:]]+) ]]; then
            display="${BASH_REMATCH[1]}"
        else
            continue
        fi

        if [ -n "$DISPLAY_FILTER" ] && [ "$display" != "$DISPLAY_FILTER" ]; then
            continue
        fi

        printf '%s %s\n' "$pid" "$display"
        return 0
    done < <(ps -eo pid=,args=)

    return 1
}

capture_nested() {
    local shell_info shell_pid nested_display bus runtime_dir dir

    shell_info="$(find_shell)" || {
        echo "capture-devkit: no live nested gnoblin-shell --devkit process found" >&2
        usage
        return 1
    }

    shell_pid="${shell_info%% *}"
    nested_display="${shell_info#* }"
    bus="$(tr '\0' '\n' <"/proc/$shell_pid/environ" |
        sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p' |
        head -n1)"
    runtime_dir="$(tr '\0' '\n' <"/proc/$shell_pid/environ" |
        sed -n 's/^XDG_RUNTIME_DIR=//p' |
        head -n1)"

    if [ -z "$bus" ]; then
        echo "capture-devkit: could not read nested DBUS_SESSION_BUS_ADDRESS from pid $shell_pid" >&2
        return 1
    fi

    dir="$(dirname "$OUT")"
    mkdir -p "$dir"
    rm -f "$OUT"

    if ! command -v grim >/dev/null 2>&1; then
        echo "capture-devkit: grim is required for nested wlr-screencopy capture" >&2
        return 1
    fi

    if ! env -u DISPLAY \
        WAYLAND_DISPLAY="$nested_display" \
        XDG_RUNTIME_DIR="${runtime_dir:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}}" \
        grim "$OUT"; then
        echo "capture-devkit: grim/wlr-screencopy failed on $nested_display" >&2
        return 1
    fi

    if [ ! -s "$OUT" ]; then
        echo "capture-devkit: grim returned but did not create $OUT" >&2
        return 1
    fi

    echo "captured $nested_display with wlr-screencopy from shell pid $shell_pid: $OUT"
}

write_crops() {
    local width height stem top_crop dock_crop dock_y dock_h top_h

    command -v magick >/dev/null 2>&1 || return 0

    read -r width height < <(magick identify -format '%w %h' "$OUT")
    stem="${OUT%.png}"
    top_crop="${stem}-topbar.png"
    dock_crop="${stem}-dock.png"
    top_h=96
    dock_h=220
    dock_y=$((height > dock_h ? height - dock_h : 0))

    magick "$OUT" -crop "${width}x${top_h}+0+0" +repage "$top_crop"
    magick "$OUT" -crop "${width}x${dock_h}+0+${dock_y}" +repage "$dock_crop"

    echo "wrote crops: $top_crop $dock_crop"
}

capture_nested && write_crops
