// Runs the patched GNOME Shell brightness manager against a fake backlight.
// Needs the patched tree: run after ./build.sh or scripts/apply-patches.sh.
//
//   node --test tests/brightness-follows-backlight.test.mjs
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

const MANAGER = new URL("../subprojects/gnome-shell/js/misc/brightnessManager.js", import.meta.url);
const MIN = 655;
const MAX = 65535;

class GObjectStub {
    constructor() {
        this._handlers = [];
        this._nextId = 1;
    }
    connect(signal, callback) {
        const id = this._nextId++;
        this._handlers.push({ id, signal, callback });
        return id;
    }
    connectObject(...args) {
        args.pop();
        for (let i = 0; i < args.length; i += 2) this.connect(args[i], args[i + 1]);
    }
    disconnect(id) {
        this._handlers = this._handlers.filter((handler) => handler.id !== id);
    }
    emit(signal, ...args) {
        for (const handler of [...this._handlers]) if (handler.signal === signal) handler.callback(this, ...args);
    }
    notify(property) {
        this.emit(`notify::${property}`);
    }
}

class Backlight extends GObjectStub {
    constructor(value) {
        super();
        this.value = value;
        this.brightnessMin = MIN;
        this.brightnessMax = MAX;
    }
    get brightness() {
        return this.value;
    }
    set brightness(value) {
        value = Math.round(value);
        if (value === this.value) return;
        this.value = value;
        this.notify("brightness");
    }
}

// One or more displays, and the manager and keys driving them.
function desktop(initialLevels = [1]) {
    const backlights = initialLevels.map((level) => new Backlight(MIN + level * (MAX - MIN)));
    const logicalMonitors = backlights.map((backlight, index) => {
        const monitor = {
            get_backlight: () => backlight,
            is_active: () => true,
            get_display_name: () => `Display ${index + 1}`,
            get_vendor: () => "vendor",
            get_product: () => `product-${index}`,
            get_serial: () => `serial-${index}`,
            get_color_mode_string: () => "default",
            get_connector: () => `eDP-${index + 1}`,
        };
        return { get_monitors: () => [monitor], get_number: () => index };
    });
    const monitorManager = Object.assign(new GObjectStub(), {
        get_logical_monitors: () => logicalMonitors,
    });
    globalThis.global = {
        backend: {
            get_monitor_manager: () => monitorManager,
            get_current_logical_monitor: () => logicalMonitors[0],
        },
        get_persistent_state: () => null,
        set_persistent_state: () => {},
    };
    Math.clamp ??= (value, low, high) => Math.min(Math.max(value, low), high);
    globalThis._ ??= (text) => text;

    const keys = {};
    const osds = [];
    const environment = {
        GObject: {
            Object: GObjectStub,
            registerClass: (meta, klass) => klass ?? meta,
            ParamSpec: { float: () => null },
            ParamFlags: { READWRITE: 0 },
        },
        Gio: {
            Settings: class {
                get_int() {
                    return 30;
                }
            },
            Icon: { new_for_string: (name) => name },
        },
        GLib: { timeout_add_once: () => 1, source_remove: () => {}, PRIORITY_DEFAULT: 0, Variant: class {} },
        Main: {
            wm: { addKeybinding: (name, _settings, _flags, _modes, handler) => (keys[name] = handler) },
            osdWindowManager: { show: (_icon, _label, levels) => osds.push(levels[0].level) },
        },
        Meta: { KeyBindingFlags: { NONE: 0 } },
        Shell: { ActionMode: { ALL: 0 } },
        SignalTracker: { registerDestroyableType: () => {} },
    };
    const source = readFileSync(MANAGER, "utf8")
        .replace(/^import .*$/gm, "")
        .replace(/^export const /gm, "const ");
    const { BrightnessManager } = new Function(...Object.keys(environment), `${source}\nreturn {BrightnessManager};`)(
        ...Object.values(environment),
    );

    const manager = new BrightnessManager();
    return {
        manager,
        osds,
        press: (key) => keys[key](),
        // e.g. brightnessctl, or a hotkey the firmware handles
        setOutside: (level, index = 0) => (backlights[index].brightness = MIN + level * (MAX - MIN)),
        level: (index = 0) => Math.round(((backlights[index].brightness - MIN) / (MAX - MIN)) * 100) / 100,
    };
}

const laptop = () => desktop([1]);

test(
    "brightness keys step from a level set outside the shell",
    { skip: !existsSync(MANAGER) && "patched gnome-shell tree not prepared" },
    () => {
        const screen = laptop();
        screen.setOutside(0.45);
        assert.equal(screen.manager.globalScale.value.toFixed(2), "0.45");

        screen.press("screen-brightness-up");
        assert.equal(screen.level(), 0.5);
        screen.press("screen-brightness-up");
        assert.equal(screen.level(), 0.55);

        screen.setOutside(0.2);
        screen.press("screen-brightness-down");
        assert.equal(screen.level(), 0.15);
    },
);

test(
    "a level set outside the shell shows no OSD",
    { skip: !existsSync(MANAGER) && "patched gnome-shell tree not prepared" },
    () => {
        const screen = laptop();
        screen.setOutside(0.45);
        screen.setOutside(0.3);
        assert.deepEqual(screen.osds, []);
        screen.press("screen-brightness-up");
        assert.deepEqual(
            screen.osds.map((level) => level.toFixed(2)),
            ["0.35"],
        );
    },
);

test(
    "brightness keys preserve relative levels across displays after an outside change",
    { skip: !existsSync(MANAGER) && "patched gnome-shell tree not prepared" },
    () => {
        const screen = desktop([0.8, 0.4]);
        screen.setOutside(0.6, 0);
        screen.press("screen-brightness-up");

        assert.equal(screen.level(0), 0.65);
        assert.equal(screen.level(1), 0.43);
    },
);
