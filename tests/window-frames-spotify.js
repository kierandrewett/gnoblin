import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import { parseDocument } from "resource:///org/gnome/shell/ui/components/gnoblinConfig.js";
export default function (api) {
    const rules = Main.componentManager._allComponents.gnoblinControl._windowRules;
    rules.refresh(
        parseDocument({
            "window-rules": [
                {
                    match: { type: "window" },
                    frame: {
                        mode: "prefer-server",
                        extents: [36, 2, 2, 2],
                        renderer: GLib.getenv("GNOBLIN_SSD_RENDERER") || "native",
                    },
                    corners: { radius: 12, mode: "force", "remove-csd": true },
                    borders: { "inner-width": 1, "outer-width": 1 },
                },
            ],
        }),
    );
    const file = Gio.File.new_for_path(GLib.get_user_config_dir() + "/gnoblin/spotify-frames.json");
    const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 200, () => {
        const rows = global
            .get_window_actors()
            .filter((a) => a.meta_window.get_window_type() === Meta.WindowType.NORMAL)
            .map((actor) => {
                const window = actor.meta_window;
                const r = window.get_frame_rect();
                return {
                    title: window.title,
                    app: window.get_wm_class(),
                    layout: Meta.gnoblin_window_frame_get(window).recursiveUnpack(),
                    frame: [r.x, r.y, r.width, r.height],
                    visible: Meta.gnoblin_window_frame_get(window).recursiveUnpack().presentation.visible,
                };
            });
        file.replace_contents(JSON.stringify(rows), null, false, 0, null);
        return GLib.SOURCE_CONTINUE;
    });
    api._disposers.push(() => GLib.source_remove(timer));
}
