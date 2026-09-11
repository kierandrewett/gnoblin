import * as Permissions from './gnoblinPermissions.js';
import * as Corners from './gnoblinCornerGeometry.js';
// Live shell settings. Protocol registration remains a compositor-startup operation.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';

export const FEATURE_KEYS = Object.freeze([
    'osd', 'osd-volume', 'osd-microphone', 'osd-brightness',
    'osd-keyboard-brightness', 'osd-pad', 'screenshot', 'notifications',
    'input-source-switcher',
]);

export const DEFAULTS = Object.freeze({
    ...Object.fromEntries(FEATURE_KEYS.map(key => [key, null])),
    'window-switcher': false,
    'minimize-animation': 'zoom',
    'minimize-duration': 200,
    'minimize-target': null,
    'layer-animation': 'slide',
    'layer-duration': 220,
    'layer-easing': 'ease-out-cubic',
    permissions: Permissions.DEFAULT_POLICY,
    autostart: [],
    'window-rules': [],
    shortcuts: [],
    keybindings: {},
});

export let settings = {...DEFAULTS};

function cleanValue(value) {
    value = value.trim();
    if (value[0] === "'" || value[0] === '"') {
        const end = value.indexOf(value[0], 1);
        if (end >= 0)
            return value.slice(1, end);
    }
    return value.replace(/\s+#.*$/, '').trim();
}

export function parseLegacy(text) {
    const next = {...DEFAULTS};
    let section = '';
    for (const raw of text.split('\n')) {
        const line = raw.trim();
        if (!line || line.startsWith('#') || line.startsWith(';'))
            continue;
        if (line.startsWith('[')) {
            const end = line.indexOf(']');
            if (end < 0)
                throw new Error(`invalid section: ${line}`);
            section = line.slice(1, end).trim();
            if (section.startsWith('permissions'))
                throw new Error('Permission rules require gnoblin.toml');
            continue;
        }
        if (section !== 'shell')
            continue;
        const separator = line.indexOf('=');
        const key = line.slice(0, separator).trim();
        const value = cleanValue(line.slice(separator + 1));
        if (separator < 0 || !Object.hasOwn(DEFAULTS, key) || ['shortcuts', 'keybindings', 'permissions'].includes(key))
            throw new Error(`unknown shell setting: ${line}`);
        if (key === 'window-switcher' || FEATURE_KEYS.includes(key)) {
            if (!/^(true|false|on|off|yes|no|1|0)$/i.test(value))
                throw new Error(`${key}: expected a boolean`);
            next[key] = /^(true|on|yes|1)$/i.test(value);
            continue;
        }
        switch (key) {
        case 'minimize-animation':
            if (!['zoom', 'fade', 'none', 'gnome'].includes(value))
                throw new Error(`${key}: expected zoom, fade, none, or gnome`);
            next[key] = value;
            break;
        case 'layer-animation':
            if (!['slide', 'fade', 'none'].includes(value))
                throw new Error(`${key}: expected slide, fade, or none`);
            next[key] = value;
            break;
        case 'layer-easing':
            if (!['ease-out-cubic', 'ease-out-quad', 'ease-in-out-cubic', 'linear'].includes(value))
                throw new Error(`${key}: unsupported easing`);
            next[key] = value;
            break;
        case 'layer-duration':
        case 'minimize-duration':
            if (!/^\d+$/.test(value) || Number(value) > 5000)
                throw new Error(`${key}: expected 0 to 5000 milliseconds`);
            next[key] = Number(value);
            break;
        }
    }
    return next;
}

export function parse(text) {
    return parseDocument(Meta.gnoblin_parse_toml(text).recursiveUnpack());
}

const CONCATENATED_ARRAY_KEYS = new Set(['autostart', 'window-rules', 'shortcuts', 'rules']);

function isTable(value) {
    return value && typeof value === 'object' && !Array.isArray(value);
}

// Includes are merged in declaration order. Tables merge recursively, while
// rule-like arrays append so a package fragment can add rules without
// replacing the user's own rules. Scalar settings and ordinary arrays in the
// user's file win over included values.
function mergeDocuments(previous, next, key = null) {
    if (isTable(previous) && isTable(next)) {
        const merged = {...previous};
        for (const [name, value] of Object.entries(next))
            merged[name] = Object.hasOwn(merged, name)
                ? mergeDocuments(merged[name], value, name) : value;
        return merged;
    }
    if (Array.isArray(previous) && Array.isArray(next) && CONCATENATED_ARRAY_KEYS.has(key))
        return [...previous, ...next];
    return next;
}

function includePaths(document, path) {
    if (document.include !== undefined && document.source !== undefined)
        throw new Error(`${path}: use either include or source, not both`);
    const value = document.include ?? document.source;
    if (value === undefined)
        return [];
    const values = Array.isArray(value) ? value : [value];
    if (!values.length || values.some(entry => typeof entry !== 'string' || !entry.trim()))
        throw new Error(`${path}: include/source must be a nonempty path or array of paths`);
    return values.map(entry => {
        let included = entry;
        if (included.startsWith('~/'))
            included = GLib.build_filenamev([GLib.get_home_dir(), included.slice(2)]);
        if (!GLib.path_is_absolute(included))
            included = GLib.build_filenamev([GLib.path_get_dirname(path), included]);
        return GLib.canonicalize_filename(included, null);
    });
}

function loadTomlDocument(path, stack = []) {
    const canonical = GLib.canonicalize_filename(path, null);
    if (stack.includes(canonical))
        throw new Error(`${canonical}: include cycle (${[...stack, canonical].join(' -> ')})`);
    let bytes;
    try {
        [, bytes] = Gio.File.new_for_path(canonical).load_contents(null);
    } catch (error) {
        throw new Error(`${canonical}: cannot read included config: ${error.message}`);
    }
    let document;
    try {
        document = Meta.gnoblin_parse_toml(new TextDecoder('utf-8', {fatal: true}).decode(bytes)).recursiveUnpack();
    } catch (error) {
        throw new Error(`${canonical}: invalid TOML: ${error.message}`);
    }
    let merged = {};
    const paths = [canonical];
    for (const included of includePaths(document, canonical)) {
        const loaded = loadTomlDocument(included, [...stack, canonical]);
        merged = mergeDocuments(merged, loaded.document);
        paths.push(...loaded.paths);
    }
    const local = {...document};
    delete local.include;
    delete local.source;
    return {document: mergeDocuments(merged, local), paths};
}

export function parseDocument(document) {
    const next = {...DEFAULTS};
    next.permissions = Permissions.validate(document.permissions);
    const shell = document.shell ?? {};
    if (!shell || Array.isArray(shell) || typeof shell !== 'object')
        throw new Error('shell must be a table');
    for (const [key, value] of Object.entries(shell)) {
        if (!Object.hasOwn(DEFAULTS, key) || ['autostart', 'window-rules', 'shortcuts', 'keybindings', 'permissions'].includes(key))
            throw new Error(`unknown shell setting: ${key}`);
        if (FEATURE_KEYS.includes(key) || key === 'window-switcher') {
            if (typeof value !== 'boolean')
                throw new Error(`${key}: expected a boolean`);
        } else if (key === 'minimize-animation') {
            if (!['zoom', 'fade', 'none', 'gnome'].includes(value))
                throw new Error(`${key}: expected zoom, fade, none, or gnome`);
        } else if (key === 'layer-animation') {
            if (!['slide', 'fade', 'none'].includes(value))
                throw new Error(`${key}: expected slide, fade, or none`);
        } else if (key === 'layer-easing') {
            if (!['ease-out-cubic', 'ease-out-quad', 'ease-in-out-cubic', 'linear'].includes(value))
                throw new Error(`${key}: unsupported easing`);
        } else if (key === 'minimize-duration' || key === 'layer-duration') {
            if (!Number.isInteger(value) || value < 0 || value > 5000)
                throw new Error(`${key}: expected 0 to 5000 milliseconds`);
        } else if (key === 'minimize-target') {
            if (!Array.isArray(value) || value.length !== 2 ||
                !value.every(n => Number.isInteger(n) && Math.abs(n) <= 1000000))
                throw new Error(`${key}: expected [x, y] in logical desktop coordinates`);
        }
        next[key] = value;
    }
    if (document.protocols !== undefined &&
        (!document.protocols || Array.isArray(document.protocols) ||
         typeof document.protocols !== 'object' ||
         !Object.values(document.protocols).every(value => typeof value === 'boolean')))
        throw new Error('protocols must be a table of booleans');
    const entries = document.autostart ?? [];
    if (!Array.isArray(entries))
        throw new Error('autostart must use [[autostart]] tables');
    const names = new Set();
    for (const entry of entries) {
        if (!entry || typeof entry !== 'object' || Array.isArray(entry) ||
            Object.keys(entry).some(key => !['name', 'command'].includes(key)) ||
            typeof entry.name !== 'string' || !entry.name.trim() || names.has(entry.name) ||
            !Array.isArray(entry.command) || entry.command.length === 0 ||
            !entry.command.every(arg => typeof arg === 'string' && !arg.includes('\0')) ||
            !entry.command[0])
            throw new Error('autostart requires a unique name and a nonempty command array');
        names.add(entry.name);
    }
    next.autostart = entries;
    const rules = document['window-rules'] ?? [];
    if (!Array.isArray(rules))
        throw new Error('window-rules must use [[window-rules]] tables');
    for (const rule of rules) {
        if (!rule || !rule.match || Array.isArray(rule.match) || typeof rule.match !== 'object' ||
            Object.keys(rule.match).length === 0 ||
            Object.keys(rule).some(key => !['match', 'blur', 'blur-ignore-shadows', 'opacity', 'animation', 'shader', 'shader-uniforms', 'corners', 'borders'].includes(key)))
            throw new Error('window rule requires match and supported effects');
        for (const [key, value] of Object.entries(rule.match)) {
            if (key === 'focused') {
                if (typeof value !== 'boolean') throw new Error('focused match must be boolean');
            } else if (['app-id', 'title', 'layer'].includes(key)) {
                if (typeof value !== 'string' || value.length > 512) throw new Error('rule matcher must be a regex string');
                new RegExp(value);
            } else if (key !== 'type' || !['layer', 'window'].includes(value)) {
                throw new Error('unknown window rule match');
            }
        }
        if (rule.corners !== undefined) Corners.validate(rule.corners);
        if (rule.borders !== undefined) Corners.validateBorders(rule.borders);
        if (rule.blur !== undefined && (!Number.isInteger(rule.blur) || rule.blur < 0 || rule.blur > 100))
            throw new Error('blur must be an integer from 0 to 100');
        if (rule['blur-ignore-shadows'] !== undefined && typeof rule['blur-ignore-shadows'] !== 'boolean')
            throw new Error('blur-ignore-shadows must be boolean');
        if (rule.opacity !== undefined && (typeof rule.opacity !== 'number' || !Number.isFinite(rule.opacity) || rule.opacity < 0 || rule.opacity > 1))
            throw new Error('opacity must be between 0 and 1');
        if (rule.animation !== undefined) {
            const animation = rule.animation;
            if (typeof animation === 'string') {
                if (!['slide', 'fade', 'none'].includes(animation))
                    throw new Error('rule animation must be slide, fade, or none');
            } else {
                if (!animation || Array.isArray(animation) || typeof animation !== 'object' ||
                    Object.keys(animation).some(key => !['in', 'out', 'duration', 'easing'].includes(key)) ||
                    ['in', 'out'].some(key => animation[key] !== undefined && !['slide', 'fade', 'none'].includes(animation[key])) ||
                    (animation.duration !== undefined && (!Number.isInteger(animation.duration) || animation.duration < 0 || animation.duration > 5000)) ||
                    (animation.easing !== undefined && !['ease-out-cubic', 'ease-out-quad', 'ease-in-out-cubic', 'linear'].includes(animation.easing)))
                    throw new Error('invalid layer animation policy');
            }
        }
        if (rule.shader !== undefined && (typeof rule.shader !== 'string' || rule.shader.length > 4096 || rule.shader.includes('\0')))
            throw new Error('shader must be a file path, or an empty string to remove it');
        if (rule['shader-uniforms'] !== undefined) {
            const uniforms = rule['shader-uniforms'];
            if (!uniforms || typeof uniforms !== 'object' || Array.isArray(uniforms) || Object.keys(uniforms).length > 64 ||
                Object.entries(uniforms).some(([name, value]) => !/^[a-zA-Z_]\w*$/.test(name) || name.startsWith('gnoblin_') ||
                    typeof value !== 'number' || !Number.isFinite(value) || Math.abs(value) > 3.4e38))
                throw new Error('shader-uniforms must contain up to 64 named finite floats; gnoblin_ names are reserved');
        }
    }
    next['window-rules'] = rules;
    Object.assign(next, validateShortcuts(document));
    return next;
}

export const KEYBINDING_SCHEMAS = Object.freeze({
    shell: 'org.gnome.shell.keybindings',
    wm: 'org.gnome.desktop.wm.keybindings',
    mutter: 'org.gnome.mutter.keybindings',
    wayland: 'org.gnome.mutter.wayland.keybindings',
    media: 'org.gnome.settings-daemon.plugins.media-keys',
});
const SHORTCUT_BASE = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/';
const SHORTCUT_PREFIX = `${SHORTCUT_BASE}gnoblin-config-`;
const SHORTCUT_SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding';

function acceleratorIdentity(value) {
    if (value === 'Super') return 'overlay-key';
    if (typeof value !== 'string' || !value || value.length > 160 ||
        !/^(?:<(?:Shift|Control|Ctrl|Primary|Alt|Mod1|Super|Meta|Hyper|Mod[2-5])>)*[A-Za-z0-9_]+$/i.test(value))
        throw new Error(`invalid shortcut accelerator: ${JSON.stringify(value)}; use <Alt>s or <Super>Return`);
    const key = value.replace(/<[^>]+>/g, '').replace(/^XF86/, '');
    if (typeof Clutter[`KEY_${key}`] !== 'number')
        throw new Error(`unknown shortcut key: ${key}`);
    const modifiers = [...value.matchAll(/<([^>]+)>/g)].map(([, modifier]) =>
        modifier.toLowerCase().replace(/^(ctrl|primary)$/, 'control').replace(/^mod1$/, 'alt'));
    return [...new Set(modifiers)].sort().join('+') + '+' + key.toLowerCase();
}

export function validateShortcuts(document) {
    const shortcuts = document.shortcuts ?? [];
    const keybindings = document.keybindings ?? {};
    if (!Array.isArray(shortcuts) || shortcuts.length > 256)
        throw new Error('shortcuts must use [[shortcuts]] tables (maximum 256)');
    const names = new Set(), accelerators = new Set();
    for (const entry of shortcuts) {
        if (!entry || typeof entry !== 'object' || Array.isArray(entry) ||
            Object.keys(entry).some(key => !['name', 'binding', 'command', 'capture-input'].includes(key)) ||
            (entry['capture-input'] !== undefined && typeof entry['capture-input'] !== 'boolean') ||
            typeof entry.name !== 'string' || !/^[a-zA-Z0-9_-]{1,80}$/.test(entry.name) || names.has(entry.name) ||
            !Array.isArray(entry.command) || !entry.command.length || !entry.command[0] ||
            !entry.command.every(arg => typeof arg === 'string' && !arg.includes('\0')))
            throw new Error('shortcut requires a unique name, binding and nonempty command array');
        const identity = acceleratorIdentity(entry.binding);
        if (accelerators.has(identity)) throw new Error(`duplicate shortcut: ${entry.binding}`);
        names.add(entry.name);
        accelerators.add(identity);
    }
    if (!keybindings || typeof keybindings !== 'object' || Array.isArray(keybindings))
        throw new Error('keybindings must be a table');
    const source = Gio.SettingsSchemaSource.get_default();
    for (const [group, entries] of Object.entries(keybindings)) {
        const schema = KEYBINDING_SCHEMAS[group] && source.lookup(KEYBINDING_SCHEMAS[group], true);
        if (!schema || !entries || typeof entries !== 'object' || Array.isArray(entries))
            throw new Error(`unknown keybinding group: ${group}`);
        for (const [key, bindings] of Object.entries(entries)) {
            if (!schema.has_key(key) || key === 'custom-keybindings' ||
                schema.get_key(key).get_value_type().dup_string() !== 'as' || !Array.isArray(bindings))
                throw new Error(`unknown or unsupported keybinding: ${group}.${key}`);
            for (const binding of bindings) {
                const identity = acceleratorIdentity(binding);
                if (accelerators.has(identity)) throw new Error(`duplicate shortcut: ${binding}`);
                accelerators.add(identity);
            }
        }
    }
    return {shortcuts, keybindings};
}

// Config owns only its named command entries. Other custom shortcuts are
// preserved; explicit built-in overrides follow the persistent feature policy.
export class Shortcuts {
    constructor(commands = null) {
        this.commands = commands;
    }

    apply(config) {
        const {shortcuts, keybindings} = validateShortcuts(config);
        const media = new Gio.Settings({schema_id: KEYBINDING_SCHEMAS.media});
        const previous = media.get_strv('custom-keybindings');
        if (!this.commands && shortcuts.some(entry => entry.binding === 'Super'))
            throw new Error('Super release requires native command shortcuts');
        const settingsShortcuts = this.commands ? [] : shortcuts;
        const paths = settingsShortcuts.map(entry => `${SHORTCUT_PREFIX}${entry.name}/`);
        const writes = [];
        if (shortcuts.some(entry => entry.binding === 'Super'))
            writes.push([new Gio.Settings({schema_id: 'org.gnome.mutter'}), 'overlay-key', new GLib.Variant('s', 'Super')]);
        for (const [group, entries] of Object.entries(keybindings)) {
            const settings = new Gio.Settings({schema_id: KEYBINDING_SCHEMAS[group]});
            for (const [key, value] of Object.entries(entries))
                writes.push([settings, key, new GLib.Variant('as', value)]);
        }
        for (const [index, entry] of settingsShortcuts.entries()) {
            const settings = new Gio.Settings({schema_id: SHORTCUT_SCHEMA, path: paths[index]});
            for (const [key, value] of Object.entries({name: entry.name, binding: entry.binding,
                command: entry.command.map(arg => GLib.shell_quote(arg)).join(' ')}))
                writes.push([settings, key, new GLib.Variant('s', value)]);
        }
        writes.push([media, 'custom-keybindings', new GLib.Variant('as', [
            ...previous.filter(path => !path.startsWith(SHORTCUT_PREFIX)), ...paths,
        ])]);
        for (const path of previous.filter(path => path.startsWith(SHORTCUT_PREFIX) && !paths.includes(path))) {
            const settings = new Gio.Settings({schema_id: SHORTCUT_SCHEMA, path});
            for (const key of ['binding', 'command', 'name'])
                writes.push([settings, key, null]);
        }
        // Check every key before changing anything, including policy-locked keys.
        const changes = writes.filter(([settings, key, value]) =>
            value ? !settings.get_value(key).equal(value) : settings.get_user_value(key) !== null);
        for (const [settings, key] of changes) {
            if (!settings.is_writable(key)) throw new Error(`shortcut setting is locked: ${key}`);
        }
        const applied = [];
        try {
            for (const [settings, key, value] of changes) {
                applied.push([settings, key, settings.get_user_value(key)]);
                if (value === null) settings.reset(key);
                else if (!settings.set_value(key, value)) throw new Error(`could not save shortcut: ${key}`);
            }
            this.commands?.apply(shortcuts);
        } catch (error) {
            for (const [settings, key, value] of applied.reverse()) {
                if (value === null) settings.reset(key);
                else settings.set_value(key, value);
            }
            throw error;
        }
    }

    destroy() { this.commands?.destroy(); }
}

// Commands are edge-triggered, not text input: holding a shortcut must never
// repeatedly launch it (or toggle a popup straight back closed).
export class CommandShortcuts {
    constructor(display, allow, launch = command => {
        const child = Gio.Subprocess.new(command, Gio.SubprocessFlags.NONE);
        child.wait_check_async(null, (process, result) => {
            try { process.wait_check_finish(result); }
            catch (error) { console.warn(`gnoblin-shortcut: ${error.message}`); }
        });
    }, prepareInput = () => {}) {
        this.display = display;
        this.prepareInput = prepareInput;
        this.allow = allow;
        this.launch = launch;
        this.bindings = new Map();
        this.overlaySignal = display.connect('overlay-key', () => {
            const binding = this.bindings.get('overlay-key');
            if (!binding) return;
            try { if (binding.entry['capture-input']) this.prepareInput(binding.entry.name); this.launch(binding.entry.command); }
            catch (error) { console.warn(`gnoblin-shortcut ${binding.entry.name}: ${error.message}`); }
        });
        this.signal = display.connect('accelerator-activated', (_display, action) => {
            const binding = [...this.bindings.values()].find(item => item.action === action);
            if (!binding) return;
            try { if (binding.entry['capture-input']) this.prepareInput(binding.entry.name); this.launch(binding.entry.command); }
            catch (error) { console.warn(`gnoblin-shortcut ${binding.entry.name}: ${error.message}`); }
        });
    }

    apply(entries) {
        const next = new Map(), added = [];
        try {
            for (const entry of entries) {
                const identity = acceleratorIdentity(entry.binding);
                let action = this.bindings.get(identity)?.action;
                if (action === undefined) {
                    action = identity === 'overlay-key' ? 'overlay-key' :
                        this.display.grab_accelerator(entry.binding, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT);
                    if (action === Meta.KeyBindingAction.NONE)
                        throw new Error(`shortcut already claimed: ${entry.binding} (${entry.name})`);
                    added.push(action);
                    this.allow(action, true);
                }
                next.set(identity, {action, entry});
            }
        } catch (error) {
            for (const action of added) this.release(action);
            throw error;
        }
        for (const [identity, binding] of this.bindings) {
            if (!next.has(identity)) this.release(binding.action);
        }
        this.bindings = next;
    }

    release(action) {
        if (action !== 'overlay-key') this.display.ungrab_accelerator(action);
        this.allow(action, false);
    }

    destroy() {
        this.display.disconnect(this.signal);
        this.display.disconnect(this.overlaySignal);
        for (const binding of this.bindings.values()) this.release(binding.action);
        this.bindings.clear();
    }
}

// Buffer native key events only while a configured popup is taking focus.
// The compositor already distinguishes a bare Super release from a Super chord.
export class ShortcutInput {
    constructor(stage, grab, ungrab) {
        this.stage = stage;
        this.grabKeyboard = grab;
        this.ungrabKeyboard = ungrab;
        this.completion = 0;
        this.ready = new Set();
        this.pending = null;
        this.timeout = 0;
        this.signal = stage.connect('captured-event', (_stage, event) => {
            if (!this.pending) return Clutter.EVENT_PROPAGATE;
            const type = event.type();
            if (![Clutter.EventType.KEY_PRESS, Clutter.EventType.KEY_RELEASE].includes(type))
                return Clutter.EVENT_PROPAGATE;
            // The release that triggered overlay-key can reach the stage after
            // begin(). Replaying it would activate the shortcut a second time.
            if ([Clutter.KEY_Super_L, Clutter.KEY_Super_R].includes(event.get_key_symbol()))
                return Clutter.EVENT_PROPAGATE;
            if (this.pending.events.length >= 1024) { this.cancel(); return Clutter.EVENT_PROPAGATE; }
            this.pending.events.push(event.copy());
            return Clutter.EVENT_STOP;
        });
    }
    begin(name) {
        if (this.ready.has(name)) return; // A toggle is closing an already focused popup.
        this.cancel();
        const grab = this.grabKeyboard();
        if (!(grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) { this.ungrabKeyboard(grab); return; }
        this.pending = {name, events: [], grab};
        this.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
            this.timeout = 0;
            this.cancel();
            return GLib.SOURCE_REMOVE;
        });
    }
    prepared(name) {
        if (this.pending?.name !== name || !this.pending.grab) return;
        this.ungrabKeyboard(this.pending.grab);
        this.pending.grab = null;
    }
    complete(name) {
        this.ready.add(name);
        if (this.pending?.name !== name || this.completion) return;
        // Keys already filtered into the stage queue still belong to our grab.
        // Finish that update before handing off, then dispatch the buffered keys
        // synchronously so newer input cannot overtake an editing key.
        this.completion = this.stage.connect('after-update', () => {
            this.stage.disconnect(this.completion);
            this.completion = 0;
            this.prepared(name);
            const events = this.pending.events;
            this.pending = null;
            if (this.timeout) GLib.source_remove(this.timeout);
            this.timeout = 0;
            for (const event of events) this.stage.handle_event(event);
        });
        this.stage.schedule_update();
    }
    closed(name) {
        this.ready.delete(name);
        if (this.pending?.name === name) this.cancel();
    }
    cancel() {
        if (this.completion) this.stage.disconnect(this.completion);
        this.completion = 0;
        if (this.timeout) GLib.source_remove(this.timeout);
        this.timeout = 0;
        if (this.pending?.grab) this.ungrabKeyboard(this.pending.grab);
        this.pending = null;
    }
    destroy() {
        this.cancel();
        this.stage.disconnect(this.signal);
        this.ready.clear();
    }
}

