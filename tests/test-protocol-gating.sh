#!/usr/bin/env bash
# Prove init.lua gates both a Gnoblin-owned global and the upstream background
# effect global when Gnoblin mode explicitly disables it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
mkdir -p "$ROOT/build/tmp"
DK="$(mktemp -d "$ROOT/build/tmp/protocol-gating.XXXXXX")"
trap 'rm -rf -- "$DK"' EXIT
mkdir -p "$DK/config/gnoblin"

# init.lua disabling layer shell and the background-effect global.
CONF_FILE="$DK/config/gnoblin/init.lua"
cat >"$CONF_FILE" <<'LUA'
gnoblin.configure {
    protocols = {
        wlr_layer_shell = false,
        ext_background_effect_v1 = false,
    },
}
LUA

probe="$DK/wl-globals"
compiler_flags="$(pkg-config --cflags --libs wayland-client)" || exit 1
read -r -a compiler_args <<<"$compiler_flags"
cc "$ROOT/tests/wl-globals.c" "${compiler_args[@]}" -o "$probe" || exit 1

devkit_exec=$(
    cat <<'SCRIPT'
set -euo pipefail
if "$GNOBLIN_TEST_WL_GLOBALS" zwlr_layer_shell_v1 | grep -q zwlr_layer_shell_v1; then
    echo 'FAIL: layer-shell global remained visible when disabled' >&2
    exit 1
fi
if "$GNOBLIN_TEST_WL_GLOBALS" ext_background_effect_manager_v1 \
    | grep -q ext_background_effect_manager_v1; then
    echo 'FAIL: background-effect global remained visible when disabled' >&2
    exit 1
fi
echo 'PASS: Lua config gates layer-shell and background-effect globals'
SCRIPT
)

GNOBLIN_STATE_DIR="$DK/state" \
    GNOBLIN_PREFIX="$PREFIX" \
    GNOBLIN_RUNTIME_BIN="$ROOT/build/ninja/gnoblin" \
    GNOBLIN_DEVKIT_CTL="$PREFIX/bin/gnoblinctl" \
    GNOBLIN_DEVKIT_CONFIG_SOURCE="$DK/config" \
    GNOBLIN_TEST_WL_GLOBALS="$probe" \
    GNOBLIN_DEVKIT_EXEC="$devkit_exec" \
    timeout 180 bash "$ROOT/scripts/run-gnoblin-devkit.sh"
