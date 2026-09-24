import * as Permissions from "./gnoblinPermissions.js";
import * as Corners from "./gnoblinCornerGeometry.js";
import * as Frames from "./gnoblinFramePolicy.js";
// Live shell settings. Protocol registration remains a compositor-startup operation.
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import Clutter from "gi://Clutter";

export const FEATURE_KEYS = Object.freeze([
    "osd",
    "osd-volume",
    "osd-microphone",
    "osd-brightness",
    "osd-keyboard-brightness",
    "osd-pad",
    "screenshot",
    "notifications",
    "input-source-switcher",
]);

export const WINDOW_PREFERENCES = Object.freeze({
    "focus-mode": "click",
    "focus-new-windows": "smart",
    "raise-on-click": true,
    "auto-raise": false,
    "auto-raise-delay": 500,
    "focus-change-on-pointer-rest": false,
    "action-double-click-titlebar": "toggle-maximize",
    "action-middle-click-titlebar": "lower",
    "action-right-click-titlebar": "menu",
    "dynamic-workspaces": false,
    "num-workspaces": 4,
    "workspaces-only-on-primary": false,
    "edge-tiling": false,
    "center-new-windows": false,
    "attach-modal-dialogs": false,
    "workspace-names": [],
});
export const COMPOSITOR_PREFERENCES = Object.freeze({
    "enable-animations": true,
    "locate-pointer": false,
    "visual-bell": false,
    "audible-bell": true,
    "visual-bell-type": "fullscreen-flash",
});
const windowDefaults = () => ({ ...WINDOW_PREFERENCES, "workspace-names": [] });
const TITLEBAR_ACTIONS = new Set([
    "toggle-maximize",
    "toggle-maximize-horizontally",
    "toggle-maximize-vertically",
    "minimize",
    "none",
    "lower",
    "menu",
]);
const INPUT_FIELDS = Object.freeze({
    mouse: {
        speed: "number",
        "left-handed": "boolean",
        "natural-scroll": "boolean",
        "accel-profile": ["default", "flat", "adaptive"],
    },
    touchpad: {
        speed: "number",
        "left-handed": ["right", "left", "mouse"],
        "natural-scroll": "boolean",
        "accel-profile": ["default", "flat", "adaptive"],
        "tap-to-click": "boolean",
        "tap-button-map": ["default", "lrm", "lmr"],
        "tap-and-drag": "boolean",
        "tap-and-drag-lock": "boolean",
        "disable-while-typing": "boolean",
        "edge-scrolling-enabled": "boolean",
        "two-finger-scrolling-enabled": "boolean",
        "click-method": ["default", "none", "areas", "fingers"],
    },
    keyboard: {
        repeat: "boolean",
        delay: "milliseconds",
        "repeat-interval": "milliseconds",
        "remember-numlock-state": "boolean",
        "numlock-state": "boolean",
        "xkb-options": "strings",
    },
    tablets: { mapping: ["absolute", "relative"], "left-handed": "boolean", "keep-aspect": "boolean" },
    styluses: {
        "button-action": ["default", "middle", "right", "back", "forward", "switch-monitor", "keybinding"],
        "secondary-button-action": ["default", "middle", "right", "back", "forward", "switch-monitor", "keybinding"],
        "tertiary-button-action": ["default", "middle", "right", "back", "forward", "switch-monitor", "keybinding"],
        "button-keybinding": "string",
        "secondary-button-keybinding": "string",
        "tertiary-button-keybinding": "string",
    },
});
const isTable = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

function validateInputFields(group, values, path) {
    if (!isTable(values)) throw new Error(`${path}: expected a table`);
    for (const [key, value] of Object.entries(values)) {
        const kind = INPUT_FIELDS[group][key];
        if (!kind) throw new Error(`unknown input setting: ${path}.${key}`);
        const valid = Array.isArray(kind)
            ? kind.includes(value)
            : kind === "boolean"
              ? typeof value === "boolean"
              : kind === "number"
                ? typeof value === "number" && Number.isFinite(value) && value >= -1 && value <= 1
                : kind === "milliseconds"
                  ? Number.isInteger(value) && value >= 1 && value <= 10000
                  : kind === "strings"
                    ? Array.isArray(value) && value.every((item) => typeof item === "string" && !item.includes("\0"))
                    : typeof value === "string" && !value.includes("\0");
        if (!valid) throw new Error(`${path}.${key}: invalid value`);
    }
}

function validateInput(input) {
    if (!isTable(input)) throw new Error("input must be a table");
    for (const [group, values] of Object.entries(input)) {
        if (group === "orientation-lock") {
            if (typeof values !== "boolean") throw new Error("input.orientation-lock: expected a boolean");
        } else if (group === "tablets" || group === "styluses") {
            if (!isTable(values)) throw new Error(`input.${group}: expected a table`);
            for (const [device, fields] of Object.entries(values)) {
                if (
                    !(
                        group === "tablets"
                            ? /^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$/
                            : /^(?:[0-9a-fA-F]+|default-[0-9a-fA-F]{4}:[0-9a-fA-F]{4})$/
                    ).test(device)
                )
                    throw new Error(`input.${group}: invalid device identifier ${device}`);
                validateInputFields(group, fields, `input.${group}.${device}`);
            }
        } else if (Object.hasOwn(INPUT_FIELDS, group)) {
            validateInputFields(group, values, `input.${group}`);
        } else throw new Error(`unknown input group: ${group}`);
    }
}

