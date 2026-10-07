#!/usr/bin/env bash
# Guest: create the fixtures the prompt broker checks need. Safe to run again.
#
# - a default keyring that holds one secret (app prompt-e2e), created through the prompt broker
# - a GPG key gnoblin-e2e@example.com in ~/.gnupg-e2e whose passphrase is gnoblin-keyring-test
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-prompts.lua"

echo "== gpg key"
export GNUPGHOME="$HOME/.gnupg-e2e"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
echo "pinentry-program /usr/bin/pinentry-gnome3" > "$GNUPGHOME/gpg-agent.conf"
if gpg --list-secret-keys gnoblin-e2e@example.com >/dev/null 2>&1; then
    echo "key exists"
else
    timeout 60 gpg --batch --yes --pinentry-mode loopback --passphrase "gnoblin-keyring-test" \
        --quick-gen-key gnoblin-e2e@example.com default default never 2>&1 | tail -2
fi

echo "== keyring secret (the broker answers the new-keyring prompt)"
cp /tmp/99-test-prompts.lua "$CONFIG"
"$G" config reload | tail -1
sleep 3
echo -n "secret-value" | timeout 60 secret-tool store --label=prompt-e2e app prompt-e2e
echo "store rc=$?"
echo "lookup: [$(timeout 30 secret-tool lookup app prompt-e2e)]"

printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload | tail -1
