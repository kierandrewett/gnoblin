import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Presentation belongs to the client. The compositor owns the drag lifetime,
// hit testing at release, work-area bounds, and the final window operation.
export class WindowSnap {
    constructor(bridge) {
        this.bridge = bridge;
        this.serial = 0;
        this.drag = null;
        this.timer = 0;
        this.pending = 0;
        this.saved = new Map();
        this.signals = [
            global.display.connect('grab-op-begin', (_display, window, op) => {
                if ((op & ~1024) === Meta.GrabOp.MOVING) this.begin(window);
            }),
            global.display.connect('grab-op-end', () => this.finish()),
        ];
        this.key = global.stage.connect('captured-event', (_stage, event) => {
            if (this.drag && event.type() === Clutter.EventType.KEY_PRESS && event.get_key_symbol() === Clutter.KEY_Escape)
                this.cancel();
            return Clutter.EVENT_PROPAGATE;
        });
    }

    subscribe(client) {
        client.trackSnap = true;
        this.publish(client);
    }

    begin(window) {
        this.cancel();
        if (!this.bridge.eligible(window) || !window.allows_move() || (!window.allows_resize() && !window.get_maximize_flags()) || window.is_fullscreen()) return;
        if (!this.hasSubscribers()) return;
        const id = String(window.get_stable_sequence());
        const previous = this.saved.get(id);
        if (previous) {
            const frame = window.get_frame_rect();
            const [x, y] = global.get_pointer();
            const ratio = Math.max(0, Math.min(1, (x - frame.x) / frame.width));
            window.move_resize_frame(true, Math.round(x - previous.width * ratio),
                Math.round(y - Math.min(32, y - frame.y)), previous.width, previous.height);
            this.saved.delete(id);
        }
        this.drag = {serial: ++this.serial, window, original: window.get_frame_rect(), offers: [], owner: null};
        this.publish();
        this.timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
            this.publish();
            return GLib.SOURCE_CONTINUE;
        });
    }

    state() {
        if (!this.drag || Main.sessionMode.isLocked) return {event: 'window-drag', active: false};
        const [x, y, modifiers] = global.get_pointer();
        const monitor = Main.layoutManager.monitors.find(m => x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height);
        if (!monitor) return {event: 'window-drag', active: false};
        const area = this.drag.window.get_workspace().get_work_area_for_monitor(monitor.index);
        return {event: 'window-drag', active: true, serial: this.drag.serial,
            window: String(this.drag.window.get_stable_sequence()), x, y, modifiers,
            monitor: {id: monitor.index, x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height},
            area: {x: area.x, y: area.y, width: area.width, height: area.height}};
    }

    publish(client = null) {
        const state = this.state();
        const key = JSON.stringify(state);
        if (!client && key === this.last) return;
        this.last = key;
        if (client) {
            this.bridge.send(client, state);
            return;
        }
        for (const peer of this.bridge.clients)
            if (peer.trackSnap) this.bridge.send(peer, state);
    }

    hasSubscribers() {
        for (const client of this.bridge.clients)
            if (client.trackSnap) return true;
        return false;
    }

    offer(client, record) {
        if (!this.drag || record.serial !== this.drag.serial || !client.trackSnap) return;
        if (this.drag.owner && this.drag.owner !== client) throw new Error('drag already controlled');
        if (!Array.isArray(record.regions) || record.regions.length > 128) throw new Error('invalid snap regions');
        for (const region of record.regions) {
            for (const rect of [region.hit, region.target]) {
                if (!rect || ![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite)
                    || Math.abs(rect.x) > 100000 || Math.abs(rect.y) > 100000
                    || rect.width < 1 || rect.height < 1 || rect.width > 32768 || rect.height > 32768)
                    throw new Error('invalid snap rectangle');
            }
        }
        this.drag.owner = client;
        this.drag.offers = record.regions;
    }

    restoreOrMinimize(window) {
        const id = String(window.get_stable_sequence());
        const saved = this.saved.get(id);
        if (window.get_maximize_flags()) {
            window.unmaximize(Meta.MaximizeFlags.BOTH);
            this.saved.delete(id);
        } else if (saved) {
            window.move_resize_frame(true, saved.x, saved.y, saved.width, saved.height);
            this.saved.delete(id);
        } else {
            if (!window.can_minimize()) throw new Error('window cannot be minimized');
            window.minimize();
        }
    }

    apply(window, target, monitorId, original = null, maximize = false) {
        if (Main.sessionMode.isLocked || !window || !target || !this.bridge.eligible(window) || (!window.allows_resize() && !window.get_maximize_flags())
            || !window.allows_move() || window.is_fullscreen()) throw new Error('window cannot snap');
        const monitor = Main.layoutManager.monitors.find(m => m.index === monitorId);
        if (!monitor) throw new Error('monitor no longer available');
        const area = window.get_workspace().get_work_area_for_monitor(monitorId);
        if (![target.x, target.y, target.width, target.height].every(Number.isFinite)
            || target.width < 1 || target.height < 1 || target.x < area.x || target.y < area.y
            || target.x + target.width > area.x + area.width + 1 || target.y + target.height > area.y + area.height + 1)
            throw new Error('snap region is outside the work area');
        const id = String(window.get_stable_sequence());
        if (maximize) {
            if (!window.can_maximize()) throw new Error('window cannot be maximized');
            this.saved.delete(id);
            window.move_to_monitor(monitorId);
            window.maximize(Meta.MaximizeFlags.BOTH);
            Main.activateWindow(window, global.get_current_time());
            return;
        }
        if (!this.saved.has(id)) this.saved.set(id, original || window.get_frame_rect());
        if (window.unmaximize.length === 0) window.unmaximize();
        else window.unmaximize(Meta.MaximizeFlags.BOTH);
        window.move_to_monitor(monitorId);
        Main.wm._prepareAnimationInfo?.(global.window_manager, window.get_compositor_private(), window.get_frame_rect().copy(), Meta.SizeChange.UNMAXIMIZE);
        window.move_resize_frame(true, Math.round(target.x), Math.round(target.y), Math.round(target.width), Math.round(target.height));
        Main.activateWindow(window, global.get_current_time());
    }

    finish() {
        const drag = this.drag;
        if (!drag) return;
        const state = this.state();
        const region = drag.offers.find(r => state.active && state.x >= r.hit.x && state.x < r.hit.x + r.hit.width
            && state.y >= r.hit.y && state.y < r.hit.y + r.hit.height
            && (r.control ? Boolean(state.modifiers & Clutter.ModifierType.CONTROL_MASK) : !(state.modifiers & Clutter.ModifierType.CONTROL_MASK)));
        this.cancel();
        // Mutter consumes Escape before stage event handlers. A move ending while
        // a pointer button remains held is cancellation, not a drop.
        if (!region || drag.owner?.closed || (state.modifiers & 0x1f00)) return;
        // Finish after Mutter's own edge-tiling handler, in the same event turn.
        this.pending = GLib.idle_add(GLib.PRIORITY_HIGH_IDLE, () => {
            this.pending = 0;
            try {
                this.apply(drag.window, region.target, state.monitor.id, drag.original, region.maximize === true);
                this.bridge.send(drag.owner, {event: 'snap-completed', layout: region.layout});
            }
            catch (error) { this.bridge.send(drag.owner, {event: 'error', message: error.message}); }
            return GLib.SOURCE_REMOVE;
        });
    }

    cancel() {
        if (this.pending) GLib.source_remove(this.pending);
        this.pending = 0;
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        this.drag = null;
        this.publish();
    }

    close(client) { if (this.drag?.owner === client) this.cancel(); }
    forget(id) { this.saved.delete(id); }
    destroy() {
        this.cancel();
        for (const signal of this.signals) global.display.disconnect(signal);
        global.stage.disconnect(this.key);
        this.saved.clear();
    }
}
