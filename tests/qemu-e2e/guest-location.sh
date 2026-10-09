#!/usr/bin/env bash
# Guest: GeoClue asks Gnoblin's agent, the Lua handler answers, and GeoClue starts the client.
#
# GeoClue 2.8 reads an app identity only from Flatpak-style systemd scopes (app-flatpak-ID-N.scope,
# flatpak-ID-N.scope, xdg-app-ID-N.scope). A client without one counts as a system component: GeoClue
# skips the agent and clamps it to the agent's MaxAccuracyLevel. The test therefore runs the client in an
# app-flatpak scope started from the compositor, as a sandboxed app would be.
#
# Needs the packaged whitelist in /etc/geoclue/conf.d/50-gnoblin.conf and the stock demo agent stopped,
# because GeoClue accepts one agent and the demo agent approves every request.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-location.lua"
LAUNCH="$HOME/.config/gnoblin/config/99-test-geolaunch.lua"
fail=0

check() {
    # check NAME HAYSTACK NEEDLE
    if printf '%s' "$2" | grep -q -F -- "$3"; then
        echo "PASS $1"
    else
        echo "FAIL $1 (missing: $3)"
        fail=$((fail + 1))
    fi
}

cp /tmp/99-test-location.lua "$CONFIG"
printf -- '-- disabled\n' > "$LAUNCH"
"$G" config reload >/dev/null
sleep 3
pkill -f 'geoclue-2.0/demos/agent'
sudo systemctl stop geoclue
: > /tmp/location-events.log
rm -f /tmp/geo-app.out
sudo rm -f /tmp/geoclue-debug.log

# Run GeoClue by hand as its own user so its debug log is available.
sudo -u geoclue env G_MESSAGES_DEBUG=Geoclue /usr/libexec/geoclue > /tmp/geoclue-debug.log 2>&1 &
sleep 4

cat > "$LAUNCH" <<'LUA'
gnoblin.on("gnoblin.config.reloaded", function()
    gnoblin.commands.run({"systemd-run", "--user", "--scope", "--quiet", "--unit=app-flatpak-gnoblin-geotest-4242.scope",
        "sh", "-c", "python3 /tmp/geoclue-ask.py gnoblin-geotest > /tmp/geo-app.out 2>&1"})
end)
LUA
"$G" config reload >/dev/null
sleep 10

debug="$(cat /tmp/geoclue-debug.log 2>/dev/null)"
sudo pkill -u geoclue -x geoclue
rm -f "$LAUNCH"
printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload >/dev/null
rm -f "$CONFIG"

check "GeoClue accepted Gnoblin as an agent" "$debug" "New agent for user ID"
check "GeoClue lists gnoblin in its allowed agents" "$debug" "gnoblin"
check "GeoClue clamps the request to the agent's maximum" "$debug" "Max accuracy level allowed by agent: 4"
check "the Lua handler received the authorization request" "$(cat /tmp/location-events.log)" "requested id="
check "the client started after the handler allowed it" "$(cat /tmp/geo-app.out 2>/dev/null)" "started"
echo "failures: $fail"
exit "$fail"