function validateInputSources(value) {
    if (!isTable(value) || Object.keys(value).some((key) => !["sources", "per-window"].includes(key)))
        throw new Error("input-sources must contain sources and per-window only");
    if (value["per-window"] !== undefined && typeof value["per-window"] !== "boolean")
        throw new Error("input-sources.per-window: expected a boolean");
    if (
        !Array.isArray(value.sources) ||
        !value.sources.every(
            (source) =>
                isTable(source) &&
                Object.keys(source).length === 2 &&
                ["xkb", "ibus"].includes(source.type) &&
                typeof source.id === "string" &&
                source.id.length > 0 &&
                !source.id.includes("\0"),
        )
    )
        throw new Error("input-sources.sources: expected {type, id} records");
}

export const DEFAULTS = Object.freeze({
    ...Object.fromEntries(FEATURE_KEYS.map((key) => [key, null])),
    "window-switcher": false,
    "window-menu": [],
    "minimize-animation": "zoom",
    "minimize-duration": 200,
    "minimize-target": null,
    "layer-animation": "slide",
    "layer-duration": 220,
    "layer-easing": "ease-out-cubic",
    permissions: Permissions.DEFAULT_POLICY,
    autostart: [],
    "window-rules": [],
    shortcuts: [],
    keybindings: {},
});

export let settings = {
    ...DEFAULTS,
    "window-management": windowDefaults(),
    compositor: { ...COMPOSITOR_PREFERENCES },
    input: null,
    "input-sources": null,
};

const WINDOW_RULE_EFFECT_KEYS = Object.freeze([
    "blur",
    "blur-ignore-shadows",
    "opacity",
    "animation",
    "shader",
    "shader-uniforms",
]);
const BORDER_GEOMETRY_KEYS = Object.freeze(["radius", "smoothing", "padding"]);
const STRING_MATCH_KEYS = new Set(["app-id", "title", "layer"]);
const WINDOW_RULE_MATCHERS = new WeakMap();

function windowRuleMatchers(rule) {
    const cached = WINDOW_RULE_MATCHERS.get(rule);
    if (cached?.match === rule.match) return cached.entries;
    const entries = Object.entries(rule.match).map(([key, value]) => [
        key,
        STRING_MATCH_KEYS.has(key) ? new RegExp(value) : value,
    ]);
    WINDOW_RULE_MATCHERS.set(rule, { match: rule.match, entries });
    return entries;
}

function compileWindowRuleMatchers(rules) {
    for (const rule of rules) windowRuleMatchers(rule);
}

