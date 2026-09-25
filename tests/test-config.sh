#!/usr/bin/env bash
# Native config tests need no display or full Mutter build.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
lua_pc=''
for candidate in lua lua5.4 lua-5.4 lua54; do
    if pkg-config --exists "$candidate >= 5.4"; then
        lua_pc=$candidate
        break
    fi
done
if [ -z "$lua_pc" ]; then
    echo 'Lua 5.4 development files are required.' >&2
    exit 1
fi
CFLAGS="$(pkg-config --cflags --libs glib-2.0 "$lua_pc")"
BIN="$(mktemp -d /tmp/gnoblin-cfg.XXXXXX)"
trap 'rm -rf "$BIN"' EXIT

tests=(lua-config lua-api glob-config lua-console)
if (($#)); then
    tests=("$@")
fi
for test in "${tests[@]}"; do
    case "$test" in
        lua-config | lua-api | glob-config | lua-console) ;;
        *)
            echo "Unknown config test: $test" >&2
            exit 2
            ;;
    esac
done

sources=(
    "$ROOT/src/config/gnoblin-config.c"
    "$ROOT/src/config/gnoblin-lua.c"
    "$ROOT/src/config/gnoblin-glob.c"
    "$ROOT/src/config/gnoblin-toml.c"
    "$ROOT/src/config/tomlc99/toml.c"
)
for test in "${tests[@]}"; do
    cc "$ROOT/tests/$test-test.c" "${sources[@]}" \
        -I "$ROOT/src/config" $CFLAGS -o "$BIN/$test-test"
    GNOBLIN_TEST_SOURCE_ROOT="$ROOT" timeout 20 "$BIN/$test-test"
done
