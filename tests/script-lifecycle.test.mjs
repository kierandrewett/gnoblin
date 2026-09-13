import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import test from 'node:test';

const source = readFileSync(new URL('../src/gnome-shell-overlay/js/ui/components/gnoblinControl.js', import.meta.url), 'utf8');
const errors = [];
const {EventBus, ScriptHost} = new Function('GLib', 'logError',
    'let scriptImportSeq=0;\n' + source.slice(source.indexOf('class EventBus {'), source.indexOf('// The wire contract.')) +
    '\nreturn {EventBus, ScriptHost};')(
    {build_filenamev: parts => parts.join('/'), get_user_config_dir: () => '/unused'},
    error => errors.push(error.message));

test('script cleanup unwinds nested wrappers and runs once', () => {
    const host = new ScriptHost({}, new EventBus());
    const events = [];
    const a = {_disposers: [() => events.push('a1'), () => events.push('a2')]};
    const b = {_disposers: [() => events.push('b1'), () => events.push('b2')]};
    host._loaded = [{api:a}, {api:b}];
    host.unload();
    host._disposeApi(a);
    host.unload();
    assert.deepEqual(events, ['b2','b1','a2','a1']);
});

test('sync and rejected async event handlers do not stop other handlers', async () => {
    const bus = new EventBus();
    const events = [];
    bus.subscribe('test', () => {throw new Error('sync failure');});
    bus.subscribe('test', async () => {throw new Error('async failure');});
    bus.subscribe('test', () => events.push('survived'));
    bus.emit('test');
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(events, ['survived']);
    assert.ok(errors.includes('sync failure'));
    assert.ok(errors.includes('async failure'));
});

test('rejected async script startup is cleaned up and later scripts load', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'gnoblin-script-lifecycle-'));
    try {
        writeFileSync(join(dir,'broken.mjs'), 'export default async api=>{api._disposers.push(()=>globalThis.scriptDisposed=true);await Promise.resolve();throw new Error("startup failure");};');
        writeFileSync(join(dir,'good.mjs'), 'export default api=>{globalThis.scriptSurvived=true;};');
        const host = new ScriptHost({}, new EventBus());
        host._recoveryChecked = true;
        host._dir = dir;
        host._scriptNames = () => ['broken.mjs','good.mjs'];
        await assert.rejects(host.load(), /broken.mjs/);
        assert.equal(globalThis.scriptDisposed, true);
        assert.equal(globalThis.scriptSurvived, true);
        assert.deepEqual(host.list(), ['good.mjs']);
        host.destroy();
    } finally {
        delete globalThis.scriptDisposed;
        delete globalThis.scriptSurvived;
        rmSync(dir, {recursive:true});
    }
});