export function parseDocument(document) {
    const next = {
        ...DEFAULTS,
        "window-management": windowDefaults(),
        compositor: { ...COMPOSITOR_PREFERENCES },
        input: null,
        "input-sources": null,
    };
    Frames.validateRenderers(document["frame-renderers"]);
    next.permissions = Permissions.validate(document.permissions);
    const windowManagement = document["window-management"] ?? {};
    if (!windowManagement || Array.isArray(windowManagement) || typeof windowManagement !== "object")
        throw new Error("window-management must be a table");
    for (const [key, value] of Object.entries(windowManagement)) {
        if (key === "constrain-drag-to-work-area") {
            if (typeof value !== "boolean") throw new Error(`${key}: expected a boolean`);
        } else if (!Object.hasOwn(WINDOW_PREFERENCES, key)) {
            throw new Error(`unknown window-management setting: ${key}`);
        } else if (key === "focus-mode") {
            if (!["click", "sloppy", "mouse"].includes(value)) throw new Error(`${key}: invalid focus mode`);
        } else if (key === "focus-new-windows") {
            if (!["smart", "strict"].includes(value)) throw new Error(`${key}: invalid focus policy`);
        } else if (key.startsWith("action-") && key.endsWith("-titlebar")) {
            if (!TITLEBAR_ACTIONS.has(value)) throw new Error(`${key}: invalid titlebar action`);
        } else if (key === "auto-raise-delay") {
            if (!Number.isInteger(value) || value < 0 || value > 10000)
                throw new Error(`${key}: expected 0 to 10000 milliseconds`);
        } else if (key === "num-workspaces") {
            if (!Number.isInteger(value) || value < 1 || value > 36)
                throw new Error(`${key}: expected 1 to 36 workspaces`);
        } else if (key === "workspace-names") {
            if (
                !Array.isArray(value) ||
                value.length > 36 ||
                !value.every((name) => typeof name === "string" && name.length <= 80 && !name.includes("\0"))
            )
                throw new Error(`${key}: expected up to 36 names of at most 80 characters`);
        } else if (typeof value !== "boolean") {
            throw new Error(`${key}: expected a boolean`);
        }
        next["window-management"][key] = value;
    }
    const compositor = document.compositor ?? {};
    if (!compositor || Array.isArray(compositor) || typeof compositor !== "object")
        throw new Error("compositor must be a table");
    for (const [key, value] of Object.entries(compositor)) {
        if (!Object.hasOwn(COMPOSITOR_PREFERENCES, key)) throw new Error(`unknown compositor setting: ${key}`);
        if (key === "visual-bell-type") {
            if (!["fullscreen-flash", "frame-flash"].includes(value))
                throw new Error(`${key}: expected fullscreen-flash or frame-flash`);
        } else if (typeof value !== "boolean") {
            throw new Error(`${key}: expected a boolean`);
        }
        next.compositor[key] = value;
    }
    if (document.input !== undefined) {
        validateInput(document.input);
        next.input = document.input;
    }
    if (document["input-sources"] !== undefined) {
        validateInputSources(document["input-sources"]);
        next["input-sources"] = document["input-sources"];
    }
    const shell = document.shell ?? {};
    if (!shell || Array.isArray(shell) || typeof shell !== "object") throw new Error("shell must be a table");
    for (const [key, value] of Object.entries(shell)) {
        if (
            !Object.hasOwn(DEFAULTS, key) ||
            ["autostart", "window-rules", "shortcuts", "keybindings", "permissions"].includes(key)
        )
            throw new Error(`unknown shell setting: ${key}`);
        if (key === "window-menu") {
            if (
                !Array.isArray(value) ||
                value.length > 32 ||
                !value.every((arg) => typeof arg === "string" && !arg.includes("\0")) ||
                (value.length && !value[0])
            )
                throw new Error("window-menu: expected a command argv or an empty array");
        } else if (FEATURE_KEYS.includes(key) || key === "window-switcher") {
            if (typeof value !== "boolean") throw new Error(`${key}: expected a boolean`);
        } else if (key === "minimize-animation") {
            if (!["zoom", "fade", "none", "gnome"].includes(value))
                throw new Error(`${key}: expected zoom, fade, none, or gnome`);
        } else if (key === "layer-animation") {
            if (!["slide", "fade", "none"].includes(value)) throw new Error(`${key}: expected slide, fade, or none`);
        } else if (key === "layer-easing") {
            if (!["ease-out-cubic", "ease-out-quad", "ease-in-out-cubic", "linear"].includes(value))
                throw new Error(`${key}: unsupported easing`);
        } else if (key === "minimize-duration" || key === "layer-duration") {
            if (!Number.isInteger(value) || value < 0 || value > 5000)
                throw new Error(`${key}: expected 0 to 5000 milliseconds`);
        } else if (key === "minimize-target") {
            if (
                !Array.isArray(value) ||
                value.length !== 2 ||
                !value.every((n) => Number.isInteger(n) && Math.abs(n) <= 1000000)
            )
                throw new Error(`${key}: expected [x, y] in logical desktop coordinates`);
        }
        next[key] = value;
    }
    if (
        document.protocols !== undefined &&
        (!document.protocols ||
            Array.isArray(document.protocols) ||
            typeof document.protocols !== "object" ||
            !Object.values(document.protocols).every((value) => typeof value === "boolean"))
    )
        throw new Error("protocols must be a table of booleans");
    const entries = document.autostart ?? [];
    if (!Array.isArray(entries)) throw new Error("autostart must use [[autostart]] tables");
    const names = new Set();
    for (const entry of entries) {
        if (
            !entry ||
            typeof entry !== "object" ||
            Array.isArray(entry) ||
            Object.keys(entry).some((key) => !["name", "command"].includes(key)) ||
            typeof entry.name !== "string" ||
            !entry.name.trim() ||
            names.has(entry.name) ||
            !Array.isArray(entry.command) ||
            entry.command.length === 0 ||
            !entry.command.every((arg) => typeof arg === "string" && !arg.includes("\0")) ||
            !entry.command[0]
        )
            throw new Error("autostart requires a unique name and a nonempty command array");
        names.add(entry.name);
    }
    next.autostart = entries;
    const rules = document["window-rules"] ?? [];
    if (!Array.isArray(rules)) throw new Error("window-rules must use [[window-rules]] tables");
    for (const rule of rules) {
        if (
            !rule ||
            !rule.match ||
            Array.isArray(rule.match) ||
            typeof rule.match !== "object" ||
            Object.keys(rule.match).length === 0 ||
            Object.keys(rule).some(
                (key) =>
                    ![
                        "match",
                        "blur",
                        "blur-ignore-shadows",
                        "opacity",
                        "animation",
                        "shader",
                        "shader-uniforms",
                        "corners",
                        "borders",
                        "frame",
                    ].includes(key),
            )
        )
            throw new Error("window rule requires match and supported effects");
        for (const [key, value] of Object.entries(rule.match)) {
            if (key === "focused") {
                if (typeof value !== "boolean") throw new Error("focused match must be boolean");
            } else if (["app-id", "title", "layer"].includes(key)) {
                if (typeof value !== "string" || value.length > 512)
                    throw new Error("rule matcher must be a regex string");
            } else if (key !== "type" || !["layer", "window"].includes(value)) {
                throw new Error("unknown window rule match");
            }
        }
        if (rule.corners !== undefined) Corners.validate(rule.corners);
        if (rule.borders !== undefined) Corners.validateBorders(rule.borders);
        if (rule.frame !== undefined) Frames.validate(rule.frame);
        if (rule.blur !== undefined && (!Number.isInteger(rule.blur) || rule.blur < 0 || rule.blur > 100))
            throw new Error("blur must be an integer from 0 to 100");
        if (rule["blur-ignore-shadows"] !== undefined && typeof rule["blur-ignore-shadows"] !== "boolean")
            throw new Error("blur-ignore-shadows must be boolean");
        if (
            rule.opacity !== undefined &&
            (typeof rule.opacity !== "number" || !Number.isFinite(rule.opacity) || rule.opacity < 0 || rule.opacity > 1)
        )
            throw new Error("opacity must be between 0 and 1");
        if (rule.animation !== undefined) {
            const animation = rule.animation;
            if (typeof animation === "string") {
                if (!["slide", "fade", "none"].includes(animation))
                    throw new Error("rule animation must be slide, fade, or none");
            } else {
                if (
                    !animation ||
                    Array.isArray(animation) ||
                    typeof animation !== "object" ||
                    Object.keys(animation).some((key) => !["in", "out", "duration", "easing"].includes(key)) ||
                    ["in", "out"].some(
                        (key) => animation[key] !== undefined && !["slide", "fade", "none"].includes(animation[key]),
                    ) ||
                    (animation.duration !== undefined &&
                        (!Number.isInteger(animation.duration) ||
                            animation.duration < 0 ||
                            animation.duration > 5000)) ||
                    (animation.easing !== undefined &&
                        !["ease-out-cubic", "ease-out-quad", "ease-in-out-cubic", "linear"].includes(animation.easing))
                )
                    throw new Error("invalid layer animation policy");
            }
        }
        if (
            rule.shader !== undefined &&
            (typeof rule.shader !== "string" || rule.shader.length > 4096 || rule.shader.includes("\0"))
        )
            throw new Error("shader must be a file path, or an empty string to remove it");
        if (rule["shader-uniforms"] !== undefined) {
            const uniforms = rule["shader-uniforms"];
            if (
                !uniforms ||
                typeof uniforms !== "object" ||
                Array.isArray(uniforms) ||
                Object.keys(uniforms).length > 64 ||
                Object.entries(uniforms).some(
                    ([name, value]) =>
                        !/^[a-zA-Z_]\w*$/.test(name) ||
                        name.startsWith("gnoblin_") ||
                        typeof value !== "number" ||
                        !Number.isFinite(value) ||
                        Math.abs(value) > 3.4e38,
                )
            )
                throw new Error(
                    "shader-uniforms must contain up to 64 named finite floats; gnoblin_ names are reserved",
                );
        }
    }
    // Compile validated regular expressions once per installed configuration.
    // The WeakMap also recompiles when a replacement configuration supplies
    // new rule or match objects.
    compileWindowRuleMatchers(rules);
    next["window-rules"] = rules;
    Object.assign(next, validateShortcuts(document));
    return next;
}

