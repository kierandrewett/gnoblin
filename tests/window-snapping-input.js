import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
export default function enable(api) {
    GLib.file_set_contents(
        GLib.getenv("XDG_CONFIG_HOME") + "/snap-display.json",
        JSON.stringify({ DISPLAY: GLib.getenv("DISPLAY"), XAUTHORITY: GLib.getenv("XAUTHORITY") }),
    );
    const device = global.stage.context
        .get_backend()
        .get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    const keyboard = global.stage.context
        .get_backend()
        .get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    const path = GLib.getenv("XDG_CONFIG_HOME") + "/snap-input.json";
    const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
        const file = Gio.File.new_for_path(path);
        if (!file.query_exists(null)) return GLib.SOURCE_CONTINUE;
        const [ok, bytes] = file.load_contents(null);
        file.delete(null);
        const cmd = JSON.parse(new TextDecoder().decode(bytes));
        if (cmd.op === "key")
            keyboard.notify_keyval(
                GLib.get_monotonic_time(),
                Clutter["KEY_" + cmd.key],
                cmd.down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED,
            );
        if (cmd.op === "move") device.notify_absolute_motion(GLib.get_monotonic_time(), cmd.x, cmd.y);
        if (cmd.op === "drop") {
            device.notify_absolute_motion(GLib.get_monotonic_time(), cmd.x, cmd.y);
            device.notify_button(GLib.get_monotonic_time(), 1, Clutter.ButtonState.RELEASED);
        }
        if (cmd.op === "button")
            device.notify_button(
                GLib.get_monotonic_time(),
                1,
                cmd.down ? Clutter.ButtonState.PRESSED : Clutter.ButtonState.RELEASED,
            );
        return GLib.SOURCE_CONTINUE;
    });
    api._disposers.push(() => {
        GLib.source_remove(timer);
        device.run_dispose();
        keyboard.run_dispose();
    });
}