// The dock hint takes precedence over the optional fallback coordinate.
export function minimizeTarget(window, monitor) {
    const [success, rect] = window.get_icon_geometry();
    if (success)
        return [true, rect];
    const point = settings['minimize-target'];
    if (!point && !monitor)
        return [false, null];
    return [true, {
        x: point ? point[0] : monitor.x + monitor.width / 2,
        y: point ? point[1] : monitor.y + monitor.height,
        width: 0,
        height: 0,
    }];
}

export class Autostart {
    constructor() {
        this._started = new Set();
    }

    apply(entries) {
        for (const {name, command} of entries) {
            if (this._started.has(name))
                continue;
            try {
                const child = Gio.Subprocess.new(command, Gio.SubprocessFlags.NONE);
                this._started.add(name);
                child.wait_async(null, (process, result) => {
                    try {
                        process.wait_finish(result);
                        if (!process.get_successful())
                            console.warn(`gnoblin-autostart: ${name} exited unsuccessfully`);
                    } catch (e) {
                        console.warn(`gnoblin-autostart: ${name}: ${e.message}`);
                    }
                });
                console.log(`gnoblin-autostart: started ${name}`);
            } catch (e) {
                console.warn(`gnoblin-autostart: could not start ${name}: ${e.message}`);
            }
        }
    }
}