export const KEYBINDING_SCHEMAS = Object.freeze({
    shell: "org.gnome.shell.keybindings",
    wm: "org.gnome.desktop.wm.keybindings",
    mutter: "org.gnome.mutter.keybindings",
    wayland: "org.gnome.mutter.wayland.keybindings",
});
const gsettingsKey = (key) => key.replaceAll("_", "-");

function acceleratorIdentity(value) {
    if (value === "Super") return "overlay-key";
    if (
        typeof value !== "string" ||
        !value ||
        value.length > 160 ||
        !/^(?:<(?:Shift|Control|Ctrl|Primary|Alt|Mod1|Super|Meta|Hyper|Mod[2-5])>)*[A-Za-z0-9_]+$/i.test(value)
    )
        throw new Error(`invalid shortcut accelerator: ${JSON.stringify(value)}; use <Alt>s or <Super>Return`);
    const key = value.replace(/<[^>]+>/g, "").replace(/^XF86/, "");
    if (typeof Clutter[`KEY_${key}`] !== "number") throw new Error(`unknown shortcut key: ${key}`);
    const modifiers = [...value.matchAll(/<([^>]+)>/g)].map(([, modifier]) =>
        modifier
            .toLowerCase()
            .replace(/^(ctrl|primary)$/, "control")
            .replace(/^mod1$/, "alt"),
    );
    return [...new Set(modifiers)].sort().join("+") + "+" + key.toLowerCase();
}

export function validateShortcuts(document) {
    const declarations = document.shortcuts ?? [];
    const shortcuts = [];
    const keybindings = Object.fromEntries(
        Object.entries(document.keybindings ?? {}).map(([group, entries]) => [
            group,
            entries && typeof entries === "object" && !Array.isArray(entries) ? { ...entries } : entries,
        ]),
    );
    if (!Array.isArray(declarations) || declarations.length > 256)
        throw new Error("shortcuts must use [[shortcuts]] tables (maximum 256)");
    const names = new Set(),
        accelerators = new Set();
    const declaredActions = new Set();
    for (const entry of declarations) {
        if (
            !entry ||
            typeof entry !== "object" ||
            Array.isArray(entry) ||
            Object.keys(entry).some(
                (key) => !["name", "binding", "command", "action", "capture-input"].includes(key),
            ) ||
            (entry["capture-input"] !== undefined && typeof entry["capture-input"] !== "boolean") ||
            typeof entry.name !== "string" ||
            !/^[a-zA-Z0-9_-]{1,80}$/.test(entry.name) ||
            names.has(entry.name) ||
            (entry.action === undefined) === (entry.command === undefined)
        )
            throw new Error("shortcut requires a unique name and exactly one of action or command");
        names.add(entry.name);
        if (entry.action !== undefined) {
            const match = /^(gnome:shell|wm|mutter|wayland)\.([a-z0-9]+(?:_[a-z0-9]+)*)$/.exec(entry.action);
            if (!match || !Array.isArray(entry.binding))
                throw new Error('built-in shortcut requires action = "group.action" and a binding list');
            const [, namespace, key] = match;
            const group = namespace === "gnome:shell" ? "shell" : namespace;
            const actionKey = `${group}.${key}`;
            if (declaredActions.has(actionKey) || Object.hasOwn(keybindings[group] ?? {}, key))
                throw new Error(`built-in shortcut action is configured more than once: ${entry.action}`);
            declaredActions.add(actionKey);
            const schema = Gio.SettingsSchemaSource.get_default().lookup(KEYBINDING_SCHEMAS[group], true);
            const nativeKey = gsettingsKey(key);
            if (!schema?.has_key(nativeKey) || schema.get_key(nativeKey).get_value_type().dup_string() !== "as")
                throw new Error(`unknown built-in shortcut action: ${entry.action}`);
            for (const binding of entry.binding) {
                const identity = acceleratorIdentity(binding);
                if (accelerators.has(identity)) throw new Error(`duplicate shortcut: ${binding}`);
                accelerators.add(identity);
            }
            keybindings[group] ??= {};
            keybindings[group][key] = entry.binding;
        } else {
            if (
                !Array.isArray(entry.command) ||
                !entry.command.length ||
                !entry.command[0] ||
                !entry.command.every((arg) => typeof arg === "string" && !arg.includes("\0"))
            )
                throw new Error("command shortcut requires a nonempty command array");
            const identity = acceleratorIdentity(entry.binding);
            if (accelerators.has(identity)) throw new Error(`duplicate shortcut: ${entry.binding}`);
            accelerators.add(identity);
            shortcuts.push(entry);
        }
    }
    if (!keybindings || typeof keybindings !== "object" || Array.isArray(keybindings))
        throw new Error("keybindings must be a table");
    const source = Gio.SettingsSchemaSource.get_default();
    for (const [group, entries] of Object.entries(keybindings)) {
        const schema = KEYBINDING_SCHEMAS[group] && source.lookup(KEYBINDING_SCHEMAS[group], true);
        if (!schema || !entries || typeof entries !== "object" || Array.isArray(entries))
            throw new Error(`unknown keybinding group: ${group}`);
        for (const [key, bindings] of Object.entries(entries)) {
            if (declaredActions.has(`${group}.${key}`)) continue;
            if (!/^[a-z0-9]+(?:_[a-z0-9]+)*$/.test(key))
                throw new Error(`invalid keybinding name: ${group}.${key}; use snake_case`);
            const nativeKey = gsettingsKey(key);
            if (
                !schema.has_key(nativeKey) ||
                schema.get_key(nativeKey).get_value_type().dup_string() !== "as" ||
                !Array.isArray(bindings)
            )
                throw new Error(`unknown or unsupported keybinding: ${group}.${key}`);
            for (const binding of bindings) {
                const identity = acceleratorIdentity(binding);
                if (accelerators.has(identity)) throw new Error(`duplicate shortcut: ${binding}`);
                accelerators.add(identity);
            }
        }
    }
    return { shortcuts, keybindings };
}

