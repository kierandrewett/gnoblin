#!/usr/bin/env bash
# Run a command with a staged build visible at the path it was built for.
#
# usage: run-staged.sh PREFIX STAGE_ROOT COMMAND [ARGUMENT...]
#
# make builds for an install prefix such as /usr/local/lib/gnoblin and writes the files below a stage root. The binaries
# have the prefix compiled in, so they cannot run from the stage until they are installed. This script gives the command
# a private mount namespace in which the staged files appear at the prefix. Nothing changes on the host, and no root
# access is needed. An overlay shows the staged build and keeps everything else in the directory above the prefix, and
# it makes the prefix and any missing parent directories inside the namespace only.
set -euo pipefail

PREFIX="${1:?usage: run-staged.sh PREFIX STAGE_ROOT COMMAND [ARGUMENT...]}"
STAGE="${2:?usage: run-staged.sh PREFIX STAGE_ROOT COMMAND [ARGUMENT...]}"
shift 2
[ "$#" -gt 0 ] || {
    echo 'run-staged: no command given.' >&2
    exit 2
}
if [ ! -d "$STAGE$PREFIX" ]; then
    echo "run-staged: no staged build in $STAGE$PREFIX. Run make first." >&2
    exit 1
fi
command -v unshare >/dev/null 2>&1 || {
    echo 'run-staged: unshare is required.' >&2
    exit 1
}

# The user is root inside the namespace, so the mounts are allowed. The kernel still sees the real user.
# shellcheck disable=SC2016  # the script text is for the inner shell, which expands it
exec unshare --user --map-root-user --mount bash -c '
set -euo pipefail
prefix="$1" stage="$2"
shift 2
# The nearest directory that exists is where the overlay goes. The root directory is too wide for that.
base="$(dirname "$prefix")"
while [ ! -d "$base" ]; do
    base="$(dirname "$base")"
done
if [ "$base" = / ]; then
    echo "run-staged: no directory above $prefix exists except the root. Create its parent first." >&2
    exit 1
fi
# The overlay work files must not be inside the directory it covers.
work=""
for root in "${XDG_RUNTIME_DIR:-}" /dev/shm /var/tmp /tmp; do
    if [ -n "$root" ] && [ -d "$root" ] && [ -w "$root" ] && [[ "$root/" != "$base"/* ]]; then
        work="$(mktemp -d -p "$root")"
        break
    fi
done
if [ -z "$work" ]; then
    echo "run-staged: no writable scratch directory outside $base." >&2
    exit 1
fi
trap '"'"'rm -rf -- "$work"'"'"' EXIT
mkdir "$work/upper" "$work/work"
mount -t overlay overlay -o "lowerdir=$base,upperdir=$work/upper,workdir=$work/work" "$base"
mkdir -p "$prefix"
mount --bind "$stage$prefix" "$prefix"
"$@"' bash "$PREFIX" "$STAGE" "$@"
