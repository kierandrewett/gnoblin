// GSETTINGS_BACKEND=memory gjs -m tests/shortcuts-test.js
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import {
    Shortcuts,
    CommandShortcuts,
    ShortcutInput,
    validateShortcuts,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js";
import Meta from "gi://Meta";
import Clutter from "gi://Clutter";

if (GLib.getenv("GSETTINGS_BACKEND") !== "memory")
    throw new Error("Tests require GSETTINGS_BACKEND=memory; never modify real shortcuts");
function assert(condition, message) {
    if (!condition) throw new Error(message);
}
const base = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const media = new Gio.Settings({ schema_id: "org.gnome.settings-daemon.plugins.media-keys" });
const native = new Gio.Settings({ schema_id: "org.gnome.shell.keybindings" });
const entry = { name: "capture", binding: "<Alt>s", command: ["qs", "-p", "/a path/with spaces", "quote'and$HOME"] };
const builtIn = { name: "screenshot", action: "gnome:shell.show_screenshot_ui", binding: [] };
const config = { shortcuts: [entry, builtIn] };
media.set_strv("custom-keybindings", [`${base}user-owned/`]);
native.set_strv("show-screenshot-ui", ["<Alt>s"]);
let registered = [];
const prefs = { prefs_apply_gnoblin_keybindings() {} };
const manager = new Shortcuts(
    {
        apply(entries) {
            registered = entries;
        },
        destroy() {},
    },
    prefs,
);
manager.apply(config);
assert(
    JSON.stringify(native.get_strv("show-screenshot-ui")) === JSON.stringify(["<Alt>s"]),
    "built-in override does not write GNOME settings",
);
assert(registered.length === 1 && registered[0] === entry, "register shortcut through native command backend");
const actionConfig = validateShortcuts({ shortcuts: [builtIn] });
assert(actionConfig.shortcuts.length === 0, "built-in action is not registered as a command");
assert(
    JSON.stringify(actionConfig.keybindings.shell.show_screenshot_ui) === "[]",
    "built-in actions become native keybinding overrides",
);
let customBindingChanges = 0;
media.connect("changed::custom-keybindings", () => customBindingChanges++);
manager.apply(config);
assert(customBindingChanges === 0, "Gnoblin never writes media-key custom shortcut settings");
manager.apply({ ...config, shortcuts: [{ ...entry, binding: "<Super>s" }] });
assert(registered[0].binding === "<Super>s", "binding edits update through native command backend");
for (const invalid of [
    { shortcuts: [entry, entry] },
    { shortcuts: [entry, { ...entry, name: "other", binding: "<Mod1>S" }] },
    { shortcuts: [{ ...entry, binding: "<Typo>s" }] },
    { shortcuts: [{ ...entry, binding: "<Alt>NotARealKey" }] },
    { shortcuts: [{ ...entry, command: "qs" }] },
    { shortcuts: [{ ...builtIn, action: "unknown.action" }] },
    { shortcuts: [{ ...builtIn, action: "wm.not_a_real_action" }] },
    { shortcuts: [{ ...builtIn, command: ["qs"] }] },
    { shortcuts: [builtIn, { ...builtIn, name: "screenshot-again" }] },
    { shortcuts: [builtIn], keybindings: { shell: { show_screenshot_ui: ["Print"] } } },
    {
        shortcuts: [
            { ...builtIn, binding: ["<Alt>s"] },
            { ...entry, binding: "<Alt>s" },
        ],
    },
    { shortcuts: [{ ...entry, name: "../escape" }] },
    { keybindings: { shell: { "show-screenshot-ui": [] } } },
    { keybindings: { media: { custom_keybindings: [] } } },
    { keybindings: { shell: { show_screenshot_ui: "Print" } } },
    { shortcuts: [entry], keybindings: { shell: { show_screenshot_ui: ["<Alt>s"] } } },
]) {
    let rejected = false;
    try {
        manager.apply(invalid);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject invalid settings: ${JSON.stringify(invalid)}`);
    assert(registered[0].binding === "<Super>s", "invalid config preserves registered shortcut");
}
manager.apply({});
assert(registered.length === 0, "empty config removes native command shortcuts");
assert(
    JSON.stringify(media.get_strv("custom-keybindings")) === JSON.stringify([`${base}user-owned/`]),
    "GNOME media-key custom shortcuts remain untouched",
);
assert(
    JSON.stringify(native.get_strv("show-screenshot-ui")) === JSON.stringify(["<Alt>s"]),
    "removing a built-in override leaves GNOME settings untouched",
);
assert(validateShortcuts({}).shortcuts.length === 0, "empty defaults");
print("PASS: native shortcut registration, reload, validation, built-in overrides and removal");

let activated, overlayReleased;
let nextAction = 100;
const grabs = [],
    releases = [],
    launched = [];
const display = {
    connect(signal, callback) {
        if (signal === "overlay-key") {
            overlayReleased = callback;
            return 2;
        }
        activated = callback;
        return 1;
    },
    disconnect(id) {
        if (id === 2) overlayReleased = null;
        else activated = null;
    },
    grab_accelerator(binding, flags) {
        grabs.push({ binding, flags });
        return binding === "<Alt>x" ? Meta.KeyBindingAction.NONE : nextAction++;
    },
    ungrab_accelerator(action) {
        releases.push(action);
    },
};
const commands = new CommandShortcuts(
    display,
    () => {},
    (argv) => launched.push(argv),
);
commands.apply([entry]);
assert(
    grabs[0].flags === Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
    "held shortcut ignores keyboard repeat at compositor",
);
activated(null, 100);
assert(launched.length === 1 && launched[0] === entry.command, "one activation launches original argv once");
commands.apply([{ ...entry, command: ["updated"] }]);
assert(grabs.length === 1, "command-only edit does not release/regrab accelerator");
activated(null, 100);
assert(launched[1][0] === "updated", "existing grab uses updated command");
let conflict = false;
try {
    commands.apply([{ ...entry, binding: "<Alt>x" }]);
} catch {
    conflict = true;
}
assert(conflict && releases.length === 0, "failed binding edit preserves working shortcut");
commands.apply([]);
assert(releases[0] === 100, "removal releases compositor grab");
const search = { name: "search", binding: "Super", command: ["binguxctl", "search", "open"] };
assert(validateShortcuts({ shortcuts: [search] }).shortcuts.length === 1, "bare Super config is valid");
const countBefore = grabs.length;
commands.apply([search]);
assert(grabs.length === countBefore, "bare Super uses Mutter release event, not a press grab");
overlayReleased();
assert(launched.at(-1) === search.command, "release executes configured argv");
commands.apply([{ ...search, command: ["changed"] }]);
overlayReleased();
assert(launched.at(-1)[0] === "changed", "release command updates on reload");
commands.apply([]);
const launchCount = launched.length;
overlayReleased();
assert(launched.length === launchCount, "removed release command cannot run");
commands.destroy();
assert(activated === null, "destroy disconnects activation listener");
print("PASS: native no-repeat registration, command updates, conflict rollback and lifecycle");

// Buffer the opening gap, preserving editing keys and cancelling safely.
let capture,
    afterUpdate,
    released = 0,
    replayed = [];
const stage = {
    connect(signal, callback) {
        if (signal === "captured-event") capture = callback;
        else afterUpdate = callback;
        return signal === "captured-event" ? 1 : 2;
    },
    disconnect(id) {
        if (id === 2) afterUpdate = null;
    },
    schedule_update() {},
    handle_event(event) {
        event.put();
    },
};
const input = new ShortcutInput(
    stage,
    () => ({ get_seat_state: () => Clutter.GrabState.KEYBOARD }),
    () => released++,
);
function event(symbol, type = Clutter.EventType.KEY_PRESS) {
    return { type: () => type, get_key_symbol: () => symbol, copy: () => ({ put: () => replayed.push(symbol) }) };
}
input.begin("search");
assert(capture(stage, event(97)) === Clutter.EVENT_STOP, "hold typing before popup focus");
capture(stage, event(Clutter.KEY_BackSpace));
assert(
    capture(stage, event(Clutter.KEY_Super_L, Clutter.EventType.KEY_RELEASE)) === Clutter.EVENT_PROPAGATE,
    "never replay shortcut release",
);
input.prepared("search");
assert(released === 1 && replayed.length === 0, "release modal before layer focus, keep input buffered");
capture(stage, event(98));
input.complete("search");
assert(replayed.length === 0, "wait for already queued stage events before handing off");
capture(stage, event(Clutter.KEY_BackSpace));
afterUpdate();
assert(
    JSON.stringify(replayed) === JSON.stringify([97, Clutter.KEY_BackSpace, 98, Clutter.KEY_BackSpace]),
    "replay buffered and already queued editing keys in order",
);
input.begin("search");
assert(input.pending === null, "toggle of focused popup never steals keyboard");
input.closed("search");
input.begin("search");
capture(stage, event(99));
input.complete("search");
input.cancel();
assert(afterUpdate === null, "cancel removes the pending handoff");
assert(replayed.length === 4 && released === 2, "cancel drops input without sending it to previous app");
input.destroy();
print("PASS: popup input handoff");
