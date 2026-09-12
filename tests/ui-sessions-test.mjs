import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = readFileSync(new URL("../src/scripts/lib/ui-sessions.js", import.meta.url), "utf8");
const { UiSessions } = await import("data:text/javascript;base64," + Buffer.from(source).toString("base64"));
const events = [],
    scenes = [];
const bus = new UiSessions((client, record) => events.push({ client, ...record }), {
    update: (state) => scenes.push(state),
});
const desktop = {},
    search = {},
    observer = {};
const state = (client, name, state) => bus.command(client, { action: "state", name, state });
bus.command(observer, { action: "watch" });
state(desktop, "desktop", { pinnedApps: ["test"] });
state(search, "search", { visible: true, surface: "search", companions: ["bar", "dock"] });
assert.deepEqual(scenes.at(-1), [{ surface: "search", companions: ["bar", "dock"] }]);
assert.throws(() => state(desktop, "search", {}), /already owned/);
bus.command(observer, { action: "command", name: "search", command: { action: "close" } });
assert.equal(events.at(-1).client, search);
// Main-process loss must leave the independent overlay's request intact.
bus.close(desktop);
assert.equal(scenes.at(-1).length, 1);
assert.equal(events.at(-1).name, "desktop");
assert.equal(events.at(-1).state, null);
const late = {};
bus.command(late, { action: "watch" });
assert.equal(events.at(-1).client, late);
assert.equal(events.at(-1).state.visible, true);
state(search, "search", {
    visible: false,
    revealCompanions: true,
    surface: "chrome",
    companions: ["bar", "dock"],
    companionsAbove: true,
});
assert.deepEqual(scenes.at(-1), [{ surface: "chrome", companions: ["bar", "dock"], companionsAbove: true }]);
bus.close(search);
assert.deepEqual(scenes.at(-1), []);
const count = events.length;
bus.command(observer, { action: "command", name: "search", command: { action: "open" } });
assert.equal(events.length, count, "Do not replay stale commands after an owner restarts");
state({}, "search", { visible: false });
assert.throws(() => state({}, "../invalid", {}), /Invalid/);
console.log("PASS: independent owners, state replay, direct commands and disconnect cleanup");
