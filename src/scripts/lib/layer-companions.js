import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Raise existing panel buffers with an independent overlay. This does not need
// a frame or a Wayland request from the process that owns the panels.
export class LayerCompanions {
    constructor() {
        this.requests = [];
        this.exiting = new Map();
        this.exitRun = null;
        this.saved = new Map();
        this.signals = new Map();
        this.restacked = global.display.connect('restacked', () => {
            // Mutter has supplied a new authoritative order.
            this.saved.clear();
            this.apply();
        });
        this.mapped = global.window_manager.connect('map', (_manager, actor) => {
            this.track(actor);
            this.apply();
        });
        this.session = Main.sessionMode.connect('updated', () => this.apply());
        for (const actor of global.get_window_actors()) this.track(actor);
    }

    track(actor) {
        if (this.signals.has(actor)) return;
        const visible = actor.connect('notify::visible', () => this.apply());
        const destroy = actor.connect('destroy', () => {
            this.signals.delete(actor);
            for (const [parent, children] of this.saved) this.saved.set(parent, children.filter(child => child !== actor));
            this.apply();
        });
        this.signals.set(actor, [visible, destroy]);
    }

    update(requests) { this.cancelDismiss(); this.requests = requests; this.apply(); }

    dismiss(surface, done) {
        if (this.exitRun) return;
        const request = this.requests.find(request => request.surface === surface);
        if (!request) { done(); return; }
        const motions = [];
        for (const actor of global.get_window_actors()) {
            const window = actor.meta_window;
            if (!actor.visible || !window || !request.companions.includes(Meta.gnoblin_layer_namespace(window))) continue;
            const monitor = Main.layoutManager.monitors[window.get_monitor()];
            if (!monitor) continue;
            const frame = window.get_frame_rect();
            // Full-height outlines are not edge panels.
            if (frame.height > monitor.height / 2) continue;
            const offset = frame.y < monitor.y + monitor.height / 2
                ? monitor.y - frame.y - frame.height : monitor.y + monitor.height - frame.y;
            motions.push({actor, offset});
        }
        if (!motions.length) { done(); return; }
        const run = {remaining: motions.length};
        this.exitRun = run;
        for (const {actor, offset} of motions) {
            this.exiting.set(actor, actor.translation_y);
            actor.ease({translation_y: actor.translation_y + offset, duration: 180,
                mode: Clutter.AnimationMode.EASE_IN_QUAD,
                onStopped: () => {
                    if (this.exitRun === run && --run.remaining === 0) done();
                }});
        }
    }

    cancelDismiss() {
        this.exitRun = null;
        for (const [actor, translation] of this.exiting) {
            if (!this.signals.has(actor)) continue;
            actor.remove_transition('translation-y');
            actor.translation_y = translation;
        }
        this.exiting.clear();
    }

    restore() {
        for (const [parent, children] of this.saved) {
            const live = parent.get_children();
            let previous = null;
            for (const child of children) {
                if (!live.includes(child)) continue;
                parent.set_child_above_sibling(child, previous);
                previous = child;
            }
        }
        this.saved.clear();
    }

    apply() {
        this.restore();
        if (Main.sessionMode.isLocked || !this.requests.length) return;
        const byNamespace = new Map();
        const byParent = new Map();
        for (const actor of global.get_window_actors()) {
            if (!actor.visible || !actor.meta_window) continue;
            const namespace = Meta.gnoblin_layer_namespace(actor.meta_window);
            if (typeof namespace !== 'string') continue;
            const parent = actor.get_parent();
            const entry = {actor, namespace};
            if (!byNamespace.has(namespace)) byNamespace.set(namespace, []);
            byNamespace.get(namespace).push(entry);
            if (!parent) continue;
            if (!byParent.has(parent)) byParent.set(parent, []);
            byParent.get(parent).push(entry);
        }
        for (const request of this.requests) {
            for (const {actor: overlay} of byNamespace.get(request.surface) ?? []) {
                const parent = overlay.get_parent();
                if (!parent) continue;
                const companions = (byParent.get(parent) ?? []).filter(entry =>
                    entry.actor.meta_window.get_monitor() === overlay.meta_window.get_monitor() &&
                    request.companions.includes(entry.namespace)).map(entry => entry.actor);
                if (!companions.length) continue;
                if (!this.saved.has(parent)) this.saved.set(parent, parent.get_children());
                if (request.companionsAbove) {
                    let sibling = overlay;
                    for (const actor of parent.get_children().filter(child => companions.includes(child))) {
                        parent.set_child_above_sibling(actor, sibling);
                        sibling = actor;
                    }
                } else {
                    for (const actor of companions) parent.set_child_below_sibling(actor, overlay);
                }
            }
        }
    }

    destroy() {
        this.cancelDismiss();
        this.restore();
        global.display.disconnect(this.restacked);
        global.window_manager.disconnect(this.mapped);
        Main.sessionMode.disconnect(this.session);
        for (const [actor, signals] of this.signals) for (const signal of signals) actor.disconnect(signal);
        this.signals.clear();
    }
}
