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
# This step is additive and atomic: everything lands under <prefix> and can
# be removed by deleting it. No system files are touched, and nothing here
# registers with your live login manager or systemd --user instance -- that
# is `scripts/register-session.sh` (see docs/installation.md), a separate,
# explicit step because it touches state outside <prefix>.
set -euo pipefail

PREFIX="${1:?usage: install-session.sh <prefix>}"
PREFIX="$(mkdir -p "$PREFIX" && cd "$PREFIX" && pwd)"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src/data/session"
source "$ROOT/src/tools/gnoblin-env.sh"
LIBDIR="${GNOBLIN_LIBDIR:-lib64}"
gnoblin_env_validate_libdir "$LIBDIR" || exit


install -Dm644 "$SRC/modes/gnoblin.json" \
  "$PREFIX/share/gnome-shell/modes/gnoblin.json"
install -Dm644 "$SRC/gnome-session/gnoblin.session" \
  "$PREFIX/share/gnome-session/sessions/gnoblin.session"

# Shared env helper first: gnoblin-session/gnoblin-shell-service both source
# it from their installed location.
install -Dm644 "$ROOT/src/tools/gnoblin-env.sh" "$PREFIX/libexec/gnoblin-env.sh"
install -Dm644 /dev/null "$PREFIX/libexec/gnoblin-libdir"
printf '%s\n' "$LIBDIR" > "$PREFIX/libexec/gnoblin-libdir"
install -Dm755 "$ROOT/src/tools/gnoblin-session" "$PREFIX/bin/gnoblin-session"
install -Dm644 "$SRC/gnoblin.desktop" "$PREFIX/share/wayland-sessions/gnoblin.desktop"
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/gnoblin-session|" \
  "$PREFIX/share/wayland-sessions/gnoblin.desktop"

# Gnoblin-specific systemd --user units (ExecStart/Environment= need the
# resolved absolute prefix, so the *.service is generated from its .in).
install -Dm755 "$ROOT/src/tools/gnoblin-shell-service" "$PREFIX/bin/gnoblin-shell-service"
install -Dm644 "$SRC/systemd-user/org.gnoblin.Shell.target" \
  "$PREFIX/lib/systemd/user/org.gnoblin.Shell.target"
sed "s|@PREFIX@|$PREFIX|g" "$SRC/systemd-user/org.gnoblin.Shell@wayland.service.in" \
  > "$PREFIX/lib/systemd/user/org.gnoblin.Shell@wayland.service.tmp"
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
echo ">> not yet registered with your login manager / systemd --user instance."
echo "   Run: ./scripts/register-session.sh $PREFIX"
