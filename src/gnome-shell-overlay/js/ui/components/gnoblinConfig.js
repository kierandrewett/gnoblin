// Live shell settings. Protocol registration remains a compositor-startup operation.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const FEATURE_KEYS = Object.freeze([
    'osd', 'osd-volume', 'osd-microphone', 'osd-brightness',
    'osd-keyboard-brightness', 'osd-pad', 'screenshot', 'notifications',
]);

export const DEFAULTS = Object.freeze({
    ...Object.fromEntries(FEATURE_KEYS.map(key => [key, null])),
    'window-switcher': false,
    'minimize-animation': 'fade',
    'minimize-duration': 200,
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

export function parse(text) {
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
            if (!['fade', 'none', 'gnome'].includes(value))
                throw new Error(`${key}: expected fade, none, or gnome`);
            next[key] = value;
            break;
        case 'minimize-duration':
            if (!/^\d+$/.test(value) || Number(value) > 5000)
                throw new Error(`${key}: expected 0 to 5000 milliseconds`);
            next[key] = Number(value);
            break;
        }
    }
    return next;
}

export class ConfigFile {
    constructor(path = GLib.getenv('GNOBLIN_CONFIG') || GLib.build_filenamev([
        GLib.get_user_config_dir(), 'gnoblin', 'gnoblin.conf',
    ]), apply = () => {}) {
        this.path = path;
        this._file = Gio.File.new_for_path(path);
        this._apply = apply;
        this._monitor = null;
        this._timeout = 0;
    }

    reload() {
        let text = '';
        try {
            const [, bytes] = this._file.load_contents(null);
            text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
        } catch (e) {
            if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
                throw e;
        }
        // Parse the complete file before replacing the last valid settings.
        const next = parse(text);
        this._apply(next);
        settings = next;
    }

    start() {
        if (this._monitor)
            return;
        const parent = this._file.get_parent();
        GLib.mkdir_with_parents(parent.get_path(), 0o700);
        // Monitor the directory so editor rename-and-replace saves work too.
        this._monitor = parent.monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
        this._monitor.connect('changed', (_monitor, file, otherFile) => {
            if (!file?.equal(this._file) && !otherFile?.equal(this._file))
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