// Gnoblin owns command shortcut registration in the compositor. Built-in
// keybinding overrides use Mutter's prefs API below and never touch GSettings.
export class Shortcuts {
    constructor(commands, prefs = Meta) {
        if (!commands || typeof commands.apply !== "function")
            throw new Error("native command shortcut registration is required");
        this.commands = commands;
        this.prefs = prefs;
        this.keybindings = {};
    }

    apply(config) {
        const { shortcuts, keybindings } = validateShortcuts(config);
        try {
            this.prefs.prefs_apply_gnoblin_keybindings(keybindingVariant(keybindings));
            this.commands.apply(shortcuts);
            this.keybindings = keybindings;
        } catch (error) {
            this.prefs.prefs_apply_gnoblin_keybindings(keybindingVariant(this.keybindings));
            throw error;
        }
    }

    destroy() {
        this.commands?.destroy();
    }
}

function keybindingVariant(groups) {
    return new GLib.Variant(
        "a{sv}",
        Object.fromEntries(
            Object.entries(groups).map(([group, entries]) => [
                group,
                new GLib.Variant(
                    "a{sv}",
                    Object.fromEntries(
                        Object.entries(entries).map(([key, bindings]) => [
                            gsettingsKey(key),
                            new GLib.Variant("as", bindings),
                        ]),
                    ),
                ),
            ]),
        ),
    );
}

export function applyWindowPreferences(preferences) {
    const values = Object.fromEntries(
        Object.entries(WINDOW_PREFERENCES).map(([key, fallback]) => [
            key,
            new GLib.Variant(
                Array.isArray(fallback)
                    ? "as"
                    : typeof fallback === "boolean"
                      ? "b"
                      : typeof fallback === "number"
                        ? "i"
                        : "s",
                preferences[key] ?? fallback,
            ),
        ]),
    );
    Meta.prefs_apply_gnoblin_window_preferences(new GLib.Variant("a{sv}", values));
}

export function applyCompositorPreferences(preferences) {
    const values = Object.fromEntries(
        Object.entries(COMPOSITOR_PREFERENCES).map(([key, fallback]) => [
            key,
            new GLib.Variant(typeof fallback === "boolean" ? "b" : "s", preferences[key] ?? fallback),
        ]),
    );
    Meta.prefs_apply_gnoblin_compositor_preferences(new GLib.Variant("a{sv}", values));
}

function inputVariant(group, values) {
    return new GLib.Variant(
        "a{sv}",
        Object.fromEntries(
            Object.entries(values).map(([key, value]) => {
                if (group === "tablets" || group === "styluses")
                    return [key, inputVariant(group === "tablets" ? "tablet" : "stylus", value)];
                const type = Array.isArray(value)
                    ? "as"
                    : typeof value === "boolean"
                      ? "b"
                      : typeof value === "number"
                        ? key === "speed"
                            ? "d"
                            : "u"
                        : "s";
                return [key, new GLib.Variant(type, value)];
            }),
        ),
    );
}

export function applyInputPreferences(input) {
    const groups = Object.fromEntries(
        Object.entries(input ?? {})
            .filter(([key]) => key !== "orientation-lock")
            .map(([key, value]) => [key, inputVariant(key, value)]),
    );
    global.display.apply_gnoblin_input_config(input === null ? null : new GLib.Variant("a{sv}", groups));
    Meta.prefs_apply_gnoblin_keyboard_preferences(
        input?.keyboard?.["xkb-options"] !== undefined
            ? inputVariant("keyboard", { "xkb-options": input.keyboard["xkb-options"] })
            : null,
    );
    const orientationManager = global.backend.get_orientation_manager();
    if (input && Object.hasOwn(input, "orientation-lock"))
        orientationManager.set_orientation_locked(input["orientation-lock"]);
    else orientationManager.clear_orientation_lock_override();
}

