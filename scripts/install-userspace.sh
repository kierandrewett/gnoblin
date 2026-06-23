#!/usr/bin/env bash
# Install gnoblin's userspace into a prefix: the layer-shell clients/daemons
# and the org.gnoblin.shell settings schema. Mirrors what the RPM ships, but
# into a dev prefix so the whole stack can run from ./install without a system
# install.
set -euo pipefail

PREFIX="${1:?usage: install-userspace.sh PREFIX}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

install -d "$PREFIX/bin"

# Userspace clients share one cargo workspace (src/clients) and target dir, with
# every role/daemon a member. Build once, install each binary on PATH so `exec`,
# `[roles]` and media-key/launcher binds resolve them. Prefer an offline build
# (deps cached); fall back online.
ws="$ROOT/src/clients/Cargo.toml"
cargo build --release --offline --manifest-path "$ws" 2>/dev/null \
  || cargo build --release --manifest-path "$ws"
target_dir="$(
  cargo metadata --format-version 1 --no-deps --manifest-path "$ws" |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
)"
for bin in gnoblin-topbar gnoblin-dock gnoblin-window-menu gnoblin-wallpaper \
           gnoblin-notifyd gnoblin-osd gnoblin-launcher gnoblin-night-light \
           gnoblin-power-menu; do
  install -m755 "$target_dir/release/$bin" "$PREFIX/bin/$bin" \
    && echo ">> installed $bin"
done
# Stale gtk4-era layout, if present.
rm -rf "$PREFIX/libexec/gnoblin"

# gnoblinctl — CLI for the dev.gnoblin.Shell action API.
install -m755 "$ROOT/dist/gnoblinctl" "$PREFIX/bin/gnoblinctl"

# gnoblin-screenshot — grim/slurp wrapper bound to the Print keys.
install -m755 "$ROOT/dist/gnoblin-screenshot" "$PREFIX/bin/gnoblin-screenshot"

# gnoblin-notify-center — toggles the notification-center history panel.
install -m755 "$ROOT/dist/gnoblin-notify-center" "$PREFIX/bin/gnoblin-notify-center"

# Default quick-settings plugin scripts (gnoblin-qs-*) — the dogfooded built-in
# tiles (wifi/bluetooth/output/mic/dnd/nightlight/darkstyle/powermode/...) as
# process/command plugins. Installed on PATH so the topbar's plugin host can
# spawn them by name from the default [qs-plugin.*] config.
for p in "$ROOT"/src/data/plugins/gnoblin-qs-*; do
  [ -e "$p" ] || continue
  install -m755 "$p" "$PREFIX/bin/$(basename "$p")" && echo ">> installed $(basename "$p")"
done

# Example config file (the compositor + protocols read $XDG_CONFIG_HOME/gnoblin/
# gnoblin.conf, NOT GSettings).
install -d "$PREFIX/share/gnoblin"
install -m644 "$ROOT/src/data/gnoblin.conf.example" "$PREFIX/share/gnoblin/"

# Settings schema — used by the control-center panel for per-monitor placement.
# Installed and compiled alongside mutter's.
install -d "$PREFIX/share/glib-2.0/schemas"
install -m644 "$ROOT/src/data/org.gnoblin.shell.gschema.xml" \
              "$PREFIX/share/glib-2.0/schemas/"
glib-compile-schemas "$PREFIX/share/glib-2.0/schemas"

echo ">> installed gnoblin userspace into $PREFIX"
echo "   clients: $PREFIX/bin/{gnoblin-topbar,gnoblin-dock,gnoblin-window-menu,gnoblin-wallpaper,...}"
echo "   schema:  $PREFIX/share/glib-2.0/schemas/org.gnoblin.shell.gschema.xml"
