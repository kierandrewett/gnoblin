#!/usr/bin/env bash
# Install gnoblin's session data into a prefix:
#   - the `gnoblin` GNOME Shell session mode (strips the stock UI declaratively)
#   - the gnome-session definition (required components: org.gnoblin.Shell,
#     not the shared org.gnome.Shell -- see the comment in gnoblin.session)
#   - the wayland-session .desktop entry (shown at the login manager)
#   - gnoblin-session, the .desktop's Exec= target: points gnome-session's own
#     lookups (PATH, session-mode env) at this prefix (see src/tools/gnoblin-session)
#   - org.gnoblin.Shell.target / org.gnoblin.Shell@wayland.service, the
#     systemd --user units gnome-session's RequiredComponents needs to
#     actually start the patched gnome-shell -- gnoblin-specific unit names
#     so they never shadow a system GNOME Shell install's own units
#
# This step is additive: everything lands under <prefix> and can be removed by
# deleting it. No system files are touched, and nothing here registers with
# your live login manager or systemd --user instance. That is handled by
# `scripts/register-session.sh` (see docs/installation.md), a separate explicit
# step because it changes state outside <prefix>.
set -euo pipefail

PREFIX="${1:?usage: install-session.sh <prefix>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src/data/session"
source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_validate_install_prefix "$PREFIX" || exit
PREFIX="$(mkdir -p "$PREFIX" && cd "$PREFIX" && pwd)"
LIBDIR="${GNOBLIN_LIBDIR:-lib64}"
gnoblin_env_validate_libdir "$LIBDIR" || exit

# A prior development install may have included GNOME's extension manager.
# Remove only the old Gnoblin-prefix artefacts. System GNOME files are never
# considered by this script.
rm -f \
    "$PREFIX/bin/gnome-extensions" \
    "$PREFIX/bin/gnome-extensions-app" \
    "$PREFIX/share/applications/org.gnome.Extensions.desktop" \
    "$PREFIX/share/dbus-1/services/org.gnome.Extensions.service" \
    "$PREFIX/share/glib-2.0/schemas/org.gnome.Extensions.gschema.xml" \
    "$PREFIX/share/metainfo/org.gnome.Extensions.metainfo.xml" \
    "$PREFIX/share/gnome-shell/org.gnome.Extensions" \
    "$PREFIX/share/gnome-shell/org.gnome.Extensions.data.gresource" \
    "$PREFIX/share/gnome-shell/org.gnome.Extensions.src.gresource" \
    "$PREFIX/share/gnome-shell/org.gnome.Shell.Extensions" \
    "$PREFIX/share/gnome-shell/org.gnome.Shell.Extensions.src.gresource" \
    "$PREFIX/share/bash-completion/completions/gnome-extensions" \
    "$PREFIX/share/applications/org.gnome.Shell.Extensions.desktop" \
    "$PREFIX/share/dbus-1/services/org.gnome.Shell.Extensions.service" \
    "$PREFIX/lib/systemd/user/org.gnome.Shell-disable-extensions.service"
if [ -d "$PREFIX/share/icons/hicolor" ]; then
    find "$PREFIX/share/icons/hicolor" -type f \( -name 'org.gnome.Extensions*' -o -name 'org.gnome.Shell.Extensions*' \) -delete
fi

install -Dm644 "$SRC/modes/gnoblin.json" \
    "$PREFIX/share/gnome-shell/modes/gnoblin.json"
install -Dm644 "$SRC/gnome-session/gnoblin.session" \
    "$PREFIX/share/gnome-session/sessions/gnoblin.session"

# Shared env helper first: gnoblin-session/gnoblin-shell-service both source
# it from their installed location.
install -Dm644 "$ROOT/src/tools/gnoblin-env.sh" "$PREFIX/libexec/gnoblin-env.sh"
install -Dm644 /dev/null "$PREFIX/libexec/gnoblin-libdir"
printf '%s\n' "$LIBDIR" >"$PREFIX/libexec/gnoblin-libdir"
# A privately built GIRepository searches its own prefix. Keep access to the
# host's service bindings (AccountsService, NetworkManager, UPower, and others).
# Link only namespaces not supplied privately. Exporting the entire host
# typelib directory would make its GLib/Gio bindings override our private ones.
if [ -d "$PREFIX/deps" ]; then
    typelib_dir="$(pkg-config --variable=typelibdir gobject-introspection-1.0)"
    [ -d "$typelib_dir" ] || {
        echo "Missing system typelib directory: $typelib_dir" >&2
        exit 1
    }
    mkdir -p "$PREFIX/$LIBDIR/girepository-1.0"
    for typelib in "$typelib_dir"/*.typelib; do
        [ -f "$typelib" ] || continue
        name="${typelib##*/}"
        target="$PREFIX/$LIBDIR/girepository-1.0/$name"
        if [ -e "$PREFIX/deps/$LIBDIR/girepository-1.0/$name" ]; then
            if [ -L "$target" ] && [ "$(readlink "$target")" = "$typelib" ]; then
                rm "$target"
            fi
            continue
        fi
        [ ! -e "$target" ] || continue
        ln -s "$typelib" "$target"
    done