// Commands are edge-triggered, not text input: holding a shortcut must never
// repeatedly launch it (or toggle a popup straight back closed).
export class CommandShortcuts {
    constructor(
        display,
        allow,
        launch = (command) => {
            const child = Gio.Subprocess.new(command, Gio.SubprocessFlags.NONE);
            child.wait_check_async(null, (process, result) => {
                try {
                    process.wait_check_finish(result);
                } catch (error) {
                    console.warn(`gnoblin-shortcut: ${error.message}`);
                }
            });
        },
        prepareInput = () => {},
    ) {
        this.display = display;
        this.prepareInput = prepareInput;
        this.allow = allow;
        this.launch = launch;
        this.bindings = new Map();
        this.overlaySignal = display.connect("overlay-key", () => {
            const binding = this.bindings.get("overlay-key");
            if (!binding) return;
            try {
                if (binding.entry["capture-input"]) this.prepareInput(binding.entry.name);
                this.launch(binding.entry.command);
            } catch (error) {
                console.warn(`gnoblin-shortcut ${binding.entry.name}: ${error.message}`);
            }
        });
        this.signal = display.connect("accelerator-activated", (_display, action) => {
            const binding = [...this.bindings.values()].find((item) => item.action === action);
            if (!binding) return;
            try {
                if (binding.entry["capture-input"]) this.prepareInput(binding.entry.name);
                this.launch(binding.entry.command);
            } catch (error) {
                console.warn(`gnoblin-shortcut ${binding.entry.name}: ${error.message}`);
            }
        });
    }

    apply(entries) {
        const next = new Map(),
            added = [];
        try {
            for (const entry of entries) {
                const identity = acceleratorIdentity(entry.binding);
                let action = this.bindings.get(identity)?.action;
                if (action === undefined) {
                    action =
                        identity === "overlay-key"
                            ? "overlay-key"
                            : this.display.grab_accelerator(entry.binding, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT);
                    if (action === Meta.KeyBindingAction.NONE)
                        throw new Error(`shortcut already claimed: ${entry.binding} (${entry.name})`);
                    added.push(action);
                    this.allow(action, true);
                }
                next.set(identity, { action, entry });
            }
        } catch (error) {
            for (const action of added) this.release(action);
            throw error;
        }
        for (const [identity, binding] of this.bindings) {
            if (!next.has(identity)) this.release(binding.action);
        }
        this.bindings = next;
    }

    release(action) {
        if (action !== "overlay-key") this.display.ungrab_accelerator(action);
        this.allow(action, false);
    }

    destroy() {
        this.display.disconnect(this.signal);
        this.display.disconnect(this.overlaySignal);
        for (const binding of this.bindings.values()) this.release(binding.action);
        this.bindings.clear();
    }
}

// Buffer native key events only while a configured popup is taking focus.
// The compositor already distinguishes a bare Super release from a Super chord.
export class ShortcutInput {
    constructor(stage, grab, ungrab) {
        this.stage = stage;
        this.grabKeyboard = grab;
        this.ungrabKeyboard = ungrab;
        this.completion = 0;
        this.ready = new Set();
        this.pending = null;
        this.timeout = 0;
        this.signal = stage.connect("captured-event", (_stage, event) => {
            if (!this.pending) return Clutter.EVENT_PROPAGATE;
            const type = event.type();
            if (![Clutter.EventType.KEY_PRESS, Clutter.EventType.KEY_RELEASE].includes(type))
                return Clutter.EVENT_PROPAGATE;
            // The release that triggered overlay-key can reach the stage after
            // begin(). Replaying it would activate the shortcut a second time.
            if ([Clutter.KEY_Super_L, Clutter.KEY_Super_R].includes(event.get_key_symbol()))
                return Clutter.EVENT_PROPAGATE;
            if (this.pending.events.length >= 1024) {
                this.cancel();
                return Clutter.EVENT_PROPAGATE;
            }
            this.pending.events.push(event.copy());
            return Clutter.EVENT_STOP;
        });
    }
    begin(name) {
        if (this.ready.has(name)) return; // A toggle is closing an already focused popup.
        this.cancel();
        const grab = this.grabKeyboard();
        if (!(grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
            this.ungrabKeyboard(grab);
            return;
        }
        this.pending = { name, events: [], grab };
        this.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
            this.timeout = 0;
            this.cancel();
            return GLib.SOURCE_REMOVE;
        });
    }
    prepared(name) {
        if (this.pending?.name !== name || !this.pending.grab) return;
        this.ungrabKeyboard(this.pending.grab);
        this.pending.grab = null;
    }
    complete(name) {
        this.ready.add(name);
        if (this.pending?.name !== name || this.completion) return;
        // Keys already filtered into the stage queue still belong to our grab.
        // Finish that update before handing off, then dispatch the buffered keys
        // synchronously so newer input cannot overtake an editing key.
        this.completion = this.stage.connect("after-update", () => {
            this.stage.disconnect(this.completion);
            this.completion = 0;
            this.prepared(name);
            const events = this.pending.events;
            this.pending = null;
            if (this.timeout) GLib.source_remove(this.timeout);
            this.timeout = 0;
            for (const event of events) this.stage.handle_event(event);
        });
        this.stage.schedule_update();
    }
    closed(name) {
        this.ready.delete(name);
        if (this.pending?.name === name) this.cancel();
    }
    cancel() {
        if (this.completion) this.stage.disconnect(this.completion);
        this.completion = 0;
        if (this.timeout) GLib.source_remove(this.timeout);
        this.timeout = 0;
        if (this.pending?.grab) this.ungrabKeyboard(this.pending.grab);
        this.pending = null;
    }
    destroy() {
        this.cancel();
        this.stage.disconnect(this.signal);
        this.ready.clear();
    }
}

// The dock hint takes precedence over the optional fallback coordinate.
export function minimizeTarget(window, monitor) {
    const [success, rect] = window.get_icon_geometry();
    if (success) return [true, rect];
    const point = settings["minimize-target"];
    if (!point && !monitor) return [false, null];
    return [
        true,
        {
            x: point ? point[0] : monitor.x + monitor.width / 2,
            y: point ? point[1] : monitor.y + monitor.height,
            width: 0,
            height: 0,
        },
    ];
}

