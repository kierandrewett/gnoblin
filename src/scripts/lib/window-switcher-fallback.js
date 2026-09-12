import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Keep switching in the compositor. The optional UI gets the first chance to
// commit its selection. If it stalls, modifier release still changes focus.
export class WindowSwitcherFallback {
    constructor(bridge) {
        this.bridge = bridge;
        this.client = {fallback: true, bindings: new Map()};
        this.bindings = new Map();
        this.owners = new WeakSet();
        this.serial = 0;
        this.gesture = null;
        this.pending = null;
        this.timer = 0;
        this.shortcuts = [
            ['<Alt>Tab', Clutter.ModifierType.MOD1_MASK, false],
            ['<Alt><Shift>Tab', Clutter.ModifierType.MOD1_MASK, true],
            ['<Super>Tab', Clutter.ModifierType.SUPER_MASK, false],
            ['<Super><Shift>Tab', Clutter.ModifierType.SUPER_MASK, true],
        ];
        // A session script may release the native GNOME bindings after load.
        this.retry = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 500, () => {
            this.ensure();
            if (this.bindings.size === this.shortcuts.length) {
                this.retry = 0;
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
        this.ensure();
    }

    ensure() {
        for (const [accelerator, hold, backwards] of this.shortcuts) {
            if (this.bindings.has(accelerator)) continue;
            const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
            if (action === Meta.KeyBindingAction.NONE) continue;
            const binding = {client: this.client, id: accelerator, hold, modal: true, action, fallback: true, backwards};
            this.bindings.set(accelerator, binding);
            this.bridge.actions.set(action, binding);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), Shell.ActionMode.NORMAL);
        }
    }

    claim(client, record) {
        if (!this.shortcuts.some(([accelerator]) => accelerator === record.accelerator)) return null;
        this.ensure();
        const base = this.bindings.get(record.accelerator);
        if (!base) return null;
        const current = this.bridge.actions.get(base.action);
        if (current.client !== this.client && current.client !== client) throw new Error('Switcher shortcut already owned');
        if (record.hold !== base.hold || record.modal === false) throw new Error('Switcher requires a modal modifier hold');
        this.owners.add(client);
        return {...base, client, id: record.id};
    }

    release(binding) {
        if (!binding.fallback) return false;
        const base = [...this.bindings.values()].find(item => item.action === binding.action);
        this.bridge.actions.set(binding.action, base);
        return true;
    }

    step(binding, first) {
        if (!binding.fallback) return 0;
        if (first) {
            // Finish a previous release before a rapid second gesture can
            // replace its fallback. Frozen clients cannot accumulate grabs.
            this.commitPending();
            this.bridge.uiSessions.command(this.client, {action: 'command', name: 'search', command: {action: 'close'}});
            const windows = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null)
                .filter(window => this.bridge.eligible(window));
            this.gesture = {serial: ++this.serial, client: binding.client, windows, selected: 0};
        }
        this.move(binding.backwards ? -1 : 1);
        return this.gesture?.serial || 0;
    }

    move(delta) {
        const gesture = this.gesture;
        if (!gesture) return;
        const selected = gesture.windows[gesture.selected];
        const live = new Set(global.display.list_all_windows());
        gesture.windows = gesture.windows.filter(window => live.has(window));
        const retained = gesture.windows.indexOf(selected);
        gesture.selected = retained >= 0 ? retained : 0;
        if (gesture.windows.length)
            gesture.selected = (gesture.selected + delta + gesture.windows.length) % gesture.windows.length;
    }

    key(key) {
        if (!this.gesture) return false;
        if (key === Clutter.KEY_Escape) { this.bridge.end('cancelled'); return true; }
        if (key === Clutter.KEY_Return || key === Clutter.KEY_KP_Enter) { this.bridge.end('released'); return true; }
        if ([Clutter.KEY_Left, Clutter.KEY_Up].includes(key)) this.move(-1);
        if ([Clutter.KEY_Right, Clutter.KEY_Down].includes(key)) this.move(1);
        return false;
    }

    end(active, reason) {
        if (!active.fallback || !this.gesture) return;
        const gesture = this.gesture;
        this.gesture = null;
        if (reason !== 'released' || Main.sessionMode.isLocked) return;
        this.pending = gesture;
        if (gesture.client === this.client || gesture.client.closed) this.commitPending();
        else this.timer = GLib.timeout_add(GLib.PRIORITY_HIGH, 80, () => {
            this.timer = 0;
            this.commitPending();
            return GLib.SOURCE_REMOVE;
        });
    }

    commitPending() {
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        const gesture = this.pending;
        this.pending = null;
        if (!gesture || Main.sessionMode.isLocked) return;
        const live = new Set(global.display.list_all_windows());
        const windows = gesture.windows.filter(window => live.has(window));
        const selected = gesture.windows[gesture.selected];
        const window = live.has(selected) ? selected : windows[Math.min(gesture.selected, windows.length - 1)];
        if (window) Main.activateWindow(window, global.get_current_time());
    }

    // Serial numbers reject old activation messages from a resumed UI. They
    // must never undo a later fallback switch or a user's subsequent focus.
    accept(client, serial) {
        if (!this.owners.has(client)) return true;
        const gesture = this.gesture || this.pending;
        if (!gesture || gesture.client !== client || gesture.serial !== serial) return false;
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        this.pending = null;
        return true;
    }

    close(client) {
        if (this.bridge.active?.client === client && this.bridge.active.fallback) {
            this.bridge.active.client = this.client;
            if (this.gesture) this.gesture.client = this.client;
        }
        if (this.pending?.client === client) this.commitPending();
    }

    destroy() {
        if (this.retry) GLib.source_remove(this.retry);
        if (this.timer) GLib.source_remove(this.timer);
        this.pending = null;
        this.gesture = null;
        for (const binding of this.bindings.values()) {
            global.display.ungrab_accelerator(binding.action);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(binding.action), Shell.ActionMode.NONE);
            this.bridge.actions.delete(binding.action);
        }
    }
}
