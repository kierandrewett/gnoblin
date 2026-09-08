import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Raise existing panel buffers with an independent overlay. This does not need
// a frame or a Wayland request from the process that owns the panels.
export class LayerCompanions {
    constructor() {
        this.requests = [];
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

    update(requests) { this.requests = requests; this.apply(); }

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
        if (Main.sessionMode.isLocked) return;
        const actors = global.get_window_actors().filter(actor => actor.visible && actor.meta_window);
        const namespace = actor => Meta.gnoblin_layer_namespace(actor.meta_window);
        for (const request of this.requests) {
            for (const overlay of actors.filter(actor => namespace(actor) === request.surface)) {
                const parent = overlay.get_parent();
                if (!parent) continue;
                const companions = actors.filter(actor => actor.get_parent() === parent &&
                    actor.meta_window.get_monitor() === overlay.meta_window.get_monitor() && request.companions.includes(namespace(actor)));
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
        this.restore();
        global.display.disconnect(this.restacked);
        global.window_manager.disconnect(this.mapped);
        Main.sessionMode.disconnect(this.session);
        for (const [actor, signals] of this.signals) for (const signal of signals) actor.disconnect(signal);
        this.signals.clear();
    }
}