export class Autostart {
    constructor() {
        this._started = new Set();
    }

    apply(entries) {
        for (const { name, command } of entries) {
            if (this._started.has(name)) continue;
            try {
                const child = Gio.Subprocess.new(command, Gio.SubprocessFlags.NONE);
                this._started.add(name);
                child.wait_async(null, (process, result) => {
                    try {
                        process.wait_finish(result);
                        if (!process.get_successful()) console.warn(`gnoblin-autostart: ${name} exited unsuccessfully`);
                    } catch (e) {
                        console.warn(`gnoblin-autostart: ${name}: ${e.message}`);
                    }
                });
                console.log(`gnoblin-autostart: started ${name}`);
            } catch (e) {
                console.warn(`gnoblin-autostart: could not start ${name}: ${e.message}`);
            }
        }
    }
}

function cloneDocument(document) {
    return JSON.parse(
        JSON.stringify(document, (_key, value) => {
            if (
                ["undefined", "function", "symbol", "bigint"].includes(typeof value) ||
                (typeof value === "number" && !Number.isFinite(value))
            )
                throw new TypeError("Configuration values must be finite numbers, strings, booleans, arrays or tables");
            return value;
        }),
    );
}

export class ConfigFile {
    constructor(path = null, apply = () => {}) {
        this._override = path || GLib.getenv("GNOBLIN_CONFIG") || null;
        this._directory = this._override
            ? GLib.path_get_dirname(this._override)
            : GLib.build_filenamev([GLib.get_user_config_dir(), "gnoblin"]);
        this._apply = apply;
        this._started = false;
        this._monitors = new Map();
        this._watchedFiles = new Set();
        this._watchedDirectories = new Set();
        this._timeout = 0;
    }

    get path() {
        if (this._override) return this._override;
        for (const name of ["init.lua", "gnoblin.toml", "gnoblin.conf"]) {
            const candidate = GLib.build_filenamev([this._directory, name]);
            if (GLib.file_test(candidate, GLib.FileTest.EXISTS)) return candidate;
        }
        return GLib.build_filenamev([this._directory, "init.lua"]);
    }

    reload() {
        const path = this.path;
        let next;
        // Mutter and Shell evaluate the same Lua files, in the same order.
        // Keep failed dependencies watched so fixing a module retries it.
        const loaded = Meta.gnoblin_load_config(path).recursiveUnpack();
        this._setWatchedFiles(loaded.paths ?? [path], loaded.directories ?? []);
        if (loaded.error) throw new Error(loaded.error);
        try {
            next = parseDocument(loaded.document);
        } catch (error) {
            throw new Error(`${path}: ${error.message}`);
        }
        const services = Object.fromEntries(
            Object.entries(loaded.document["frame-renderers"] ?? {}).map(([name, argv]) => [
                name,
                new GLib.Variant("as", argv),
            ]),
        );
        Meta.gnoblin_frame_renderers_configure(new GLib.Variant("a{sv}", services), true);
        this._apply(next);
        settings = next;
        this._document = cloneDocument(loaded.document);
        this._liveUndo = [];
    }

    document() {
        return cloneDocument(this._document ?? {});
    }

    applyLive(document) {
        document = cloneDocument(document);
        const next = parseDocument(document);
        const previous = this.document();
        const stable = (value) =>
            JSON.stringify(value, function (_key, item) {
                return item && !Array.isArray(item) && typeof item === "object"
                    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)))
                    : item;
            });
        const live = new Set([
            "cursor",
            "window-management",
            "compositor",
            "input",
            "input-sources",
            "shell",
            "window-rules",
            "shortcuts",
            "keybindings",
            "permissions",
        ]);
        for (const key of new Set([...Object.keys(previous), ...Object.keys(document)])) {
            if (!live.has(key) && stable(previous[key]) !== stable(document[key]))
                throw new Error(`${key}: edit the config file and reload; protocol changes need a new session`);
        }
        for (const key of FEATURE_KEYS) {
            if (next[key] !== settings[key])
                throw new Error(`${key}: this feature uses saved desktop preferences; edit the config file and reload`);
        }
        this._apply(next);
        this._liveUndo ??= [];
        this._liveUndo.push(previous);
        if (this._liveUndo.length > 50) this._liveUndo.shift();
        this._document = document;
        settings = next;
        return this.document();
    }

    undoLive() {
        const previous = this._liveUndo?.at(-1);
        if (!previous) throw new Error("No live configuration change to undo");
        const next = parseDocument(previous);
        this._apply(next);
        this._liveUndo.pop();
        this._document = previous;
        settings = next;
        return this.document();
    }

    start() {
        if (this._started) return;
        const parent = Gio.File.new_for_path(this._directory);
        GLib.mkdir_with_parents(parent.get_path(), 0o700);
        this._setWatchedFiles([this.path]);
        this._started = true;
        this._tryReload();
    }

    _setWatchedFiles(paths, watchedDirectories = []) {
        const wanted = new Set(paths.map((path) => GLib.canonicalize_filename(path, null)));
        this._watchedDirectories = new Set(watchedDirectories.map((path) => GLib.canonicalize_filename(path, null)));
        const directories = new Set();
        const parents = [...wanted].map((path) => GLib.path_get_dirname(path));
        const targets = [...parents, ...this._watchedDirectories];
        // Parents survive an atomic replacement of a watched directory.
        for (let directory of [...targets, ...targets.map((path) => GLib.path_get_dirname(path))]) {
            while (!GLib.file_test(directory, GLib.FileTest.IS_DIR)) {
                const parent = GLib.path_get_dirname(directory);
                if (parent === directory) break;
                directory = parent;
            }
            directories.add(directory);
        }
        for (const [directory, monitor] of this._monitors) {
            if (directories.has(directory)) continue;
            monitor.cancel();
            this._monitors.delete(directory);
        }
        for (const directory of directories) {
            if (this._monitors.has(directory)) continue;
            const monitor = Gio.File.new_for_path(directory).monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
            monitor.connect("changed", (_monitor, file, otherFile) => {
                const changed = [file, otherFile]
                    .filter(Boolean)
                    .map((item) => GLib.canonicalize_filename(item.get_path(), null));
                if (
                    !changed.some(
                        (path) =>
                            [...this._watchedFiles].some(
                                (watched) => watched === path || watched.startsWith(`${path}/`),
                            ) ||
                            [...this._watchedDirectories].some(
                                (directory) =>
                                    directory === path ||
                                    path.startsWith(`${directory}/`) ||
                                    directory.startsWith(`${path}/`),
                            ),
                    )
                )
                    return;
                for (const path of changed) {
                    for (const [watched, oldMonitor] of this._monitors) {
                        if (watched === path || watched.startsWith(`${path}/`)) {
                            oldMonitor.cancel();
                            this._monitors.delete(watched);
                        }
                    }
                }
                if (this._timeout) GLib.source_remove(this._timeout);
                this._timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
                    this._timeout = 0;
                    this._tryReload();
                    return GLib.SOURCE_REMOVE;
                });
            });
            this._monitors.set(directory, monitor);
        }
        this._watchedFiles = wanted;
    }

    _tryReload() {
        try {
            this.reload();
            console.log(`gnoblin-config: loaded ${this.path}`);
        } catch (e) {
            console.warn(`gnoblin-config: keeping last valid settings: ${e.message}`);
        }
    }

    destroy() {
        if (this._timeout) GLib.source_remove(this._timeout);
        this._timeout = 0;
        for (const monitor of this._monitors.values()) monitor.cancel();
        this._monitors.clear();
        this._watchedFiles.clear();
        this._watchedDirectories.clear();
        this._started = false;
    }
}