export class ConfigFile {
    constructor(path = null, apply = () => {}, parseToml = parse) {
        this._override = path || GLib.getenv('GNOBLIN_CONFIG') || null;
        this._directory = this._override ? GLib.path_get_dirname(this._override) :
            GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin']);
        this._apply = apply;
        this._parseToml = parseToml;
        this._monitor = null;
        this._monitors = new Map();
        this._watchedFiles = new Set();
        this._timeout = 0;
    }

    get path() {
        if (this._override)
            return this._override;
        const toml = GLib.build_filenamev([this._directory, 'gnoblin.toml']);
        const legacy = GLib.build_filenamev([this._directory, 'gnoblin.conf']);
        return GLib.file_test(toml, GLib.FileTest.EXISTS) ||
            !GLib.file_test(legacy, GLib.FileTest.EXISTS) ? toml : legacy;
    }

    reload() {
        let text = '';
        const path = this.path;
        let loaded = {document: {}, paths: [path]};
        try {
            const [, bytes] = Gio.File.new_for_path(path).load_contents(null);
            text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
            if (path.endsWith('.conf'))
                loaded = null;
            else
                loaded = loadTomlDocument(path);
        } catch (e) {
            if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
                throw e;
        }
        // Parse the complete file and every included fragment before replacing
        // the last valid settings. A missing main file means defaults.
        let next;
        try {
            next = loaded ? parseDocument(loaded.document) : parseLegacy(text);
        } catch (e) {
            throw new Error(`${path}: ${e.message}`);
        }
        this._apply(next);
        settings = next;
        this._setWatchedFiles(loaded?.paths ?? [path]);
    }

