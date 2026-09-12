import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/scripts/compositor-bridge.js', import.meta.url), 'utf8')
    .replace(/^import .*;\n/gm, '')
    .replace("Gio._promisify(Shell.Screenshot, 'composite_to_stream');", '')
    .replace('class CompositorBridge', 'this.CompositorBridge = class CompositorBridge')
    .replace('export default function enable(api)', 'function enable(api)');
const context = vm.createContext({Gio: {}, Shell: {}, GLib: {PRIORITY_DEFAULT: 0}, Meta: {}, Clutter: {}, Cogl: {}, Main: {}, Config: {},
    TextDecoder, Uint8Array, console});
vm.runInContext(source, context);
const bridge = Object.create(context.CompositorBridge.prototype);
let encodes = 0;
bridge.encoder = {encode(value) { encodes++; return new TextEncoder().encode(value); }};
bridge.clients = new Set();
bridge.write = () => {};
bridge.close = client => { client.closed = true; };
const watched = {trackWindows: true, queue: [], queuedBytes: 0, closed: false};
const second = {trackWindows: true, queue: [], queuedBytes: 0, closed: false};
const ignored = {trackWindows: false, queue: [], queuedBytes: 0, closed: false};
bridge.clients = new Set([watched, second, ignored]);
bridge.sendToSubscribers({event: 'windows', windows: []}, client => client.trackWindows);
assert.equal(encodes, 1, 'A broadcast serialises its record once');
assert.equal(watched.queue.length, 1);
assert.strictEqual(watched.queue[0], second.queue[0], 'Subscribers share immutable encoded bytes');
assert.equal(ignored.queue.length, 0);
bridge.send({fallback: true}, {event: 'ignored'});
bridge.send({closed: true}, {event: 'ignored'});
assert.equal(encodes, 1, 'Closed and local fallback clients need no serialisation');

const slow = {queue: Array.from({length: 64}, () => new Uint8Array()), queuedBytes: 0, closed: false};
bridge.enqueue(slow, new Uint8Array([1]));
assert.equal(slow.closed, true, 'The 65th queued record closes a slow client before appending');

const received = [];
const input = new TextEncoder().encode('{"id":"one"}\n{"id":"two"}\n{"id":"three"}\n');
const stream = {
    read_bytes_async(_size, _priority, _cancel, callback) { callback(stream, {}); },
    read_bytes_finish() { return {toArray: () => input}; },
};
const reader = {connection: {get_input_stream: () => stream}, cancel: {}, buffer: new Uint8Array(),
    decoder: new TextDecoder('utf-8', {fatal: true}), closed: false};
bridge.command = (_client, record) => received.push(record.id);
bridge.read = () => {};
context.CompositorBridge.prototype.read.call(bridge, reader);
assert.deepEqual(received, ['one', 'two', 'three']);
assert.equal(reader.buffer.length, 0, 'A multi-record read retains no processed bytes');
console.log('PASS: bridge broadcasts encode once and enforce the slow-client queue limit');
