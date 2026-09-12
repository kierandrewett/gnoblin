#!/usr/bin/env bash
# Verify that persistent test logs are published through a private state directory
# without following an existing destination symlink.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-state-test.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

export XDG_STATE_HOME="$TMP/state"
source "$ROOT/scripts/gnoblin-state.sh"

printf 'original\n' > "$TMP/source.log"
state_dir="$(gnoblin_state_dir)"
[ "$(stat -c %a "$state_dir")" = 700 ] || {
    echo "FAIL: state directory is not mode 700" >&2
    exit 1
}

printf 'do not overwrite\n' > "$TMP/victim"
ln -s "$TMP/victim" "$state_dir/test.log"
gnoblin_publish_log "$TMP/source.log" test.log

[ ! -L "$state_dir/test.log" ] || {
    echo "FAIL: published log is still a symlink" >&2
    exit 1
}
[ "$(cat "$state_dir/test.log")" = original ] || {
    echo "FAIL: published log content is wrong" >&2
    exit 1
}
[ "$(cat "$TMP/victim")" = 'do not overwrite' ] || {
    echo "FAIL: destination symlink target was overwritten" >&2
    exit 1
}
[ "$(stat -c %a "$state_dir/test.log")" = 600 ] || {
    echo "FAIL: published log is not mode 600" >&2
    exit 1
}

echo "PASS: secure state publication"
