#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d /tmp/gnoblin-config-seed.XXXXXX)"
trap 'rm -rf -- "$tmp"' EXIT

prefix="$tmp/prefix"
fake_bin="$tmp/fake-bin"
home="$tmp/home"
config_home="$tmp/xdg-config"
mkdir -p "$prefix/bin" "$prefix/libexec" "$prefix/share/gnoblin" "$fake_bin" "$home"
install -m 755 "$ROOT/src/tools/gnoblin-session" "$prefix/bin/gnoblin-session"
install -m 644 "$ROOT/src/tools/gnoblin-env.sh" "$prefix/libexec/gnoblin-env.sh"
install -m 755 "$ROOT/src/tools/gnoblin-seed-config" "$prefix/libexec/gnoblin-seed-config"
install -m 644 "$ROOT/src/data/init.lua.example" "$prefix/share/gnoblin/init.lua.example"

cat >"$fake_bin/systemctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$fake_bin/dbus-update-activation-environment" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$fake_bin/gnome-session" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
config="$("$GNOBLIN_TEST_CTL" config path)"
[[ "$config" == "$XDG_CONFIG_HOME/gnoblin/init.lua" ]]
cmp -s "$config" "$GNOBLIN_TEST_TEMPLATE"
EOF
chmod 755 "$fake_bin/systemctl" "$fake_bin/dbus-update-activation-environment" "$fake_bin/gnome-session"

env -u GNOBLIN_CONFIG HOME="$home" XDG_CONFIG_HOME="$config_home" \
    XDG_DATA_DIRS="$prefix/share:/usr/share" PATH="$fake_bin:$PATH" \
    GNOBLIN_TEST_CTL="$ROOT/src/tools/gnoblinctl" \
    GNOBLIN_TEST_TEMPLATE="$prefix/share/gnoblin/init.lua.example" \
    "$prefix/bin/gnoblin-session"

# A later login preserves local edits.
printf '%s\n' '-- user edit' >"$config_home/gnoblin/init.lua"
env -u GNOBLIN_CONFIG HOME="$home" XDG_CONFIG_HOME="$config_home" \
    "$prefix/libexec/gnoblin-seed-config" "$prefix/share/gnoblin/init.lua.example"
[[ "$(cat "$config_home/gnoblin/init.lua")" == '-- user edit' ]]

# Existing TOML configs remain the selected config and are not shadowed by Lua.
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

echo "PASS: first-login config is seeded and selected, with user and legacy configs preserved"
