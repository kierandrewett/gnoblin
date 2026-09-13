import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import { parseDocument } from "resource:///org/gnome/shell/ui/components/gnoblinConfig.js";

export default function (api) {
    const rules = Main.componentManager._allComponents.gnoblinControl._windowRules;
    const file = Gio.File.new_for_path(GLib.get_user_config_dir() + "/gnoblin/corner-test.json");
    let pending = 0;
    const dump = () => {
        const rows = global
            .get_window_actors()
            .filter((a) => a.meta_window.title === "Corner fixture")
            .map((a) => {
                const f = a.meta_window.get_frame_rect();
                return {
                    frame: { x: f.x, y: f.y, width: f.width, height: f.height },
                    effect: !!rules._actors.get(a)?.corners?.effect,
                    uniforms: [...(rules._actors.get(a)?.corners?.effect?.values ?? [])],
                };
            });
        Gio.File.new_for_path(GLib.get_user_config_dir() + "/gnoblin/corner-frames.json").replace_contents(
            JSON.stringify(rows),
            null,
            false,
            0,
            null,
        );
    };
    const later = () => {
        if (pending) GLib.source_remove(pending);
        pending = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
            pending = 0;
            dump();
            return GLib.SOURCE_REMOVE;
        });
    };
    const apply = () => {
        const [, bytes] = file.load_contents(null);
        const { _action, ...corners } = JSON.parse(new TextDecoder().decode(bytes));
        const win = global.get_window_actors().find((a) => a.meta_window.title === "Corner fixture")?.meta_window;
        if (_action === "reprobe") win?.get_compositor_private().emit("first-frame");
        if (_action === "maximize") win.maximize(Meta.MaximizeFlags.BOTH);
        if (_action === "unmaximize") win.unmaximize(Meta.MaximizeFlags.BOTH);
        if (_action === "fullscreen") win.make_fullscreen();
        if (_action === "unfullscreen") win.unmake_fullscreen();
        rules.refresh(
            parseDocument({ "window-rules": [{ match: { type: "window" }, frame: { mode: "off" }, corners }] }),
        );
        later();
    };
    const monitor = file.get_parent().monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
    monitor.connect("changed", (_m, f, other) => {
        if (f?.equal(file) || other?.equal(file)) apply();
    });
    apply();
    const id = global.window_manager.connect("map", later);
    api._disposers.push(() => {
        monitor.cancel();
        if (pending) GLib.source_remove(pending);
        global.window_manager.disconnect(id);
    });
}
