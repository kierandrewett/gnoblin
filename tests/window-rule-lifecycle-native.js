import Gio from "gi://Gio";
import GLib from "gi://GLib";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import { parseDocument } from "resource:///org/gnome/shell/ui/components/gnoblinConfig.js";

export default function (api) {
    const control = Main.componentManager._allComponents.gnoblinControl;
    const rules = control._windowRules;
    const file = Gio.File.new_for_path(GLib.get_user_config_dir() + "/gnoblin/lifecycle-results.json");
    let records = [];
    if (file.query_exists(null)) records = JSON.parse(new TextDecoder().decode(file.load_contents(null)[1]));
    rules.refresh(
        parseDocument({
            "window-rules": [
                {
                    match: { type: "window" },
                    corners: { radius: 14, mode: "force", shadow: { blur: 16, opacity: 0.5 } },
                    borders: { "inner-width": 1, "outer-width": 1 },
                },
            ],
        }),
    );
    // Reproduce the compatibility script that uses its owner during disposal.
    api._disposers.push(() => rules.refresh());
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
        const actor = global.get_window_actors().find((a) => a.meta_window.title === "Lifecycle fixture");
        const entry = rules._actors.get(actor);
        records.push({
            decorations: actor.get_children().filter((c) => c._gnoblinDecoration).length,
            surfaceIsDecoration: !!entry.surface._gnoblinDecoration,
            cornerEffects: entry.surface.get_effects().filter((e) => String(e).includes("GnoblinCornersEffect")).length,
        });
        file.replace_contents(JSON.stringify(records), null, false, 0, null);
        if (records.length < 4) {
            control.disable();
            control.enable();
        }
        return GLib.SOURCE_REMOVE;
    });
}
