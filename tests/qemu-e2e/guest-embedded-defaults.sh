#!/usr/bin/env bash
# Guest: helper for run-embedded-defaults.sh. Usage: guest-embedded-defaults.sh hide | read | unhide | restore
#
# hide moves the user's init.lua aside, with any old gnoblin.toml and gnoblin.conf (Gnoblin refuses to shadow an old
# config with a new default), so the next login has no user config and loads the embedded tree. read
# reports what that session looks like: the compositor state, whether it fell back, and the frame of a GTK window that
# asks for server-side decorations. unhide puts init.lua back. restore runs gnoblinctl config restore-default, checks
# the new folder and the backup, then puts the original folder back.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
DIR="$HOME/.config/gnoblin"
HOLD="$HOME/.config/gnoblin-embedded-defaults-test-hold"

case "${1:-}" in
    hide)
        mkdir -p "$HOLD"
        for name in init.lua gnoblin.toml gnoblin.conf; do
            if [ -e "$DIR/$name" ]; then mv "$DIR/$name" "$HOLD/$name"; fi
        done
        rm -f "$XDG_RUNTIME_DIR/gnoblin/config-fallback"
        ;;
    read)
        cat > /tmp/embedded-window.py <<'PY'
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="embedded-defaults-test")
window.set_default_size(700, 400)
window.set_child(Gtk.Label(label="frame"))
window.present()
loop = GLib.MainLoop()
GLib.timeout_add_seconds(12, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
PY
        echo "status=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        echo "marker=$(head -1 "$XDG_RUNTIME_DIR/gnoblin/config-fallback" 2>/dev/null || echo none)"
        echo "init_present=$([ -e "$DIR/init.lua" ] && echo yes || echo no)"
        nohup env GTK_CSD=0 GDK_BACKEND=wayland python3 /tmp/embedded-window.py >/dev/null 2>&1 < /dev/null &
        sleep 6
        "$G" window list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "embedded-defaults-test":
        f = w["frame"]; print("frame=%dx%d" % (f["width"], f["height"]))'
        pkill -f embedded-window.py
        ;;
    unhide)
        for name in init.lua gnoblin.toml gnoblin.conf; do
            if [ -e "$HOLD/$name" ]; then mv -f "$HOLD/$name" "$DIR/$name"; fi
        done
        rmdir "$HOLD" 2>/dev/null || true
        ;;
    restore)
        before="$(ls -d "$HOME"/.config/gnoblin.recovery-* 2>/dev/null | wc -l)"
        out="$("$G" config restore-default 2>&1 | head -c 300)"
        echo "restore_output=$out"
        after_dirs="$(ls -d "$HOME"/.config/gnoblin.recovery-* 2>/dev/null)"
        echo "backups_before=$before backups_after=$(printf '%s\n' "$after_dirs" | grep -c recovery)"
        backup="$(ls -dt "$HOME"/.config/gnoblin.recovery-* 2>/dev/null | head -1)"
        echo "new_init=$([ -f "$DIR/init.lua" ] && echo yes || echo no)"
        echo "new_has_shortcuts=$([ -f "$DIR/config/40-shortcuts.lua" ] && echo yes || echo no)"
        echo "new_has_switch_binding=$(grep -c '<Super>space' "$DIR/config/40-shortcuts.lua" 2>/dev/null || echo 0)"
        echo "new_has_no_test_files=$(ls "$DIR/config" | grep -c '^99-test' || true)"
        old_files="$(ls "$backup/config" 2>/dev/null | grep -c '^99-test' || true)"
        echo "backup_has_old_files=$([ "${old_files:-0}" -gt 0 ] && echo yes || echo no)"
        echo "backup_kept_bingux=$([ -f "$backup/bingux.lua" ] && echo yes || echo no)"
        echo "status_after=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        # Put the original folder back.
        mv "$DIR" "$HOME/.config/gnoblin.embedded-defaults-test-new"
        mv "$backup" "$DIR"
        rm -rf "$HOME/.config/gnoblin.embedded-defaults-test-new"
        "$G" config reload >/dev/null 2>&1
        echo "restored_init=$([ -f "$DIR/init.lua" ] && echo yes || echo no)"
        ;;
    *)
        echo "usage: $0 hide | read | unhide | restore" >&2
        exit 2
        ;;
esac
