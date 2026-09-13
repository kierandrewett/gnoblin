import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import { parseDocument } from "resource:///org/gnome/shell/ui/components/gnoblinConfig.js";

export default function (api) {
    const file = Gio.File.new_for_path(GLib.get_user_config_dir() + "/gnoblin/frames-results.json");
    const rules = Main.componentManager._allComponents.gnoblinControl._windowRules;
    let phase = 0;
    let attempts = 0;
    let restoredAt = 0;
    const records = [];
    const negotiated = GLib.getenv("GNOBLIN_SSD_NEGOTIATED") === "1";
    const renderer = GLib.getenv("GNOBLIN_SSD_RENDERER") || "native";
    const top = 36;
    const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
        try {
            const actor = global.get_window_actors().find((a) => a.meta_window.title === "SSD fixture");
            if (
                global.get_window_actors().filter((a) => a.meta_window.get_window_type() === Meta.WindowType.NORMAL)
                    .length > 1
            )
                throw new Error("renderer presented an extra application window");
            if (!actor) throw new Error("fixture not mapped");
            const window = actor.meta_window;
            const layout = Meta.gnoblin_window_frame_get(window).recursiveUnpack();
            if (++attempts > 80) throw new Error(`timeout phase ${phase}: ${JSON.stringify(layout)}`);
            if (phase === 0) {
                if (layout.border.some(Boolean) || layout.presentation.visible)
                    throw new Error("SSD was enabled without an explicit frame rule");
                rules.refresh(
                    parseDocument({
                        "window-rules": [
                            {
                                match: { title: "SSD fixture" },
                                frame: {
                                    mode: negotiated ? "prefer-server" : "replace",
                                    crop: [20, 0, 0, 0],
                                    extents: [top, 2, 2, 2],
                                    renderer,
                                },
                                borders: { "inner-width": 1, "inner-color": "#505050ff", "outer-width": 1 },
                            },
                        ],
                    }),
                );
                phase++;
            } else if (phase === 1 && layout.border[0] === top) {
                const frame = window.get_frame_rect();
                const root = actor.get_children().find((c) => c.get_name() === "gnoblin-native-frame");
                if (!root?.visible || (renderer !== "native" && !layout.presentation.external))
                    return GLib.SOURCE_CONTINUE;
                records.push({
                    layout,
                    frame: [frame.x, frame.y, frame.width, frame.height],
                    title: window.title,
                    header: [root.x, root.y, root.width, top],
                });
                window.make_fullscreen();
                phase++;
            } else if (phase === 2 && window.fullscreen && layout.border.every((n) => n === 0)) {
                records.push({ fullscreen: true, layout });
                window.unmake_fullscreen();
                phase++;
            } else if (phase === 3 && !window.fullscreen && layout.border[0] === top) {
                restoredAt = GLib.get_monotonic_time();
                phase++;
            } else if (phase === 4 && !window.fullscreen && layout.border[0] === top) {
                if (GLib.get_monotonic_time() - restoredAt < 500000) return GLib.SOURCE_CONTINUE;
                if (!layout.presentation.visible || (renderer !== "native" && !layout.presentation.external))
                    return GLib.SOURCE_CONTINUE;
                actor.get_image(null).writeToPNG(GLib.get_user_config_dir() + "/gnoblin/ssd-window.png");
                records.push({ restored: true, layout });
                file.replace_contents(JSON.stringify({ records }), null, false, 0, null);
                return GLib.SOURCE_REMOVE;
            }
        } catch (error) {
            file.replace_contents(
                JSON.stringify({ error: error.message, stack: error.stack, records }),
                null,
                false,
                0,
                null,
            );
            return GLib.SOURCE_REMOVE;
        }
        return GLib.SOURCE_CONTINUE;
    });
    api._disposers.push(() => {
        if (GLib.MainContext.default().find_source_by_id(timer)) GLib.source_remove(timer);
    });
}