// Opposing anchors cancel on an axis; corners move diagonally. Full-screen
// overlays fade, since their transparent input surface has no single edge.
export function layerOffset(anchor, rect, monitor) {
    if (!monitor) return [0, 0];
    const top = Boolean(anchor & 1),
        bottom = Boolean(anchor & 2);
    const left = Boolean(anchor & 4),
        right = Boolean(anchor & 8);
    const x = left === right ? 0 : left ? monitor.x - rect.x - rect.width : monitor.x + monitor.width - rect.x;
    const y = top === bottom ? 0 : top ? monitor.y - rect.y - rect.height : monitor.y + monitor.height - rect.y;
    return [x, y];
}

// Later matching rules override individual effects, leaving others intact.
export function windowEffects(properties, config = settings, defaultBlur = 0) {
    const effects = {
        borders: { ...Corners.borderDefaults },
        corners: { ...Corners.defaults },
        blur: defaultBlur,
        "blur-ignore-shadows": false,
        opacity: 1,
        animation: config["layer-animation"],
        shader: "",
        "shader-uniforms": {},
    };
    effects.frame = { ...Frames.defaults };
    let borderGeometryOverrides = null;
    for (const rule of config["window-rules"]) {
        let matches = true;
        for (const [key, matcher] of windowRuleMatchers(rule)) {
            if (
                key === "type" || key === "focused"
                    ? properties[key] !== matcher
                    : properties[key] === null || !matcher.test(properties[key] ?? "")
            ) {
                matches = false;
                break;
            }
        }
        if (matches) {
            if (rule.frame !== undefined) effects.frame = { ...effects.frame, ...rule.frame };
            if (rule.corners !== undefined) effects.corners = Corners.merge(effects.corners, rule.corners);
            if (rule.borders !== undefined) {
                effects.borders = { ...effects.borders, ...rule.borders };
                for (const key of BORDER_GEOMETRY_KEYS) {
                    if (Object.hasOwn(rule.borders, key)) {
                        borderGeometryOverrides ??= new Set();
                        borderGeometryOverrides.add(key);
                    }
                }
            }
            for (const key of WINDOW_RULE_EFFECT_KEYS) if (rule[key] !== undefined) effects[key] = rule[key];
        }
    }
    // Keep the common case visually aligned: a border follows the clip's
    // geometry unless a border rule explicitly opts into its own shape.
    for (const key of BORDER_GEOMETRY_KEYS)
        if (!borderGeometryOverrides?.has(key))
            effects.borders[key] = Array.isArray(effects.corners[key])
                ? [...effects.corners[key]]
                : effects.corners[key];
    return effects;
}

export function windowProperties(window) {
    const namespace = Meta.gnoblin_layer_namespace(window);
    return {
        "app-id": window.get_gtk_application_id() || window.get_wm_class() || "",
        title: window.get_title() || "",
        layer: namespace,
        type: namespace !== null ? "layer" : "window",
        focused: window.has_focus(),
    };
}

// Resolve each phase independently; string rules retain their existing meaning.
export function layerAnimation(properties, opening, config = settings) {
    const rule = windowEffects(properties, config).animation;
    return {
        animation: typeof rule === "string" ? rule : (rule[opening ? "in" : "out"] ?? config["layer-animation"]),
        duration: typeof rule === "object" ? (rule.duration ?? config["layer-duration"]) : config["layer-duration"],
        easing: typeof rule === "object" ? (rule.easing ?? config["layer-easing"]) : config["layer-easing"],
    };
}
