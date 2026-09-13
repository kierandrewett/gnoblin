import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../src/scripts/lib/fullscreen-return-guard.js", import.meta.url), "utf8").replace(
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
    guard = new context.FullscreenReturnGuard({
        buttonPress: "press",
        buttonRelease: "release",
        pick: () => fullscreen,
        dismiss: () => events.push("dismiss"),
        set: (value) => states.push(value),
    });

guard.arm();
assert.equal(guard.handle({type: () => "motion", get_button: () => { throw new Error("not a button event"); }}), false);
assert.equal(guard.armed, true);
assert.deepEqual(states, [true]);
assert.equal(guard.handle(event("press")), true, "the first fullscreen left press is consumed");
assert.deepEqual(events, ["dismiss"]);
assert.equal(guard.handle(event("release")), true, "the matching release is consumed");
assert.equal(guard.handle(event("press")), false, "the next click is available to the app");

guard.arm();
guard.pick = () => normal;
assert.equal(guard.handle(event("press")), false, "clicks on non-fullscreen windows are untouched");
guard.pick = () => fullscreen;
assert.equal(guard.handle(event("press", 3)), false, "non-left buttons are untouched");
guard.disarm();
assert.equal(guard.armed, false);
assert.deepEqual(states, [true, true, false]);

console.log("PASS: search outside-click guard consumes exactly one left-button press/release pair");
