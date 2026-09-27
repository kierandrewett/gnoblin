#!/usr/bin/env bash
# Install gnoblin's session data into a prefix:
#   - the `gnoblin` GNOME Shell session mode (strips the stock UI declaratively)
#   - the gnome-session definition (required components: org.gnoblin.Shell,
#     not the shared org.gnome.Shell -- see the comment in gnoblin.session)
#   - the wayland-session .desktop entry (shown at the login manager)
#   - gnoblin, the .desktop's Exec= target: sets runtime lookup paths
#     for the direct compositor launch or optional GNOME Session mode
#   - org.gnoblin.Shell.target / org.gnoblin.Shell@wayland.service, the
#     systemd --user units gnome-session's RequiredComponents needs to
#     actually start the patched gnome-shell -- gnoblin-specific unit names
#     so they never shadow a system GNOME Shell install's own units
#   - gnoblin-session.target, used by the lean direct launch path
#
# This step is additive: everything lands under <prefix> and can be removed by
# deleting it. No system files are touched, and nothing here registers with
# your live login manager or systemd --user instance. That is handled by
# `./build.sh --register-session` (see docs/install-source.md), a separate explicit
# step because it changes state outside <prefix>.
set -euo pipefail

PREFIX="${1:?usage: install-session.sh <prefix>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src/data/session"
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_validate_install_prefix "$PREFIX" || exit
STAGE_ROOT="${GNOBLIN_STAGE_ROOT:-}"
INSTALL_PREFIX="$STAGE_ROOT$PREFIX"
INSTALL_PREFIX="$(mkdir -p "$INSTALL_PREFIX" && cd "$INSTALL_PREFIX" && pwd)"
LIBDIR="${GNOBLIN_LIBDIR:-lib64}"
gnoblin_env_validate_libdir "$LIBDIR" || exit
IDLE_BINARY="${GNOBLIN_IDLE_BINARY:?Build the session with ./build.sh}"
GNOBLINCTL_BINARY="${GNOBLINCTL_BINARY:?Build the session with ./build.sh}"
GNOBLIN_IDENTITY_FILE="${GNOBLIN_IDENTITY_FILE:?Build the session with ./build.sh}"

# A prior development install may have included GNOME's extension manager,
# captive-network portal helper, calendar server, or test tools.
# Remove only the old Gnoblin-prefix artefacts. System GNOME files are never
# considered by this script.
rm -f \
    "$INSTALL_PREFIX/bin/gnome-extensions" \
    "$INSTALL_PREFIX/bin/gnome-extensions-app" \
    "$INSTALL_PREFIX/share/applications/org.gnome.Extensions.desktop" \
    "$INSTALL_PREFIX/share/dbus-1/services/org.gnome.Extensions.service" \
    "$INSTALL_PREFIX/share/glib-2.0/schemas/org.gnome.Extensions.gschema.xml" \
    "$INSTALL_PREFIX/share/metainfo/org.gnome.Extensions.metainfo.xml" \
    "$INSTALL_PREFIX/share/gnome-shell/org.gnome.Extensions" \
    "$INSTALL_PREFIX/share/gnome-shell/org.gnome.Extensions.data.gresource" \
    "$INSTALL_PREFIX/share/gnome-shell/org.gnome.Extensions.src.gresource" \
    "$INSTALL_PREFIX/share/gnome-shell/org.gnome.Shell.Extensions" \
    "$INSTALL_PREFIX/share/gnome-shell/org.gnome.Shell.Extensions.src.gresource" \
    "$INSTALL_PREFIX/share/bash-completion/completions/gnome-extensions" \
    "$INSTALL_PREFIX/share/applications/org.gnome.Shell.Extensions.desktop" \
    "$INSTALL_PREFIX/share/dbus-1/services/org.gnome.Shell.Extensions.service" \
    "$INSTALL_PREFIX/lib/systemd/user/org.gnome.Shell-disable-extensions.service"
rm -f \
    "$INSTALL_PREFIX/libexec/gnome-shell-portal-helper" \
    "$INSTALL_PREFIX/share/applications/org.gnome.Shell.PortalHelper.desktop" \
    "$INSTALL_PREFIX/share/dbus-1/services/org.gnome.Shell.PortalHelper.service" \
    "$INSTALL_PREFIX/libexec/gnome-shell-calendar-server" \
    "$INSTALL_PREFIX/share/dbus-1/services/org.gnome.Shell.CalendarServer.service" \
    "$INSTALL_PREFIX/bin/gnome-shell-test-tool" \
    "$INSTALL_PREFIX/libexec/gnome-shell-perf-helper"
if [ -d "$INSTALL_PREFIX/share/icons/hicolor" ]; then
    find "$INSTALL_PREFIX/share/icons/hicolor" -type f \( -name 'org.gnome.Extensions*' -o -name 'org.gnome.Shell.Extensions*' \) -delete
fi

install -Dm644 "$SRC/modes/gnoblin.json" \
    "$INSTALL_PREFIX/share/gnome-shell/modes/gnoblin.json"
install -Dm644 "$SRC/gnome-session/gnoblin.session" \
    "$INSTALL_PREFIX/share/gnome-session/sessions/gnoblin.session"

