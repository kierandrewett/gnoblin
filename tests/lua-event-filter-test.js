// Run with the matching Mutter library and typelib on the search path.
import GLib from "gi://GLib";
import { ConfigFile } from "../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js";

globalThis.global = {
    get_pointer: () => [0, 0],
    stage: { get_actor_at_pos: () => null },
};

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

const directory = GLib.dir_make_tmp("gnoblin-event-filter-XXXXXX");
const path = `${directory}/init.lua`;
const config = new ConfigFile(path);
try {
    GLib.file_set_contents(path, "require('gnoblin').on('mutter.display.notify', function() end)");
    config.start();
    assert(config.wantsEvent("mutter.display.notify"), "registered events are forwarded");
    assert(!config.wantsEvent("mutter.window.unmanaged"), "unregistered events are skipped");
    config._events = new Set(["*"]);
    assert(config.wantsEvent("mutter.window.unmanaged"), "wildcard registration forwards every event");
    print("PASS: Lua event registration and compositor event filtering");
} finally {
    config.destroy();
    GLib.unlink(path);
    GLib.rmdir(directory);
}