    setPermissions(policy, expected) {
        if (this.path.endsWith('.conf'))
            throw new Error('Permission rules require gnoblin.toml; migrate the legacy configuration first');
        const file = Gio.File.new_for_path(this.path);
        let text = '', etag = null;
        try {
            const [, bytes, loadedEtag] = file.load_contents(null);
            text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
            etag = loadedEtag;
        } catch (e) {
            if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND)) throw e;
        }
        const decode = contents => Meta.gnoblin_parse_toml(contents).recursiveUnpack();
        const before = decode(text);
        if (JSON.stringify(Permissions.validate(before.permissions)) !== JSON.stringify(expected))
            throw new Error("Permission policy changed; inspect it and retry");
        const replacement = Permissions.replacePolicy(text, policy);
        const after = decode(replacement);
        delete before.permissions;
        delete after.permissions;
        if (JSON.stringify(before) !== JSON.stringify(after))
            throw new Error('Cannot safely edit this TOML layout; edit the permissions table manually');
        this._parseToml(replacement);
        file.replace_contents(replacement, etag, false, Gio.FileCreateFlags.PRIVATE, null);
        this.reload();
    }

    start() {
        if (this._monitor)
            return;
        const parent = Gio.File.new_for_path(this._directory);
        GLib.mkdir_with_parents(parent.get_path(), 0o700);
        this._setWatchedFiles([this.path]);
        this._monitor = true;
        this._tryReload();
    }

    _setWatchedFiles(paths) {
        const wanted = new Set(paths.map(path => GLib.canonicalize_filename(path, null)));
        const directories = new Map();
        for (const path of wanted) {
            const directory = GLib.path_get_dirname(path);
            if (!directories.has(directory))
                directories.set(directory, new Set());
            directories.get(directory).add(path);
        }
        for (const [directory, monitor] of this._monitors) {
            if (directories.has(directory))
                continue;
            monitor.cancel();
            this._monitors.delete(directory);
        }
        for (const [directory, files] of directories) {
            if (this._monitors.has(directory))
                continue;
            const monitor = Gio.File.new_for_path(directory).monitor_directory(
                Gio.FileMonitorFlags.WATCH_MOVES, null);
            monitor.connect('changed', (_monitor, file, otherFile) => {
                const changed = [file, otherFile].filter(Boolean).map(item =>
                    GLib.canonicalize_filename(item.get_path(), null));
                if (!changed.some(path => this._watchedFiles.has(path)))
                    return;
                if (this._timeout)
                    GLib.source_remove(this._timeout);
                this._timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
                    this._timeout = 0;
                    this._tryReload();
                    return GLib.SOURCE_REMOVE;
                });
            });
            this._monitors.set(directory, monitor);
        }
        this._watchedFiles = wanted;
    }

    _tryReload() {
        try {
            this.reload();
            console.log(`gnoblin-config: loaded ${this.path}`);
        } catch (e) {
            console.warn(`gnoblin-config: keeping last valid settings: ${e.message}`);
        }
    }

    destroy() {
        if (this._timeout)
            GLib.source_remove(this._timeout);
        this._timeout = 0;
        for (const monitor of this._monitors.values())
            monitor.cancel();
        this._monitors.clear();
        this._watchedFiles.clear();
        this._monitor = null;
    }
}

