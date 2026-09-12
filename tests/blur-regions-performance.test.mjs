import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(
    process.env.GNOBLIN_BLUR_REGIONS_SOURCE ?? new URL("../src/scripts/lib/blur-regions.js", import.meta.url),
    "utf8",
)
    .replace(/^import .*;\n/gm, "")
    .replace("export class BlurRegions", "this.BlurRegions = class BlurRegions");
let scans = 0;
let writes = 0;
const actors = [];
const context = vm.createContext({
    Meta: { gnoblin_layer_namespace: (window) => window.namespace },
    global: {
        get_window_actors() {
            scans++;
            return actors;
        },
        display: { get_monitor_geometry: (monitor) => ({ x: monitor * 1280, y: 0 }) },
    },
});
vm.runInContext(source, context);
const regions = new context.BlurRegions();
function actor(pid, namespace = "panel", monitor = 0) {
    const effect = {
        region: null,
        set_region(...value) {
            writes++;
            this.region = value;
        },
        clear_region() {
            writes++;
            this.region = null;
        },
    };
    const result = {
        effect,
        meta_window: { namespace, get_pid: () => pid, get_monitor: () => monitor },
        get_effect: () => effect,
    };
    actors.push(result);
    return result;
}
function client(pid) {
    return { connection: { get_socket: () => ({ get_credentials: () => ({ get_unix_pid: () => pid }) }) } };
}
const first = client(11);
const second = client(12);
const own = actor(11);
const other = actor(12);
const otherScreen = actor(11, "panel", 1);
const otherNamespace = actor(11, "popup");
for (let i = 0; i < 200; i++) actor(100 + i, null);
const record = { namespace: "panel", screen: [0, 0], region: [10, 20, 300, 40] };
regions.update(first, record);
regions.update(second, { ...record, region: [1, 2, 30, 40] });
assert.deepEqual(own.effect.region, record.region);
assert.deepEqual(other.effect.region, [1, 2, 30, 40]);
assert.equal(otherScreen.effect.region, null);
assert.equal(otherNamespace.effect.region, null);

scans = writes = 0;
for (let i = 0; i < 500; i++) regions.update(first, { ...record, region: [...record.region] });
const repeated = { scans, writes };
scans = writes = 0;
regions.update(first, { ...record, region: [11, 20, 300, 40] });
const changed = { scans, writes };
assert.deepEqual(own.effect.region, [11, 20, 300, 40]);
assert.deepEqual(other.effect.region, [1, 2, 30, 40]);

// A new actor and a replacement effect must receive an existing registration.
const newlyMapped = actor(11);
regions.apply(newlyMapped);
assert.deepEqual(newlyMapped.effect.region, [11, 20, 300, 40]);
newlyMapped.effect.region = null;
regions.apply(newlyMapped);
assert.deepEqual(newlyMapped.effect.region, [11, 20, 300, 40]);

// Multiple connections from one process retain their original precedence.
const fallback = client(11);
regions.update(fallback, { ...record, region: [5, 6, 7, 8] });
assert.deepEqual(own.effect.region, [11, 20, 300, 40]);
regions.close(first);
assert.deepEqual(own.effect.region, [5, 6, 7, 8]);
regions.close(fallback);
assert.equal(own.effect.region, null);
assert.equal(newlyMapped.effect.region, null);
assert.deepEqual(other.effect.region, [1, 2, 30, 40]);
regions.update(second, { ...record, region: null });
assert.equal(other.effect.region, null);
scans = writes = 0;
regions.update(second, { ...record, region: null });
assert.throws(() => regions.update(second, { ...record, region: [0, 0, -1, 1] }), /Invalid blur region/);
const absent = { scans, writes };
regions.close(second);
console.log(JSON.stringify({ repeated, changed, absent }));
assert.deepEqual(repeated, { scans: 0, writes: 0 }, "Unchanged registrations must not scan or update windows");
assert.deepEqual(changed, { scans: 1, writes: 1 }, "Only the matching surface must receive a changed region");
assert.deepEqual(absent, { scans: 0, writes: 0 }, "Removing an absent registration must do no window work");
console.log("PASS: blur-region work bounds, peer ownership, outputs, replacement effects and disconnects");
