#!/usr/bin/env bash
# Add the files a distribution package ships outside Gnoblin's private prefix.
#
# ./build.sh installs the whole runtime below one private prefix, for example /usr/lib/gnoblin. A system package also
# needs a few entries where the desktop looks for them: /usr/bin, the display manager session list, the systemd user
# unit directory, the portal configuration and the polkit actions. This script makes those entries from the private
# tree. Every package recipe gets the same layout from the build, and none of them repeats these steps.
#
# usage: install-system-layout.sh PRIVATE_PREFIX [SYSTEM_PREFIX]
#
#   PRIVATE_PREFIX   where the runtime is installed on the target system (for example /usr/lib/gnoblin)
#   SYSTEM_PREFIX    where public entries go on the target system (default /usr)
#
# GNOBLIN_STAGE_ROOT       a root, like DESTDIR, under which both prefixes are written (default: none)
# GNOBLIN_SYSCONFDIR       the configuration directory on the target system (default /etc)
# GNOBLIN_LAYOUT_GEOCLUE   set to 1 to add the GeoClue agent authorisation, for a package that ships it separately
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRIVATE="${1:?usage: install-system-layout.sh PRIVATE_PREFIX [SYSTEM_PREFIX]}"
SYSTEM="${2:-/usr}"
STAGE="${GNOBLIN_STAGE_ROOT:-}"
SYSCONFDIR="${GNOBLIN_SYSCONFDIR:-/etc}"

case "$PRIVATE" in
    /*) ;;
    *)
        echo "[layout] the private prefix must be an absolute path: $PRIVATE" >&2
        exit 2
        ;;
esac
PRIVATE="${PRIVATE%/}"
SYSTEM="${SYSTEM%/}"
if [ "$PRIVATE" = "$SYSTEM" ] || [ -z "$PRIVATE" ]; then
    echo "[layout] the private prefix must differ from the system prefix" >&2
    exit 2
fi

PRIVATE_DIR="$STAGE$PRIVATE"
SYSTEM_DIR="$STAGE$SYSTEM"
if [ ! -x "$PRIVATE_DIR/bin/gnoblin" ]; then
    echo "[layout] no Gnoblin runtime in $PRIVATE_DIR. Build it first with ./build.sh." >&2
    exit 1
fi

# publish SOURCE DESTINATION [MODE]: copy a file from the private tree to a public place.
publish() {
    install -Dm"${3:-644}" "$PRIVATE_DIR/$1" "$SYSTEM_DIR/$2"
    echo "[layout] $SYSTEM/$2"
}

# publish_if_present SOURCE DESTINATION: the portal backend is optional, so its files may be absent.
publish_if_present() {
    if [ -e "$PRIVATE_DIR/$1" ]; then
        publish "$1" "$2"
    fi
}

install -d "$SYSTEM_DIR/bin"
for tool in gnoblin gnoblinctl; do
    ln -sfn "$PRIVATE/bin/$tool" "$SYSTEM_DIR/bin/$tool"
    echo "[layout] $SYSTEM/bin/$tool -> $PRIVATE/bin/$tool"
done

publish share/wayland-sessions/gnoblin.desktop share/wayland-sessions/gnoblin.desktop
sed -i -e "s|^Exec=.*|Exec=$PRIVATE/bin/gnoblin|" \
    -e 's|^DesktopNames=.*|DesktopNames=Gnoblin;|' \
    "$SYSTEM_DIR/share/wayland-sessions/gnoblin.desktop"

publish share/xdg-desktop-portal/gnoblin-portals.conf share/xdg-desktop-portal/gnoblin-portals.conf
publish lib/systemd/user/gnoblin-session.target lib/systemd/user/gnoblin-session.target
publish lib/systemd/user/gnoblin-idle.service lib/systemd/user/gnoblin-idle.service

publish share/man/man1/gnoblin.1 share/man/man1/gnoblin.1
publish share/man/man1/gnoblinctl.1 share/man/man1/gnoblinctl.1

publish_if_present share/xdg-desktop-portal/portals/gnoblin.portal share/xdg-desktop-portal/portals/gnoblin.portal
publish_if_present share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service \
    share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
publish_if_present lib/systemd/user/xdg-desktop-portal-gnoblin.service lib/systemd/user/xdg-desktop-portal-gnoblin.service

# Mutter's backlight helper ships a polkit action under a GNOME name. A system package must not clash with GNOME's own
# copy, so Gnoblin publishes the action under its own name.
backlight_policy=share/polkit-1/actions/org.gnome.mutter.backlight-helper.policy
if [ -e "$PRIVATE_DIR/$backlight_policy" ]; then
    install -d "$SYSTEM_DIR/share/polkit-1/actions"
    sed 's/org.gnome.mutter.backlight-helper/org.gnoblin.mutter.backlight-helper/g' \
        "$PRIVATE_DIR/$backlight_policy" \
        >"$SYSTEM_DIR/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
    chmod 644 "$SYSTEM_DIR/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
    echo "[layout] $SYSTEM/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy"
fi

# The optional GeoClue agent authorisation. Only a recipe with a separate package for it asks for the file, because a
# package that does not list it would fail as having unpackaged files.
if [ "${GNOBLIN_LAYOUT_GEOCLUE:-0}" = 1 ]; then
    install -Dm644 "$ROOT/packaging/geoclue/50-gnoblin.conf" "$STAGE$SYSCONFDIR/geoclue/conf.d/50-gnoblin.conf"
    echo "[layout] $SYSCONFDIR/geoclue/conf.d/50-gnoblin.conf"
fi

# The schema cache is generated on the target, after the package is installed, so the package must not carry one.
rm -f "$PRIVATE_DIR/share/glib-2.0/schemas/gschemas.compiled"
