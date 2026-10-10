#!/usr/bin/env bash
# The build's system layout step gives every public file the package recipes list.
#
# usage: tests/system-layout.test.sh [BUILT_PREFIX]
#        tests/system-layout.test.sh --stage STAGE_ROOT
#
# With BUILT_PREFIX, a private prefix made by ./build.sh (default: ./install), the test copies it into a scratch stage
# root as /usr/lib/gnoblin and runs cmake/system-layout.cmake on it. With --stage, it checks a stage root that
# ./build.sh --layout system --destdir STAGE_ROOT made, so it tests the real build output. Either way it checks:
#   - every absolute /usr and /etc path in the %files sections of the RPM specs exists in the stage root;
#   - the command links, the session file, the polkit action and the schema cache are right;
#   - a second run changes nothing.
# It writes its result to build/logs/system-layout-<time>.txt as a record of the run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRIVATE=/usr/lib/gnoblin
STAGED_BY_BUILD=false
if [ "${1:-}" = --stage ]; then
    STAGED_BY_BUILD=true
    STAGE="$(cd "${2:?--stage needs a stage root}" && pwd)"
    BUILT="$STAGE$PRIVATE"
else
    BUILT="${1:-$ROOT/install}"
    STAGE=""
fi
LOG_DIR="$ROOT/build/logs"
mkdir -p "$LOG_DIR"
LOG="$LOG_DIR/system-layout-$(date -u +%Y%m%dT%H%M%SZ).txt"
fail=0

if [ ! -x "$BUILT/bin/gnoblin" ]; then
    echo "No built prefix at $BUILT. Run ./build.sh first, or pass the prefix." >&2
    exit 2
fi

if ! "$STAGED_BY_BUILD"; then
    STAGE="$(mktemp -d "${TMPDIR:-/tmp}/gnoblin-layout.XXXXXX")"
fi
cleanup() {
    if ! "$STAGED_BY_BUILD" && [ -n "$STAGE" ] && [ -d "$STAGE" ]; then
        rm -rf -- "$STAGE"
    fi
}
trap cleanup EXIT

check() {
    # check NAME ACTUAL EXPECTED
    if [ "$2" = "$3" ]; then
        echo "PASS $1"
    else
        echo "FAIL $1 (got: ${2:-nothing}, expected: $3)"
        fail=$((fail + 1))
    fi
}

{
    echo "command: tests/system-layout.test.sh $BUILT"
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "commit: $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

    if ! "$STAGED_BY_BUILD"; then
        mkdir -p "$STAGE$PRIVATE"
        cp -a "$BUILT/." "$STAGE$PRIVATE/"
        GNOBLIN_LAYOUT_GEOCLUE=1 GNOBLIN_STAGE_ROOT="$STAGE" cmake -DGNOBLIN_PRIVATE_PREFIX="$PRIVATE" -DGNOBLIN_SYSTEM_PREFIX=/usr -P "$ROOT/cmake/system-layout.cmake" >/dev/null
    fi

    # The recipes' own file lists are the contract. Take every literal /usr and /etc path from their %files sections.
    expected="$(awk '/^%files/ {in_files = 1; next} /^%(package|description|changelog|posttrans|postun|check)/ {in_files = 0}
        in_files && $NF ~ /^\/(usr|etc)\// && $NF !~ /%/ {print $NF}' \
        "$ROOT/packaging/rpm/gnoblin.spec.in" "$ROOT/packaging/rpm/xdg-desktop-portal-gnoblin.spec" | sort -u)"
    # Only the Fedora recipe asks for the GeoClue file. A stage made without GNOBLIN_LAYOUT_GEOCLUE=1 has none.
    if "$STAGED_BY_BUILD" && [ "${GNOBLIN_LAYOUT_GEOCLUE:-0}" != 1 ]; then
        expected="$(grep -v '^/etc/geoclue/' <<<"$expected" || true)"
    fi
    check "the RPM specs list public paths" "$([ -n "$expected" ] && echo yes)" "yes"
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        # A path with a * is a glob: the package compresses man pages, so its file list ends in .1*.
        if [[ $path == *"*"* ]]; then
            check "the layout provides $path" "$(compgen -G "$STAGE$path" >/dev/null && echo yes)" "yes"
        else
            check "the layout provides $path" "$([ -e "$STAGE$path" ] || [ -L "$STAGE$path" ] && echo yes)" "yes"
        fi
    done <<<"$expected"

    check "/usr/bin/gnoblin links to the private runtime" "$(readlink "$STAGE/usr/bin/gnoblin")" "$PRIVATE/bin/gnoblin"
    check "/usr/bin/gnoblinctl links to the private runtime" "$(readlink "$STAGE/usr/bin/gnoblinctl")" "$PRIVATE/bin/gnoblinctl"
    check "the session file starts the private compositor" \
        "$(sed -n 's/^Exec=//p' "$STAGE/usr/share/wayland-sessions/gnoblin.desktop")" "$PRIVATE/bin/gnoblin"
    check "the session file names the Gnoblin desktop" \
        "$(sed -n 's/^DesktopNames=//p' "$STAGE/usr/share/wayland-sessions/gnoblin.desktop")" "Gnoblin;"
    policy="$STAGE/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
    if [ -e "$policy" ]; then
        check "the polkit action no longer carries the GNOME name" "$(grep -c 'org.gnome.mutter.backlight-helper' "$policy" || true)" "0"
    fi
    check "the package carries no compiled schema cache" \
        "$([ -e "$STAGE$PRIVATE/share/glib-2.0/schemas/gschemas.compiled" ] && echo present || echo absent)" "absent"

    if ! "$STAGED_BY_BUILD"; then
        before="$(cd "$STAGE" && find . \( -type f -o -type l \) -print0 | sort -z | xargs -0 sha256sum 2>/dev/null | sha256sum)"
        GNOBLIN_LAYOUT_GEOCLUE=1 GNOBLIN_STAGE_ROOT="$STAGE" cmake -DGNOBLIN_PRIVATE_PREFIX="$PRIVATE" -DGNOBLIN_SYSTEM_PREFIX=/usr -P "$ROOT/cmake/system-layout.cmake" >/dev/null
        after="$(cd "$STAGE" && find . \( -type f -o -type l \) -print0 | sort -z | xargs -0 sha256sum 2>/dev/null | sha256sum)"
        check "a second run changes nothing" "$after" "$before"

        # Without the GeoClue request, the file is absent. The Arch and openSUSE packages do not list it.
        PLAIN="$(mktemp -d "${TMPDIR:-/tmp}/gnoblin-layout-plain.XXXXXX")"
        mkdir -p "$PLAIN$PRIVATE"
        cp -a "$BUILT/." "$PLAIN$PRIVATE/"
        GNOBLIN_STAGE_ROOT="$PLAIN" cmake -DGNOBLIN_PRIVATE_PREFIX="$PRIVATE" -DGNOBLIN_SYSTEM_PREFIX=/usr -P "$ROOT/cmake/system-layout.cmake" >/dev/null
        check "GeoClue is left out unless the recipe asks for it" \
            "$([ -e "$PLAIN/etc/geoclue/conf.d/50-gnoblin.conf" ] && echo present || echo absent)" "absent"
        rm -rf -- "$PLAIN"
    fi

    echo "failures: $fail"
} 2>&1 | tee "$LOG"

echo "saved: $LOG"
if grep -q '^FAIL' "$LOG"; then
    exit 1
fi
