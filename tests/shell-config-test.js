// Lua evaluation belongs to Mutter. This suite checks the Shell schema that
// receives its resulting document.
import {
    DEFAULTS,
    parseDocument,
    minimizeTarget,
    layerOffset,
    windowEffects,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js";

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

assert(Object.entries(DEFAULTS).every(([key, value]) => JSON.stringify(parseDocument({})[key]) === JSON.stringify(value)),
    "missing Lua keys use defaults");
const windowPrefs = parseDocument({ "window-management": {
    "focus-mode": "sloppy", "action-middle-click-titlebar": "minimize", "edge-tiling": true,
    "workspace-names": ["Main", "Chat"],
} })["window-management"];
assert(windowPrefs["focus-mode"] === "sloppy" && windowPrefs["action-middle-click-titlebar"] === "minimize" &&
    windowPrefs["edge-tiling"] && windowPrefs["num-workspaces"] === 4 &&
    windowPrefs["workspace-names"].join(",") === "Main,Chat",
    "window policy merges configured values with Gnoblin defaults");
const compositor = parseDocument({ compositor: {
    "enable-animations": false, "locate-pointer": true, "visual-bell": true,
    "audible-bell": false, "visual-bell-type": "frame-flash",
} }).compositor;
assert(!compositor["enable-animations"] && compositor["locate-pointer"] && compositor["visual-bell"] &&
    !compositor["audible-bell"] && compositor["visual-bell-type"] === "frame-flash",
    "compositor interaction preferences accept supported values");
const input = parseDocument({
    input: {
        mouse: { speed: -0.25, "left-handed": true, "accel-profile": "flat" },
        touchpad: { "tap-to-click": true, "click-method": "fingers", "left-handed": "mouse" },
        keyboard: { repeat: true, delay: 500, "repeat-interval": 30, "xkb-options": ["caps:escape"] },
        "orientation-lock": true,
        tablets: { "1234:5678": { mapping: "absolute", "keep-aspect": true } },
        styluses: { "default-1234:5678": { "button-action": "keybinding", "button-keybinding": "<Super>p" } },
    },
    "input-sources": { sources: [{ type: "xkb", id: "us" }, { type: "ibus", id: "anthy" }], "per-window": true },
});
assert(input.input.keyboard["xkb-options"][0] === "caps:escape" && input.input["orientation-lock"] &&
    input.input.tablets["1234:5678"].mapping === "absolute" &&
    input["input-sources"].sources[1].id === "anthy" && input["input-sources"]["per-window"],
    "input preferences, XKB options, orientation lock, and input sources parse");
assert(
    parseDocument({ shell: { "window-menu": ["binguxctl", "ipc", "shell", "windowMenu"] } })["window-menu"].length ===
        4,
    "window menu command is configurable",
);
const typed = parseDocument({
    shell: { "minimize-animation": "zoom", "minimize-target": [500, 900] },
    autostart: [{ name: "dock", command: ["qs", "-p", "/a path/with spaces"] }],
});
assert(
    typed["minimize-target"][1] === 900 && typed.autostart[0].command[2] === "/a path/with spaces",
    "Lua arrays and autostart records retain values",
);
for (const document of [
    { shell: { osd: "false" } },
    { shell: { "window-menu": "sh -c unsafe" } },
    { shell: { "window-menu": [""] } },
    { shell: { "window-menu": ["command", "bad\0arg"] } },
    { shell: { "minimize-duration": 5001 } },
    { shell: { "minimize-target": [1] } },
    { autostart: [{ name: "dock", command: "qs" }] },
    { "window-management": { "focus-mode": "follow" } },
    { "window-management": { "auto-raise-delay": -1 } },
    { "window-management": { "action-right-click-titlebar": "bad" } },
    { "window-management": { "workspace-names": Array(37).fill("Workspace") } },
    { "window-management": { "workspace-names": [4] } },
    { compositor: { "visual-bell-type": "window-flash" } },
    { compositor: { "locate-pointer": "true" } },
    { input: { mouse: { speed: 1.1 } } },
    { input: { touchpad: { "click-method": "invalid" } } },
    { input: { keyboard: { delay: 0 } } },
    { input: { keyboard: { "xkb-options": "caps:escape" } } },
    { input: { "orientation-lock": "true" } },
    { input: { tablets: { "1234": { mapping: "absolute" } } } },
    { input: { styluses: { default: { "button-action": "invalid" } } } },
    { "input-sources": { sources: [{ type: "invalid", id: "us" }] } },
    { "input-sources": { sources: [{ type: "xkb", id: "" }] } },
]) {
    let rejected = false;
    try {
        parseDocument(document);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject invalid Lua document: ${JSON.stringify(document)}`);
}

const target = minimizeTarget(
    { get_icon_geometry: () => [false, null] },
    { x: 100, y: 200, width: 800, height: 600 },
)[1];
assert(target.x === 500 && target.y === 800, "bottom-centre fallback");
const icon = { x: 32, y: 64, width: 48, height: 48 };
assert(minimizeTarget({ get_icon_geometry: () => [true, icon] }, null)[1] === icon, "dock rectangle wins");
const panel = { x: -800, y: 30, width: 200, height: 40 };
const monitor = { x: -800, y: 0, width: 800, height: 600 };
assert(JSON.stringify(layerOffset(1 | 4 | 8, panel, monitor)) === "[0,-70]", "top edge slides vertically");
assert(JSON.stringify(layerOffset(1 | 4, panel, monitor)) === "[-200,-70]", "corner slides diagonally");
assert(JSON.stringify(layerOffset(2 | 8, panel, monitor)) === "[800,570]", "bottom right respects monitor origin");

const rules = parseDocument({
    "window-rules": [
        { match: { type: "layer" }, blur: 24, opacity: 0.9 },
        { match: { layer: "^dock$" }, blur: 12, animation: "none" },
    ],
});
const matched = windowEffects({ type: "layer", layer: "dock", focused: false }, rules);
assert(
    matched.blur === 12 && matched.opacity === 0.9 && matched.animation === "none",
    "later Lua rules override individual effects",
);
assert(windowEffects({ type: "window", layer: null }, rules).blur === 0, "layer rules do not match applications");
assert(windowEffects({ type: "window" }, rules, 24).blur === 24, "standard client blur has a compositor default");
assert(
    windowEffects({ type: "layer", layer: "dock" }, rules, 24).blur === 12,
    "explicit rules override standard blur strength",
);
const disabledBlur = parseDocument({ "window-rules": [{ match: { type: "window" }, blur: 0 }] });
assert(windowEffects({ type: "window" }, disabledBlur, 24).blur === 0, "explicit zero disables standard client blur");
for (const document of [
    { "window-rules": [{ match: { type: "layer" }, blur: 101 }] },
    { "window-rules": [{ match: { type: "layer" }, opacity: 2 }] },
    { "window-rules": [{ match: { title: "[" } }] },
    { "window-rules": [{ match: { unknown: "x" } }] },
]) {
    let rejected = false;
    try {
        parseDocument(document);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject invalid window rule: ${JSON.stringify(document)}`);
}

const shortcuts = parseDocument({
    keybindings: { shell: { show_screenshot_ui: [] } },
    shortcuts: [{ name: "capture", binding: "<Alt>s", command: ["qs", "ipc", "call", "capture", "open"] }],
});
assert(shortcuts.shortcuts[0].binding === "<Alt>s", "Lua shortcut record accepted");
assert(shortcuts.keybindings.shell.show_screenshot_ui.length === 0, "Lua built-in override accepted");
for (const document of [
    { shell: { shortcuts: [] } },
    { shortcuts: [{ name: "missing-fields" }] },
    { keybindings: { shell: { show_screenshot_ui: "Print" } } },
]) {
    let rejected = false;
    try {
        parseDocument(document);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject malformed Lua shortcut document: ${JSON.stringify(document)}`);
}
print("PASS: Lua shell configuration schema, rules, shortcuts, and geometry");
