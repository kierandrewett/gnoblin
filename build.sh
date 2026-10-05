#!/usr/bin/env bash
# Public entry point for the CMake/Ninja source build.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "$0")")"

blue='' green='' dim='' reset='' red='' err_reset=''
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    blue=$'\033[1;36m'
    green=$'\033[1;32m'
    dim=$'\033[2m'
    reset=$'\033[0m'
fi
if [ -t 2 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    red=$'\033[1;31m'
    err_reset=$'\033[0m'
fi

usage() {
    cat <<'HELP'
Usage: ./build.sh [--prefix DIR] [--jobs N] [--without-xwayland] [--with-vector-cursors] [--with-portal] [--verbose] [--dry-run]
       ./build.sh [--prefix DIR] --register-session
       ./build.sh [--prefix DIR] --preview [--terminal NAME]

Build Gnoblin using the pinned Mutter sources and installed development libraries.
The build does not change system packages.

  --jobs N            Parallel compilation jobs (default: 4)
  --prefix DIR        Build output directory (default: ./install)
  --without-xwayland  Omit support for X11 applications
  --with-vector-cursors  Build the optional Adwaita vector cursor theme
  --with-portal       Also build Gnoblin's GTK-based XDG portal backend
  --verbose           Stream every build command and its output
  --dry-run           Show stages without changing files
  --target NAME       Build a CMake target (default: gnoblin-session)
  --register-session  Add the standalone Gnoblin login
  --preview           Open the compositor viewer and terminal; start a sample
                      Waybar panel when Waybar is installed.
  --terminal NAME     Terminal to open with --preview (default: first available)
  --help              Show this help
HELP
}

jobs="${GNOBLIN_BUILD_JOBS:-4}"
verbose=false dry_run=false register_session=false preview=false
xwayland=true
vector_cursors=false
with_portal=false
xwayland_selected=false
target_selected=false
prefix="$PWD/install"
terminal=''
target=gnoblin-session
while [ "$#" -gt 0 ]; do
    case "$1" in
        --jobs)
            jobs="${2:?--jobs needs a number}"
            shift
            ;;
        --prefix)
            prefix="${2:?--prefix needs a directory}"
            shift
            ;;
        --without-xwayland)
            xwayland=false
            xwayland_selected=true
            ;;
        --with-vector-cursors) vector_cursors=true ;;
        --with-portal) with_portal=true ;;
        --target)
            target="${2:?--target needs a CMake target}"
            target_selected=true
            shift
            ;;
        --verbose) verbose=true ;;
        --dry-run) dry_run=true ;;
        --register-session) register_session=true ;;
        --preview) preview=true ;;
        --terminal)
            terminal="${2:?--terminal needs a name}"
            shift
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
    shift
done
[[ "$jobs" =~ ^[1-9][0-9]*$ ]] || {
    echo '--jobs needs a positive integer.' >&2
    exit 2
}
if "$preview" && "$register_session"; then
    echo '--preview and --register-session cannot be combined.' >&2
    exit 2
fi
if [ -n "$terminal" ] && ! "$preview"; then
    echo '--terminal requires --preview.' >&2
    exit 2
fi
if "$xwayland_selected" && { "$preview" || "$register_session"; }; then
    echo '--without-xwayland is a build option.' >&2
    exit 2
fi
if "$vector_cursors" && { "$preview" || "$register_session"; }; then
    echo '--with-vector-cursors is a build option.' >&2
    exit 2
fi
if "$with_portal" && { "$preview" || "$register_session"; }; then
    echo '--with-portal is a build option.' >&2
    exit 2
fi
if "$with_portal" && "$target_selected"; then
    echo '--with-portal cannot be combined with --target.' >&2
    exit 2
fi
if "$with_portal"; then
    target=standalone-session
fi
prefix="$(realpath -m -- "$prefix")"
export GNOBLIN_PREFIX="$prefix"
if [ -n "${GNOBLIN_SOURCE_MODE:-}" ]; then
    source_mode="$GNOBLIN_SOURCE_MODE"
elif [ -e .git ]; then
    source_mode=checkout
elif [ -d sources ]; then
    source_mode=release-archive
else
    echo 'This source tree has neither Git checkout metadata nor release sources.' >&2
    echo 'Download the Gnoblin source tarball or clone the repository.' >&2
    exit 1
fi
if "$register_session"; then
    if "$verbose" || "$dry_run"; then
        echo '--register-session cannot be combined with --verbose or --dry-run.' >&2
        exit 2
    fi
    if [ ! -f "$prefix/share/wayland-sessions/gnoblin.desktop" ]; then
        echo 'No source build found. Run ./build.sh first.' >&2
        exit 1
    fi
    exec ./scripts/register-session.sh "$prefix"
fi
if "$preview"; then
    if "$verbose" || "$dry_run" || [ "$target" != gnoblin-session ] || "$target_selected"; then
        echo '--preview must be used on its own, optionally with --terminal NAME.' >&2
        exit 2
    fi
    exec ./scripts/run-gnoblin-devkit.sh "$terminal"
fi
if "$dry_run"; then
    printf 'Build Gnoblin from pinned sources.\n'
    printf '  Output: %s\n' "$prefix"
    echo "  Ninja target: $target"
    echo "  Gnoblin portal backend: $with_portal"
    echo "  XWayland: $xwayland"
    echo "  Vector cursor theme: $vector_cursors"
    echo "  Development viewer: ${GNOBLIN_DEVKIT:-disabled}"
    exit 0
