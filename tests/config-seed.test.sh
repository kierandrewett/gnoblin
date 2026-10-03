#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT

prefix="$tmp/prefix"
home="$tmp/home"
config_home="$tmp/xdg-config"
mkdir -p "$prefix/libexec" "$prefix/share/gnoblin" "$home"
install -m 755 "$ROOT/src/tools/gnoblin-seed-config" "$prefix/libexec/gnoblin-seed-config"
install -m 644 "$ROOT/src/data/init.lua.example" "$prefix/share/gnoblin/init.lua.example"

# The first login seeds a private Lua config with safe permissions.
env -u GNOBLIN_CONFIG HOME="$home" XDG_CONFIG_HOME="$config_home" \
    "$prefix/libexec/gnoblin-seed-config" "$prefix/share/gnoblin/init.lua.example"
cmp -s "$config_home/gnoblin/init.lua" "$prefix/share/gnoblin/init.lua.example"
[[ "$(stat -c '%a' "$config_home/gnoblin/init.lua")" == 600 ]]

# Later logins preserve local edits.
printf '%s\n' '-- user edit' >"$config_home/gnoblin/init.lua"
env -u GNOBLIN_CONFIG HOME="$home" XDG_CONFIG_HOME="$config_home" \
    "$prefix/libexec/gnoblin-seed-config" "$prefix/share/gnoblin/init.lua.example"
[[ "$(cat "$config_home/gnoblin/init.lua")" == '-- user edit' ]]

# Existing TOML configs remain selected and are not shadowed by Lua.
legacy_config="$tmp/legacy/gnoblin"
mkdir -p "$legacy_config"
printf '%s\n' 'legacy = true' >"$legacy_config/gnoblin.toml"
env -u GNOBLIN_CONFIG HOME="$home" XDG_CONFIG_HOME="$tmp/legacy" \
    "$prefix/libexec/gnoblin-seed-config" "$prefix/share/gnoblin/init.lua.example"
[[ ! -e "$legacy_config/init.lua" ]]

# An explicit caller-selected path does not cause a default config to be made.
override_config="$tmp/custom.lua"
env HOME="$home" XDG_CONFIG_HOME="$tmp/override-config" GNOBLIN_CONFIG="$override_config" \
    "$prefix/libexec/gnoblin-seed-config" "$prefix/share/gnoblin/init.lua.example"
[[ ! -e "$tmp/override-config/gnoblin/init.lua" ]]

echo "PASS: first-login Lua config is seeded without replacing user or legacy configs"
