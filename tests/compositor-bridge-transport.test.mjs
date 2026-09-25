import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(
    new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinBridge/compositor-bridge.js", import.meta.url),
    "utf8",
)
    .replace(/^import .*;\n/gm, "")
    .replace("export class CompositorBridge", "this.CompositorBridge = class CompositorBridge");
const context = vm.createContext({
    Gio: { _promisify() {} },
    Shell: { ActionMode: { NORMAL: 1, POPUP: 2 } },
    GLib: { PRIORITY_DEFAULT: 0 },
    Meta: {
        KeyBindingAction: { NONE: 0 },
        KeyBindingFlags: { NONE: 0 },
        external_binding_name_for_action: (action) => String(action),
    },
    Clutter: { ModifierType: { MOD1_MASK: 8, SUPER_MASK: 67108864, CONTROL_MASK: 4 } },
    Cogl: {},
    Main: { sessionMode: { isLocked: false }, wm: { allowKeybinding() {} } },
    SessionLock: { isLocked: () => false },
    global: {
        get_pointer: () => [0, 0, 0],
        get_current_time: () => 123,
        display: {
            grab_accelerator: (() => {
                let action = 10;
                return () => action++;
            })(),
        },
    },
    Config: {},
    TextDecoder,
    Uint8Array,
    console,
});
vm.runInContext(source, context);
const bridge = Object.create(context.CompositorBridge.prototype);
let encodes = 0;
bridge.encoder = {
    encode(value) {
        encodes++;
        return new TextEncoder().encode(value);
    },
};
bridge.clients = new Set();
bridge.write = () => {};
bridge.close = (client) => {
    client.closed = true;
};
const watched = { trackWindows: true, queue: [], queuedBytes: 0, closed: false };
const second = { trackWindows: true, queue: [], queuedBytes: 0, closed: false };
const ignored = { trackWindows: false, queue: [], queuedBytes: 0, closed: false };
bridge.clients = new Set([watched, second, ignored]);
bridge.sendToSubscribers({ event: "windows", windows: [] }, (client) => client.trackWindows);
assert.equal(encodes, 1, "A broadcast serialises its record once");
assert.equal(watched.queue.length, 1);
assert.strictEqual(watched.queue[0], second.queue[0], "Subscribers share immutable encoded bytes");
assert.equal(ignored.queue.length, 0);
bridge.send({ fallback: true }, { event: "ignored" });
bridge.send({ closed: true }, { event: "ignored" });
assert.equal(encodes, 1, "Closed and local fallback clients need no serialisation");

const slow = { queue: Array.from({ length: 64 }, () => new Uint8Array()), queuedBytes: 0, closed: false };
bridge.enqueue(slow, new Uint8Array([1]));
assert.equal(slow.closed, true, "The 65th queued record closes a slow client before appending");

const activations = [];
const releaseClient = { bindings: new Map() };
const releaseBridge = Object.create(context.CompositorBridge.prototype);
releaseBridge.actions = new Map([
    [10, { client: releaseClient, id: "release", action: 10, hold: 0, trigger: "release" }],
    [11, { client: releaseClient, id: "press", action: 11, hold: 0, trigger: "press" }],
]);
releaseBridge.active = null;
releaseBridge.switcherFallback = { step: () => 0 };
releaseBridge.send = (_client, event) => activations.push(event);
assert.equal(releaseBridge.activate(10, "press"), false, "release binding ignores accelerator press");
assert.equal(releaseBridge.activate(10, "release"), true, "release binding accepts accelerator release");
assert.equal(activations.at(-1).id, "release");
assert.equal(releaseBridge.activate(11, "release"), false, "press binding ignores accelerator release");
assert.equal(releaseBridge.activate(11, "press"), true, "press binding keeps the default edge");
assert.equal(activations.at(-1).id, "press");

const registrationBridge = Object.create(context.CompositorBridge.prototype);
registrationBridge.actions = new Map();
registrationBridge.extensionOperations = new Map();
registrationBridge.switcherFallback = { claim: () => null };
registrationBridge.send = (_client, event) => activations.push(event);
const bindingClient = { bindings: new Map() };
for (const [id, accelerator, hold] of [
    ["alt", "<Alt>space", 0],
    ["control", "<Control>space", 0],
    ["shift", "<Shift>space", 0],
    ["super", "<Super>space", 0],
]) {
    registrationBridge.command(bindingClient, { op: "bind", id, accelerator, hold, trigger: "release" });
    assert.equal(bindingClient.bindings.get(id).trigger, "release", `${accelerator} accepts release trigger`);
}
assert.throws(
    () =>
        registrationBridge.command(bindingClient, {
            op: "bind",
            id: "super-only-press",
            accelerator: "Super",
            hold: 0,
            trigger: "press",
        }),
    /invalid shortcut registration/,
    "bare Super must wait until Mutter rules out a chord",
);

const received = [];
const input = new TextEncoder().encode('{"id":"one"}\n{"id":"two"}\n{"id":"three"}\n');
const stream = {
    read_bytes_async(_size, _priority, _cancel, callback) {
        callback(stream, {});
    },
    read_bytes_finish() {
        return { toArray: () => input };
    },
};
const reader = {
    connection: { get_input_stream: () => stream },
    cancel: {},
    buffer: new Uint8Array(),
    decoder: new TextDecoder("utf-8", { fatal: true }),
    closed: false,
};
bridge.command = (_client, record) => received.push(record.id);
bridge.read = () => {};
context.CompositorBridge.prototype.read.call(bridge, reader);
assert.deepEqual(received, ["one", "two", "three"]);
assert.equal(reader.buffer.length, 0, "A multi-record read retains no processed bytes");
console.log("PASS: bridge broadcasts encode once and enforce the slow-client queue limit");
