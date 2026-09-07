import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BINDINGS = ['switch-applications', 'switch-applications-backward',
    'switch-windows', 'switch-windows-backward'];
const TILE = 'padding: 12px; border-radius: 10px; border: 1px solid transparent;';
const SELECTED = `${TILE} background-color: #354656; border-color: #99c1f1;`;

// Window order is captured once per gesture. Icon actors are prepared when
// windows map, and are never needed to commit a quick Alt+Tab.
class WindowSwitcher {
    constructor() {
        this.options = {enabled: true, delay: 80, workspaceOnly: false};
        this.windows = [];
        this.tiles = new Map();
        this.signals = [];
        this.index = 0;
        this.grab = null;
        this.delay = 0;
        this.warm = 0;
        this.reload = 0;
        this.registered = false;
        this.actor = new St.Widget({name: 'gnoblin-window-switcher', visible: false,
            reactive: true, layout_manager: new Clutter.BinLayout()});
        this.actor.add_constraint(new Clutter.BindConstraint({source: global.stage,
            coordinate: Clutter.BindCoordinate.ALL}));
        this.card = new St.BoxLayout({vertical: true,
            style: 'background-color: rgba(27,27,27,0.97); color: #fafafa; border: 1px solid #505050; border-radius: 16px; padding: 16px; spacing: 12px;',
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER});
        this.row = new St.BoxLayout({style: 'spacing: 4px;', x_align: Clutter.ActorAlign.CENTER});
        this.title = new St.Label({style: 'font-family: Adwaita Sans; font-size: 14px; text-align: center;',
            x_align: Clutter.ActorAlign.CENTER});
        this.card.add_child(this.row);
        this.card.add_child(this.title);
        this.actor.add_child(this.card);
        Main.uiGroup.add_child(this.actor);
        this.actor.connect('key-press-event', (_actor, event) => this.keyPress(event));
        this.actor.connect('key-release-event', (_actor, event) => {
            if (this.grab && !(global.get_pointer()[2] & this.mask)) this.finish(event.get_time());
            return Clutter.EVENT_STOP;
        });
        this.actor.connect('button-press-event', () => { this.cancel(); return Clutter.EVENT_STOP; });
        this.connect(global.window_manager, 'map', () => this.queueWarm());
        this.connect(global.display, 'notify::focus-window', () => this.queueWarm());
        this.connect(Main.layoutManager, 'monitors-changed', () => this.cancel());
        this.connect(Main.layoutManager, 'system-modal-opened', () => this.cancel());
        this.connect(Main.sessionMode, 'updated', () => { if (Main.sessionMode.isLocked) this.cancel(); });

        this.file = Gio.File.new_for_path(GLib.getenv('GNOBLIN_CONFIG') ||
            GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin', 'gnoblin.toml']));
        this.monitor = this.file.get_parent().monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
        this.monitor.connect('changed', (_monitor, file, other) => {
            if (!file?.equal(this.file) && !other?.equal(this.file)) return;
            if (this.reload) GLib.source_remove(this.reload);
            this.reload = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
                this.reload = 0;
                this.readConfig();
                return GLib.SOURCE_REMOVE;
            });
        });
        this.readConfig();
        this.queueWarm();
    }

    connect(object, signal, callback) {
        this.signals.push([object, object.connect(signal, callback)]);
    }

    readConfig() {
        try {
            const document = this.file.query_exists(null)
                ? this.parseConfig(new TextDecoder().decode(this.file.load_contents(null)[1])) : {};
            const options = document.switcher ?? {};
            if (!options || typeof options !== 'object' || Array.isArray(options) ||
                Object.keys(options).some(key => !['enabled', 'show-delay', 'current-workspace-only'].includes(key)) ||
                (options.enabled !== undefined && typeof options.enabled !== 'boolean') ||
                (options['current-workspace-only'] !== undefined && typeof options['current-workspace-only'] !== 'boolean') ||
                (options['show-delay'] !== undefined && (!Number.isInteger(options['show-delay']) || options['show-delay'] < 0 || options['show-delay'] > 500)))
                throw new Error('invalid [switcher] settings');
            const next = {enabled: options.enabled ?? true, delay: options['show-delay'] ?? 80,
                workspaceOnly: options['current-workspace-only'] ?? false};
            this.cancel();
            this.options = next;
            this.bind(next.enabled);
        } catch (error) {
            console.warn(`gnoblin-switcher: keeping previous configuration: ${error.message}`);
        }
    }

    parseConfig(text) {
        if (typeof Meta.gnoblin_parse_toml === 'function')
            return Meta.gnoblin_parse_toml(text).recursiveUnpack();
        // Compatibility for a session started before native TOML was installed.
        // This runs only on config changes, never in the keyboard event path.
        const process = Gio.Subprocess.new(['python3', '-c',
            'import json,sys,tomllib; print(json.dumps(tomllib.loads(sys.stdin.read())))'],
        Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
        const [, output, error] = process.communicate_utf8(text, null);
        if (!process.get_successful()) throw new Error(error);
        return JSON.parse(output);
    }

    bind(enabled) {
        if (enabled === this.registered) return;
        for (const name of BINDINGS) {
            Main.wm.setCustomKeybindingHandler(name, Shell.ActionMode.NORMAL, enabled
                ? (_display, _window, _event, binding) => this.begin(binding)
                : Main.wm._startSwitcher.bind(Main.wm));
        }
        this.registered = enabled;
        console.log(`gnoblin-switcher: ${enabled ? 'enabled' : 'disabled'}`);
    }

    list() {
        const workspace = this.options.workspaceOnly ? global.workspace_manager.get_active_workspace() : null;
        return [...new Set(global.display.get_tab_list(Meta.TabList.NORMAL_ALL, workspace)
            .map(window => window.is_attached_dialog() ? window.get_transient_for() : window))]
            .filter(window => window && !window.skip_taskbar && !window.is_override_redirect());
    }

    queueWarm() {
        if (this.warm) return;
        this.warm = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this.warm = 0;
            for (const window of this.list()) this.tile(window);
            return GLib.SOURCE_REMOVE;
        });
    }

    tile(window) {
        if (this.tiles.has(window)) return this.tiles.get(window).actor;
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const button = new St.Button({style: TILE, can_focus: false, reactive: true,
            accessible_name: window.title || app?.get_name() || 'Window'});
        const icon = app?.get_app_info()?.get_icon();
        button.set_child(icon ? new St.Icon({gicon: icon, icon_size: 48})
            : new St.Icon({icon_name: 'application-x-executable', icon_size: 48}));
        button.connect('clicked', () => {
            const index = this.windows.indexOf(window);
            if (index < 0) return;
            this.index = index;
            this.finish(global.get_current_time());
        });
        const signal = window.connect('unmanaged', () => {
            this.tiles.delete(window);
            const index = this.windows.indexOf(window);
            if (index >= 0) {
                this.windows.splice(index, 1);
                if (index < this.index) this.index--;
                this.index = Math.min(this.index, this.windows.length - 1);
                if (!this.windows.length) this.cancel();
                else if (this.grab && this.actor.opacity) this.render();
            }
            button.destroy();
        });
        this.tiles.set(window, {actor: button, signal});
        return button;
    }

    begin(binding) {
        if (this.grab) return;
        this.windows = this.list();
        if (!this.windows.length) return;
        const focused = global.display.focus_window;
        const origin = Math.max(0, this.windows.indexOf(focused));
        this.index = (origin + (binding.is_reversed() ? -1 : 1) + this.windows.length) % this.windows.length;
        this.mask = binding.get_mask();
        // Only Alt/Super/Control sustain a gesture. Shift controls direction.
        this.mask &= ~Clutter.ModifierType.SHIFT_MASK;
        if (!this.mask || !(global.get_pointer()[2] & this.mask)) {
            this.finish(global.get_current_time());
            return;
        }
        this.actor.opacity = 0;
        this.actor.show();
        const grab = Main.pushModal(this.actor, {actionMode: Shell.ActionMode.POPUP});
        if (!(grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
            Main.popModal(grab);
            this.cancel();
            return;
        }
        this.grab = grab;
        if (!(global.get_pointer()[2] & this.mask)) {
            this.finish(global.get_current_time());
            return;
        }
        this.delay = GLib.timeout_add(GLib.PRIORITY_DEFAULT, Math.max(1, this.options.delay), () => {
            this.delay = 0;
            this.render();
            this.actor.opacity = 255;
            return GLib.SOURCE_REMOVE;
        });
    }

    keyPress(event) {
        const key = event.get_key_symbol();
        if (key === Clutter.KEY_Escape) this.cancel();
        else if (key === Clutter.KEY_Return || key === Clutter.KEY_KP_Enter) this.finish(event.get_time());
        else if ([Clutter.KEY_Tab, Clutter.KEY_ISO_Left_Tab, Clutter.KEY_Left, Clutter.KEY_Right].includes(key)) {
            const backwards = key === Clutter.KEY_Left || key === Clutter.KEY_ISO_Left_Tab ||
                (key === Clutter.KEY_Tab && (event.get_state() & Clutter.ModifierType.SHIFT_MASK));
            this.index = (this.index + (backwards ? -1 : 1) + this.windows.length) % this.windows.length;
            if (this.actor.opacity) this.render();
        }
        return Clutter.EVENT_STOP;
    }

    render() {
        if (!this.windows.length) return;
        // No thumbnails, background capture, pointer-hover selection or fades.
        // Keep the active monitor's chooser bounded even with many windows.
        const monitor = Main.layoutManager.focusMonitor ?? Main.layoutManager.primaryMonitor;
        this.card.translation_x = monitor.x + monitor.width / 2 - global.stage.width / 2;
        this.card.translation_y = monitor.y + monitor.height / 2 - global.stage.height / 2;
        const count = Math.max(1, Math.min(9, Math.floor((monitor.width - 64) / 78)));
        const start = Math.min(Math.max(0, this.index - Math.floor(count / 2)), Math.max(0, this.windows.length - count));
        this.row.remove_all_children();
        for (let i = start; i < Math.min(this.windows.length, start + count); i++) {
            const tile = this.tile(this.windows[i]);
            tile.style = i === this.index ? SELECTED : TILE;
            this.row.add_child(tile);
        }
        const window = this.windows[this.index];
        this.title.text = `${window.title || 'Window'}   ·   ${this.index + 1} / ${this.windows.length}`;
        this.title.width = Math.min(monitor.width - 96, Math.max(240, Math.min(count, this.windows.length) * 78));
    }

    finish(time) {
        const window = this.windows[this.index];
        this.cancel();
        if (window && window.get_compositor_private()) Main.activateWindow(window, time);
    }

    cancel() {
        if (this.delay) GLib.source_remove(this.delay);
        this.delay = 0;
        if (this.grab) {
            const grab = this.grab;
            this.grab = null;
            Main.popModal(grab);
        }
        this.actor.hide();
        this.windows = [];
    }

    destroy() {
        this.cancel();
        this.bind(false);
        this.monitor.cancel();
        for (const id of [this.warm, this.reload]) if (id) GLib.source_remove(id);
        for (const [object, signal] of this.signals) object.disconnect(signal);
        for (const [window, tile] of this.tiles) {
            window.disconnect(tile.signal);
            tile.actor.destroy();
        }
        this.tiles.clear();
        this.actor.destroy();
    }
}

export default function enable(api) {
    const switcher = new WindowSwitcher();
    api._disposers.push(() => switcher.destroy());
}