fi
tools=(cmake ninja meson pkg-config python3)
[ "$source_mode" = checkout ] && tools+=(git)
[ "$source_mode" = release-archive ] && tools+=(tar xz)
"$vector_cursors" && tools+=(hyprcursor-util)
for tool in "${tools[@]}"; do
    if ! command -v "$tool" >/dev/null; then
        echo "Missing $tool. See docs/install-source.md for prerequisites." >&2
        exit 1
    fi
done
if "$vector_cursors"; then
    if ! pkg-config --exists 'hyprcursor >= 0.1.13' librsvg-2.0; then
        echo 'The optional vector cursor build needs Hyprcursor >= 0.1.13 and librsvg development files. See docs/guides/cursors.md.' >&2
        exit 1
    fi
    if [ ! -d "${ADWAITA_CURSOR_FALLBACK:-/usr/share/icons/Adwaita}/cursors" ]; then
        echo 'The optional vector cursor build needs an installed Adwaita Xcursor theme. See docs/guides/cursors.md.' >&2
        exit 1
    fi
fi

export TMPDIR="$PWD/build/tmp"
mkdir -p "$TMPDIR" build/logs
log="$PWD/build/logs/build-$(date +%Y%m%d-%H%M%S)-$$.log"
: >"$log"
printf '%sBuild log:%s %s\n' "$dim" "$reset" "$log"

run_step() {
    local label="$1" status
    shift
    printf '\n%s==>%s %s\n' "$blue" "$reset" "$label"
    printf '\n== %s ==\n' "$label" >>"$log"
    set +e
    if "$verbose"; then
        "$@" 2>&1 | tee -a "$log"
        status=${PIPESTATUS[0]}
    else
        "$@" 2>&1 | tee -a "$log" | awk -v blue="$blue" -v reset="$reset" '
            function preview( i, first) {
                if (!count) return
                first = count > 14 ? count - 14 : 0
                if (first) print "   … last 14 lines"
                for (i = first; i < count; i++) print "   " lines[i % 14]
                count = 0
                fflush()
            }
            function progress(done, total, bucket) {
                if (total != nested_total) {
                    nested_total = total
                    last_bucket = -1
                }
                bucket = int(done * 10 / total)
                if (bucket > last_bucket && bucket > 0) {
                    printf "   %d%% (%d/%d build steps)\n", int(done * 100 / total), done, total
                    last_bucket = bucket
                    fflush()
                }
            }
            /^\[[0-9]+\/[0-9]+\]/ {
                split(substr($1, 2, length($1) - 2), step, "/")
                done = step[1] + 0
                total = step[2] + 0
                if (!top_total) top_total = total
                if (total != top_total) {
                    progress(done, total)
                    next
                }
                preview()
                print blue $0 reset
                nested_total = 0
                fflush()
                next
            }
            /^FAILED:|: (fatal )?error:|: warning:|: ERROR:|^ninja: build stopped/ {
                print "   " $0
                fflush()
                next
            }
            { lines[count % 14] = $0; count++ }
            END { preview() }
        '
        status=${PIPESTATUS[0]}
    fi
    set -e
    return "$status"
}

fail_step() {
    printf '\n%s%s failed%s (exit %s). Full output: %s\n' \
        "$red" "$1" "$err_reset" "$2" "$log" >&2
    exit "$2"
}

if run_step 'Configure build' cmake -S . -B build/ninja -G Ninja \
    "-DGNOBLIN_PREFIX=$prefix" \
    "-DGNOBLIN_LIBDIR=${GNOBLIN_LIBDIR:-lib64}" \
    "-DGNOBLIN_BUILD_TYPE=${GNOBLIN_BUILD_TYPE:-debugoptimized}" \
    "-DGNOBLIN_DEVKIT=${GNOBLIN_DEVKIT:-disabled}" \
    "-DGNOBLIN_XWAYLAND=$xwayland" \
    "-DGNOBLIN_VECTOR_CURSORS=$vector_cursors" \
    "-DGNOBLIN_SOURCE_MODE=$source_mode" \
    "-DGNOBLIN_JOBS=$jobs"; then
    :
else
    fail_step 'Configure build' "$?"
fi
if run_step 'Build Gnoblin' cmake --build build/ninja --parallel "$jobs" --target "$target"; then
    :
else
    fail_step 'Build Gnoblin' "$?"
fi
printf '\n%sBuild complete%s in %s\n' "$green" "$reset" "$prefix"
if [ "$target" = gnoblin-session ]; then
    if [ -x "$prefix/libexec/xdg-desktop-portal-gnoblin" ]; then
        printf 'Portal: an existing Gnoblin backend was kept in this prefix and was not rebuilt.\n'
    else
        printf 'Portal: using the portal frontend and backend installed on your system. Build Gnoblin GTK backend with ./build.sh --with-portal.\n'
    fi
fi
if [ "$prefix" = "$PWD/install" ]; then
    printf 'Try it: ./build.sh --preview\n'
    printf 'Add it to your login screen: ./build.sh --register-session\n'
else
    printf 'Try it: ./build.sh --prefix %q --preview\n' "$prefix"
    printf 'Add it to your login screen: ./build.sh --prefix %q --register-session\n' "$prefix"
fi
printf 'Full output: %s\n' "$log"