// Opposing anchors cancel on an axis; corners move diagonally. Full-screen
// overlays fade, since their transparent input surface has no single edge.
export function layerOffset(anchor, rect, monitor) {
    if (!monitor)
        return [0, 0];
    const top = Boolean(anchor & 1), bottom = Boolean(anchor & 2);
    const left = Boolean(anchor & 4), right = Boolean(anchor & 8);
    const x = left === right ? 0 : left
        ? monitor.x - rect.x - rect.width : monitor.x + monitor.width - rect.x;
    const y = top === bottom ? 0 : top
        ? monitor.y - rect.y - rect.height : monitor.y + monitor.height - rect.y;
    return [x, y];
}

// Later matching rules override individual effects, leaving others intact.
export function windowEffects(properties, config = settings) {
    const effects = {borders: {...Corners.borderDefaults}, corners: {...Corners.defaults}, blur: 0, 'blur-ignore-shadows': false, opacity: 1, animation: config['layer-animation'], shader: '', 'shader-uniforms': {}};
    for (const rule of config['window-rules']) {
        if (Object.entries(rule.match).every(([key, value]) =>
            key === 'type' || key === 'focused' ? properties[key] === value :
                properties[key] !== null && new RegExp(value).test(properties[key] ?? ''))) {
            if (rule.corners !== undefined) effects.corners = Corners.merge(effects.corners, rule.corners);
            if (rule.borders !== undefined) effects.borders = {...effects.borders, ...rule.borders};
            for (const key of ['blur', 'blur-ignore-shadows', 'opacity', 'animation', 'shader', 'shader-uniforms'])
                if (rule[key] !== undefined) effects[key] = rule[key];
        }
    }
    return effects;
}

export function windowProperties(window) {
    const namespace = Meta.gnoblin_layer_namespace(window);
    return {
        'app-id': window.get_gtk_application_id() || window.get_wm_class() || '',
        title: window.get_title() || '',
        layer: namespace,
        type: namespace !== null ? 'layer' : 'window',
        focused: window.has_focus(),
    };
}

// Resolve each phase independently; string rules retain their existing meaning.
export function layerAnimation(properties, opening, config = settings) {
    const rule = windowEffects(properties, config).animation;
    return {
        animation: typeof rule === 'string' ? rule : rule[opening ? 'in' : 'out'] ?? config['layer-animation'],
        duration: typeof rule === 'object' ? rule.duration ?? config['layer-duration'] : config['layer-duration'],
        easing: typeof rule === 'object' ? rule.easing ?? config['layer-easing'] : config['layer-easing'],
    };
}