# Shared env helper first: gnoblin/gnoblin-shell-service both source
# it from their installed location.
install -Dm644 "$ROOT/src/tools/gnoblin-env.sh" "$INSTALL_PREFIX/libexec/gnoblin-env.sh"
install -Dm644 /dev/null "$INSTALL_PREFIX/libexec/gnoblin-libdir"
printf '%s\n' "$LIBDIR" >"$INSTALL_PREFIX/libexec/gnoblin-libdir"
install -Dm755 "$ROOT/src/tools/gnoblin" "$INSTALL_PREFIX/bin/gnoblin"
if [ -L "$INSTALL_PREFIX/bin/gnoblin-session" ] &&
    [ "$(readlink "$INSTALL_PREFIX/bin/gnoblin-session")" = gnoblin ]; then
    rm "$INSTALL_PREFIX/bin/gnoblin-session"
fi
install -Dm755 "$ROOT/src/tools/gnoblin-seed-config" "$INSTALL_PREFIX/libexec/gnoblin-seed-config"
install -Dm644 "$ROOT/src/data/init.lua.example" "$INSTALL_PREFIX/share/gnoblin/init.lua.example"
install -Dm644 "$SRC/gnoblin.desktop" "$INSTALL_PREFIX/share/wayland-sessions/gnoblin.desktop"
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/gnoblin|" \
    "$INSTALL_PREFIX/share/wayland-sessions/gnoblin.desktop"

# Normal Xcursor themes work without compiling extra artwork. Keep the custom
# vector theme available to users who explicitly request it from build.sh.
case "${GNOBLIN_VECTOR_CURSORS:-OFF}" in
    ON | TRUE | true | 1) vector_cursors_enabled=true ;;
    OFF | FALSE | false | 0) vector_cursors_enabled=false ;;
    *)
        echo "invalid vector cursor setting: $GNOBLIN_VECTOR_CURSORS" >&2
        exit 2
        ;;
esac
if "$vector_cursors_enabled"; then
    theme_build="$(mktemp -d)"
    trap 'rm -rf -- "$theme_build"' EXIT
    python3 "$ROOT/scripts/build-adwaita-hyprcursor.py" \
        --output "$theme_build/Adwaita-Hyprcursor" \
        --fallback "${ADWAITA_CURSOR_FALLBACK:-/usr/share/icons/Adwaita}"
    install -d "$INSTALL_PREFIX/share/icons/Adwaita-Hyprcursor"
    cp -a "$theme_build/Adwaita-Hyprcursor/." "$INSTALL_PREFIX/share/icons/Adwaita-Hyprcursor/"
fi

# Gnoblin-specific systemd --user units (ExecStart/Environment= need the
# resolved absolute prefix, so the *.service is generated from its .in).
install -Dm755 "$ROOT/src/tools/gnoblin-shell-service" "$INSTALL_PREFIX/bin/gnoblin-shell-service"
install -Dm644 "$SRC/systemd-user/org.gnoblin.Shell.target" \
    "$INSTALL_PREFIX/lib/systemd/user/org.gnoblin.Shell.target"
install -Dm644 "$SRC/systemd-user/gnoblin-session.target" \
    "$INSTALL_PREFIX/lib/systemd/user/gnoblin-session.target"
install -Dm755 "$IDLE_BINARY" "$INSTALL_PREFIX/libexec/gnoblin-idle"
sed "s|@PREFIX@|$PREFIX|g" "$SRC/systemd-user/gnoblin-idle.service.in" \
    >"$INSTALL_PREFIX/lib/systemd/user/gnoblin-idle.service.tmp"
install -Dm644 "$INSTALL_PREFIX/lib/systemd/user/gnoblin-idle.service.tmp" \
    "$INSTALL_PREFIX/lib/systemd/user/gnoblin-idle.service"
rm -f "$INSTALL_PREFIX/lib/systemd/user/gnoblin-idle.service.tmp"
# The drop-in that actually pulls the shell target into the session. Modern
# systemd-managed gnome-session ignores the .session RequiredComponents= line;
# gnome-session@gnoblin.target takes its deps from this .d/ drop-in instead.
# Without it the session logs in to a frozen screen with no compositor.
install -Dm644 "$SRC/systemd-user/gnome-session@gnoblin.target.d.conf" \
    "$INSTALL_PREFIX/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
sed "s|@PREFIX@|$PREFIX|g" "$SRC/systemd-user/org.gnoblin.Shell@wayland.service.in" \
    >"$INSTALL_PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp"
install -Dm644 "$INSTALL_PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp" \
    "$INSTALL_PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service"
rm -f "$INSTALL_PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp"

# Desktop-specific schema defaults. This runs after mutter/gnome-shell have
# installed their schemas, so the override is compiled into the prefix used by
# Gnoblin's wrappers (`XDG_CURRENT_DESKTOP=GNOME:Gnoblin`).
install -Dm644 "$SRC/schemas/00_org.gnoblin.mutter.gschema.override" \
    "$INSTALL_PREFIX/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override"
glib-compile-schemas "$INSTALL_PREFIX/share/glib-2.0/schemas"
# The gnoblinctl CLI (org.gnoblin.Shell control front-end).
install -Dm755 "$GNOBLINCTL_BINARY" "$INSTALL_PREFIX/bin/gnoblinctl"
install -Dm644 "$GNOBLIN_IDENTITY_FILE" "$INSTALL_PREFIX/share/gnoblin/version.json"

echo "Session data installed in $PREFIX"
