// Run with the matching Mutter library and typelib on the search path.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {ConfigFile} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition)
        throw new Error(message);
}

function settle() {
    const loop = new GLib.MainLoop(null, false);
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 450, () => {
        loop.quit();
        return GLib.SOURCE_REMOVE;
    });
    loop.run();
}

const directory = GLib.dir_make_tmp('gnoblin-lua-shell-XXXXXX');
const external = GLib.dir_make_tmp('gnoblin-lua-external-XXXXXX');
const path = `${directory}/init.lua`;
const modulePath = `${directory}/appearance.lua`;
const nested = `${directory}/parts/motion.lua`;
let current;
let updates = 0;
const config = new ConfigFile(path, next => {
    current = next;
    updates++;
});
const initial = `
local g = require('gnoblin')
require('appearance')
require('appearance')
g.set({shell = {['minimize-duration'] = 2 * 60}})
`;
try {
    GLib.file_set_contents(modulePath, `
local g = require('gnoblin')
g.set({shell = {['minimize-animation'] = 'none'},
    ['window-rules'] = {{match = {type = 'window'}, opacity = 0.9}}})
`);
    GLib.file_set_contents(path, initial);
    config.start();
    assert(current['minimize-duration'] === 120 && current['minimize-animation'] === 'none',
        'Lua expressions and imported settings apply');
    assert(current['window-rules'].length === 1, 'require loads a module once per evaluation');
    settle();
    GLib.file_set_contents(`${modulePath}.tmp`, `
require('gnoblin').set({shell = {['minimize-animation'] = 'fade'}})
`);
    Gio.File.new_for_path(`${modulePath}.tmp`).move(Gio.File.new_for_path(modulePath),
        Gio.FileCopyFlags.OVERWRITE, null, null);
    settle();
    assert(current['minimize-animation'] === 'fade' && current['window-rules'].length === 0,
        'atomic module save reloads from fresh state');
    GLib.file_set_contents(modulePath, 'this is not valid Lua');
    settle();
    assert(current['minimize-animation'] === 'fade', 'Lua syntax failure retains working state');
    let diagnostic = '';
    try { config.reload(); } catch (error) { diagnostic = error.message; }
    assert(diagnostic.includes('appearance.lua'), 'syntax error names its module');
    GLib.file_set_contents(modulePath, `return {shell = {['minimize-duration'] = 'bad'}}`);
    GLib.file_set_contents(path, `require('gnoblin').load('appearance.lua')`);
    settle();
    assert(current['minimize-duration'] === 120, 'schema failure retains working state');
    GLib.file_set_contents(modulePath, `return {shell = {['minimize-duration'] = 75}}`);
    settle();
    assert(current['minimize-duration'] === 75, 'correcting invalid module recovers automatically');
    GLib.file_set_contents(path, `require('gnoblin').load('parts/motion.lua')`);
    settle();
    assert(current['minimize-duration'] === 75, 'missing dependency retains working state');
    GLib.mkdir_with_parents(`${directory}/parts`, 0o700);
    GLib.file_set_contents(nested, `return {shell = {['minimize-duration'] = 35}}`);
    settle();
    assert(current['minimize-duration'] === 35, 'creating missing dependency directory recovers');
    GLib.file_set_contents(path, `return {shell = {['minimize-duration'] = 90}}`);
    settle();
    const before = updates;
    GLib.file_set_contents(nested, `return {shell = {['minimize-duration'] = 10}}`);
    settle();
    assert(updates === before && current['minimize-duration'] === 90,
        'removed dependencies stop triggering reloads');
    GLib.file_set_contents(path, `require('gnoblin').load('parts/**/*.lua')`);
    settle();
    assert(current['minimize-duration'] === 10, 'glob loads existing matching files');
    GLib.mkdir_with_parents(`${directory}/parts/nested`, 0o700);
    GLib.file_set_contents(`${directory}/parts/nested/90-extra.lua`,
        `return {shell = {['minimize-duration'] = 45}}`);
    settle();
    assert(current['minimize-duration'] === 45, 'new directory and matching file trigger glob reload');
    GLib.file_set_contents(`${directory}/parts/nested/90-extra.lua`, 'return {shell = ');
    settle();
    assert(current['minimize-duration'] === 45, 'invalid glob member retains last valid state');
    GLib.unlink(`${directory}/parts/nested/90-extra.lua`);
    settle();
    assert(current['minimize-duration'] === 10, 'removed glob member restores earlier values');
    GLib.mkdir_with_parents(`${external}/conf`, 0o700);
    GLib.file_set_contents(`${external}/conf/motion.lua`,
        `return {shell = {['minimize-duration'] = 55}}`);
    GLib.file_set_contents(path, `require('gnoblin').load('${external}/conf/*.lua')`);
    settle();
    assert(current['minimize-duration'] === 55, 'absolute external glob loads');
    Gio.File.new_for_path(`${external}/conf`).move(Gio.File.new_for_path(`${external}/previous`),
        Gio.FileCopyFlags.NONE, null, null);
    GLib.mkdir_with_parents(`${external}/conf`, 0o700);
    GLib.file_set_contents(`${external}/conf/motion.lua`,
        `return {shell = {['minimize-duration'] = 65}}`);
    settle();
    assert(current['minimize-duration'] === 65, 'replacing an external directory reloads the glob');
    GLib.file_set_contents(`${external}/conf/motion.lua`,
        `return {shell = {['minimize-duration'] = 85}}`);
    settle();
    assert(current['minimize-duration'] === 85, 'replacement directory stays watched');
    Gio.File.new_for_path(path).delete(null);
    settle();
    assert(current['minimize-duration'] === 200, 'deleting Lua root restores defaults');
    print('PASS: Lua modules, atomic saves, fresh state, validation, missing-file recovery, deletion');
} finally {
    config.destroy();
    for (const file of [path, modulePath, nested, `${modulePath}.tmp`])
        GLib.unlink(file);
    GLib.unlink(`${directory}/parts/nested/90-extra.lua`);
    GLib.rmdir(`${directory}/parts/nested`);
    GLib.rmdir(`${directory}/parts`);
    GLib.rmdir(directory);
    for (const name of ['conf', 'previous']) {
        GLib.unlink(`${external}/${name}/motion.lua`);
        GLib.rmdir(`${external}/${name}`);
    }
    GLib.rmdir(external);
}
