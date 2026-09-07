// Run with: gjs -m tests/shell-config-test.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {ConfigFile, DEFAULTS, parse} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition)
        throw new Error(message);
}

assert(JSON.stringify(parse('')) === JSON.stringify(DEFAULTS), 'missing keys use defaults');
const parsed = parse(`[protocols]
wlr-layer-shell = on
[shell]
window-switcher = yes # comment
minimize-animation = 'none' # comment
minimize-duration = 25
minimize-duration = 70
osd = off
screenshot = yes
`);
assert(parsed.osd === false && parsed.screenshot === true && parsed.notifications === null,
    'explicit feature values and unspecified persistent features');
assert(parsed['window-switcher'] && parsed['minimize-animation'] === 'none' &&
    parsed['minimize-duration'] === 70, 'comments, quotes, booleans, and last value');
for (const invalid of ['window-switcher = maybe', 'minimize-animation = shrink',
    'minimize-duration = -1', 'minimize-duration = 5001', 'minimize-duration = 2ms',
    'typo = true', 'notifications = maybe', '[shell']) {
    let rejected = false;
    try {
        parse(`[shell]\n${invalid}`);
    } catch {
        rejected = true;
    }
    assert(rejected, `reject ${invalid}`);
}

const dir = GLib.dir_make_tmp('gnoblin-config-test-XXXXXX');
const path = `${dir}/new/config/gnoblin.conf`;
let current;
let updates = 0;
const config = new ConfigFile(path, next => {
    current = next;
    updates++;
});
function settle() {
    const loop = new GLib.MainLoop(null, false);
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 400, () => {
        loop.quit();
        return GLib.SOURCE_REMOVE;
    });
    loop.run();
}
try {
    config.start();
    assert(current['window-switcher'] === false, 'start without file');
    GLib.file_set_contents(path, '[shell]\nwindow-switcher = true');
    settle();
    assert(current['window-switcher'] === true, 'first creation is watched');
    GLib.file_set_contents(`${path}.tmp`, '[shell]\nminimize-animation = none');
    Gio.File.new_for_path(`${path}.tmp`).move(Gio.File.new_for_path(path),
        Gio.FileCopyFlags.OVERWRITE, null, null);
    settle();
    assert(current['minimize-animation'] === 'none' && !current['window-switcher'],
        'atomic replacement reloads and removed keys reset');
    GLib.file_set_contents(path, '[shell]\nminimize-duration = invalid');
    settle();
    assert(current['minimize-animation'] === 'none', 'invalid edit retains last valid state');
    Gio.File.new_for_path(path).delete(null);
    settle();
    assert(current['minimize-animation'] === 'fade', 'deletion restores defaults');
    config.destroy();
    const before = updates;
    GLib.file_set_contents(path, '[shell]\nwindow-switcher = true');
    settle();
    assert(updates === before, 'destroy cancels watcher');
    config.start();
    assert(current['window-switcher'], 'restart reads latest file');
    config.destroy();
    print('PASS: shell config parsing, watching, atomic saves, recovery, deletion, lifecycle');
} finally {
    config.destroy();
    GLib.unlink(path);
    GLib.rmdir(`${dir}/new/config`);
    GLib.rmdir(`${dir}/new`);
    GLib.rmdir(dir);
}