fi
install -Dm755 "$ROOT/src/tools/gnoblin-session" "$PREFIX/bin/gnoblin-session"
install -Dm644 "$SRC/gnoblin.desktop" "$PREFIX/share/wayland-sessions/gnoblin.desktop"
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/gnoblin-session|" \
    "$PREFIX/share/wayland-sessions/gnoblin.desktop"

# Gnoblin-specific systemd --user units (ExecStart/Environment= need the
# resolved absolute prefix, so the *.service is generated from its .in).
install -Dm755 "$ROOT/src/tools/gnoblin-shell-service" "$PREFIX/bin/gnoblin-shell-service"
install -Dm644 "$SRC/systemd-user/org.gnoblin.Shell.target" \
    "$PREFIX/lib/systemd/user/org.gnoblin.Shell.target"
# The drop-in that actually pulls the shell target into the session. Modern
# systemd-managed gnome-session ignores the .session RequiredComponents= line;
# gnome-session@gnoblin.target takes its deps from this .d/ drop-in instead.
# Without it the session logs in to a frozen screen with no compositor.
install -Dm644 "$SRC/systemd-user/gnome-session@gnoblin.target.d.conf" \
    "$PREFIX/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
sed "s|@PREFIX@|$PREFIX|g" "$SRC/systemd-user/org.gnoblin.Shell@wayland.service.in" \
    >"$PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp"
install -Dm644 "$PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp" \
    "$PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service"
rm -f "$PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp"

# Desktop-specific schema defaults. This runs after mutter/gnome-shell have
# installed their schemas, so the override is compiled into the prefix used by
# Gnoblin's wrappers (`XDG_CURRENT_DESKTOP=GNOME:Gnoblin`).
install -Dm644 "$SRC/schemas/00_org.gnoblin.mutter.gschema.override" \
    "$PREFIX/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override"
glib-compile-schemas "$PREFIX/share/glib-2.0/schemas"
# The gnoblinctl CLI (org.gnoblin.Shell control front-end).
install -Dm755 "$ROOT/src/tools/gnoblinctl" "$PREFIX/bin/gnoblinctl"
install -Dm755 "$ROOT/src/tools/gnoblin-clipboard-paste" "$PREFIX/bin/gnoblin-clipboard-paste"
# Installed for explicit development only. This unit is intentionally neither
# enabled nor attached to the session while GNOME ScreenShield owns locking.
install -Dm755 "$ROOT/src/lock/gnoblin-lockd.py" "$PREFIX/libexec/gnoblin-lockd"
install -Dm644 "$ROOT/src/lock/policy.py" "$PREFIX/libexec/policy.py"
install -Dm755 "$ROOT/src/lock/gnoblin-lockctl" "$PREFIX/bin/gnoblin-lockctl"
install -Dm644 "$ROOT/src/lock/lock.conf.example" "$PREFIX/share/gnoblin/lock.conf.example"
sed "s|@PREFIX@|$PREFIX|g" "$ROOT/src/lock/gnoblin-lockd.service" \
    >"$PREFIX/lib/systemd/user/gnoblin-lockd.service.tmp"
install -Dm644 "$PREFIX/lib/systemd/user/gnoblin-lockd.service.tmp" \
    "$PREFIX/lib/systemd/user/gnoblin-lockd.service"
rm -f "$PREFIX/lib/systemd/user/gnoblin-lockd.service.tmp"
install -Dm644 "$ROOT/gnoblin-version.json" "$PREFIX/share/gnoblin/version.json"

echo ">> installed gnoblin session data into $PREFIX:"
echo "     share/gnome-shell/modes/gnoblin.json     (UI-strip session mode)"
echo "     share/gnome-session/sessions/gnoblin.session (required components)"
echo "     share/wayland-sessions/gnoblin.desktop   (login entry, Exec= -> bin/gnoblin-session)"
echo "     libexec/gnoblin-env.sh                   (shared prefix lookup-path helper)"
echo "     libexec/gnoblin-libdir                  (installed library-directory contract)"
echo "     bin/gnoblin-session                      (login-manager wrapper)"
echo "     bin/gnoblin-shell-service                (systemd unit ExecStart wrapper)"
echo "     share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override (Gnoblin schema defaults)"
echo "     lib/systemd/user/org.gnoblin.Shell{.target,@wayland.service} (patched shell unit)"
echo "     bin/gnoblinctl                           (control CLI)"
echo "     bin/gnoblin-lockctl                      (disabled lock-broker CLI)"
echo ">> not yet registered with your login manager / systemd --user instance."
echo "   Run: ./scripts/register-session.sh $PREFIX"
