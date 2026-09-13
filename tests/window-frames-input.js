import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
export default function (api) {
    const pointer = global.stage.context
        .get_backend()
        .get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    const command = Gio.File.new_for_path(GLib.get_user_config_dir() + "/snap-input.json");
    const keyboard = global.stage.context.get_backend().get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    const result = Gio.File.new_for_path(GLib.get_user_config_dir() + "/snap-result.json");
    const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
        if (!command.query_exists(null)) return GLib.SOURCE_CONTINUE;
        const data = JSON.parse(new TextDecoder().decode(command.load_contents(null)[1]));
        command.delete(null);
        if (data.op === "key") keyboard.notify_key(GLib.get_monotonic_time(), data.code,
            data.down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
        if (data.op === "config-path")
            Main.componentManager._allComponents.gnoblinControl._config._override = data.path;
        if (data.op === "renderers") {
            try {
                const services = Object.fromEntries(
                    Object.entries(data.services).map(([name, argv]) => [name, new GLib.Variant("as", argv)]),
                );
                Meta.gnoblin_frame_renderers_configure(new GLib.Variant("a{sv}", services), false);
            } catch (error) {
                data.error = error.message;
            }
        }
        if (data.op === "move") pointer.notify_absolute_motion(GLib.get_monotonic_time(), data.x, data.y);
        else if (data.op === "button")
            pointer.notify_button(
                GLib.get_monotonic_time(),
                data.button ?? 1,
                data.down ? Clutter.ButtonState.PRESSED : Clutter.ButtonState.RELEASED,
            );
        if (data.op === "move") {
            let actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, data.x, data.y);
            data.pick = [];
            while (actor) {
                data.pick.push(String(actor));
                actor = actor.get_parent();
            }
        }
        data.pointer = global.get_pointer();
        const window = global.get_window_actors().find((a) => a.meta_window.title === "SSD fixture")?.meta_window;
        if (data.op === "unminimize") {
            window?.unminimize();
            window?.activate(global.get_current_time());
        }
        const rect = window?.get_frame_rect();
        data.window = window
            ? {
                  frame: [rect.x, rect.y, rect.width, rect.height],
                  maximized: window.is_maximized(),
                  minimized: window.minimized,
                  layout: Meta.gnoblin_window_frame_get(window).recursiveUnpack(),
              }
            : null;
        result.replace_contents(JSON.stringify(data), null, false, 0, null);
        return GLib.SOURCE_CONTINUE;
    });
    api._disposers.push(() => {
        GLib.source_remove(timer);
        pointer.run_dispose();
        keyboard.run_dispose();
    });
}
