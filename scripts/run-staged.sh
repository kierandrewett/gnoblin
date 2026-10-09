#!/usr/bin/env bash
# Run a command with a staged build visible at the path it was built for.
#
# usage: run-staged.sh PREFIX STAGE_ROOT COMMAND [ARGUMENT...]
#
# make builds for an install prefix such as /usr/local/lib/gnoblin and writes the files below a stage root. The binaries
# have the prefix compiled in, so they cannot run from the stage until they are installed. This script gives the command
# a private mount namespace in which the staged files appear at the prefix. Nothing changes on the host, and no root
# access is needed. The directory above the prefix is hidden inside the namespace, so only the staged build is there.
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
PARENT="$(dirname "$PREFIX")"
case "$PARENT" in
    / | /usr)
        echo "run-staged: the prefix $PREFIX is directly below $PARENT. Use a deeper prefix." >&2
        exit 2
        ;;
esac
if [ ! -d "$PARENT" ]; then
    echo "run-staged: the directory $PARENT does not exist, so the staged build cannot be shown at $PREFIX." >&2
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
mount -t tmpfs none "$(dirname "$1")"
mkdir "$1"
mount --bind "$2$1" "$1"
shift 2
exec "$@"' bash "$PREFIX" "$STAGE" "$@"
