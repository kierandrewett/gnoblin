import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinBridge/lib/fullscreen-return-guard.js", import.meta.url), "utf8").replace(
    "export class FullscreenReturnGuard",
    "this.FullscreenReturnGuard = class FullscreenReturnGuard",
);
const context = vm.createContext({});
vm.runInContext(source, context);

const fullscreen = { is_fullscreen: () => true };
const normal = { is_fullscreen: () => false };
const event = (type, button = 1) => ({ type: () => type, get_button: () => button });
const states = [],
    events = [],
    dismissals = [],
    guard = new context.FullscreenReturnGuard({
        buttonPress: "press",
        buttonRelease: "release",
        pointerEvents: ["press", "release", "motion", "scroll"],
        pick: () => fullscreen,
        dismiss: (done) => {
            events.push("dismiss");
            dismissals.push(done);
        },
        set: (value) => states.push(value),
    });

guard.update(
    { visible: true, revealCompanions: true, surface: "bingux-search" },
    [{ surface: "bingux-search" }],
);
assert.equal(
    guard.handle({
        type: () => "motion",
        get_button: () => {
            throw new Error("not a button event");
        },
    }),
    true,
    "fullscreen motion is blocked as soon as chrome is revealed",
);
assert.equal(guard.armed, true);
assert.deepEqual(states, [true]);
guard.pick = () => null;
assert.equal(guard.handle(event("press")), false, "the search surface owns the first outside click");
guard.update(
    { visible: false, revealCompanions: true, surface: "bingux-search-chrome" },
    [{ surface: "bingux-search-chrome" }],
);
assert.deepEqual(states, [true], "changing from search to chrome-only keeps one continuous barrier");
guard.pick = () => fullscreen;
assert.equal(guard.handle(event("scroll", 0)), true, "the fullscreen app cannot scroll before the second click");
assert.equal(guard.handle(event("press")), true, "the second fullscreen left press is consumed");
assert.deepEqual(events, ["dismiss"]);
assert.equal(guard.handle(event("release")), true, "the matching release is consumed");
assert.equal(guard.handle(event("motion", 0)), true, "motion stays blocked during the chrome exit");
assert.equal(guard.handle(event("press", 3)), true, "other buttons stay blocked during the chrome exit");
assert.deepEqual(states, [true], "the native barrier remains armed until the exit animation finishes");
dismissals.shift()();
assert.deepEqual(states, [true, false]);
assert.equal(guard.handle(event("press")), false, "the first post-animation click is available to the app");

guard.update(
    { visible: false, revealCompanions: true, surface: "bingux-search-chrome" },
    [{ surface: "bingux-search-chrome" }],
);
guard.pick = () => normal;
assert.equal(guard.handle(event("press")), false, "clicks on non-fullscreen windows are untouched");
guard.pick = () => fullscreen;
assert.equal(guard.handle(event("press", 3)), true, "non-left input cannot reach fullscreen before dismissal");
assert.deepEqual(events, ["dismiss"], "only the second left click starts dismissal");
guard.update(null, []);
assert.equal(guard.armed, false);
assert.deepEqual(states, [true, false, true, false]);

console.log("PASS: fullscreen input stays blocked through the second click and complete chrome exit");
