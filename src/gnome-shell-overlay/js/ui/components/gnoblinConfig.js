// Live shell settings. Protocol registration remains a compositor-startup operation.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

export const FEATURE_KEYS = Object.freeze([
    'osd', 'osd-volume', 'osd-microphone', 'osd-brightness',
    'osd-keyboard-brightness', 'osd-pad', 'screenshot', 'notifications',
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
    autostart: [],
    'window-rules': [],
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
            continue;
        }
        if (section !== 'shell')
            continue;
        const separator = line.indexOf('=');
        const key = line.slice(0, separator).trim();
        const value = cleanValue(line.slice(separator + 1));
        if (separator < 0 || !Object.hasOwn(DEFAULTS, key))
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
    const document = Meta.gnoblin_parse_toml(text).recursiveUnpack();
    const next = {...DEFAULTS};
    const shell = document.shell ?? {};
    if (!shell || Array.isArray(shell) || typeof shell !== 'object')
        throw new Error('shell must be a table');
    for (const [key, value] of Object.entries(shell)) {
        if (!Object.hasOwn(DEFAULTS, key) || key === 'autostart' || key === 'window-rules')
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
            Object.keys(rule).some(key => !['match', 'blur', 'opacity', 'animation'].includes(key)))
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
        if (rule.blur !== undefined && (!Number.isInteger(rule.blur) || rule.blur < 0 || rule.blur > 100))
            throw new Error('blur must be an integer from 0 to 100');
        if (rule.opacity !== undefined && (typeof rule.opacity !== 'number' || !Number.isFinite(rule.opacity) || rule.opacity < 0 || rule.opacity > 1))
            throw new Error('opacity must be between 0 and 1');
        if (rule.animation !== undefined && !['slide', 'fade', 'none'].includes(rule.animation))
            throw new Error('rule animation must be slide, fade, or none');
    }
    next['window-rules'] = rules;
    return next;
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
    constructor(path = null, apply = () => {}) {
        this._override = path || GLib.getenv('GNOBLIN_CONFIG') || null;
        this._directory = this._override ? GLib.path_get_dirname(this._override) :
            GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin']);
        this._apply = apply;
        this._monitor = null;
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
        try {
            const [, bytes] = Gio.File.new_for_path(this.path).load_contents(null);
            text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
        } catch (e) {
            if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
                throw e;
        }
        // Parse the complete file before replacing the last valid settings.
        const next = this.path.endsWith('.conf') ? parseLegacy(text) : parse(text);
        this._apply(next);
        settings = next;
    }

    start() {
        if (this._monitor)
            return;
        const parent = Gio.File.new_for_path(this._directory);
        GLib.mkdir_with_parents(parent.get_path(), 0o700);
        // Monitor the directory so editor rename-and-replace saves work too.
        this._monitor = parent.monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
        this._monitor.connect('changed', (_monitor, file, otherFile) => {
            const names = this._override ? [GLib.path_get_basename(this._override)] :
                ['gnoblin.toml', 'gnoblin.conf'];
            if (!names.includes(file?.get_basename()) && !names.includes(otherFile?.get_basename()))
                return;
            if (this._timeout)
                GLib.source_remove(this._timeout);
            this._timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
                this._timeout = 0;
                this._tryReload();
                return GLib.SOURCE_REMOVE;
            });
        });
        this._tryReload();
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
        this._monitor?.cancel();
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
    const effects = {blur: 0, opacity: 1, animation: config['layer-animation']};
    for (const rule of config['window-rules']) {
        if (Object.entries(rule.match).every(([key, value]) =>
            key === 'type' || key === 'focused' ? properties[key] === value :
                properties[key] !== null && new RegExp(value).test(properties[key] ?? ''))) {
            for (const key of ['blur', 'opacity', 'animation'])
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
