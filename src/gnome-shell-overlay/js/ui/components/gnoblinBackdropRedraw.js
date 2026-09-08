import Clutter from 'gi://Clutter';

// Backdrop sampling needs pixels outside the current damage rectangle. Limit
// the full-redraw fallback to mapped menus; do not disable desktop culling.
export class BackdropRedraw {
    constructor() {
        this._actors = new Map();
        this._ownsFlag = false;
    }

    set(actor, enabled) {
        if (enabled && !this._actors.has(actor)) {
            const mapped = actor.connect('notify::mapped', () => this._sync());
            const destroy = actor.connect('destroy', () => this.set(actor, false));
            this._actors.set(actor, {mapped, destroy});
        } else if (!enabled && this._actors.has(actor)) {
            const entry = this._actors.get(actor);
            actor.disconnect(entry.mapped);
            actor.disconnect(entry.destroy);
            this._actors.delete(actor);
        }
        this._sync();
    }

    _sync() {
        const needed = [...this._actors.keys()].some(actor => actor.mapped);
        const flag = Clutter.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS;
        if (needed && !this._ownsFlag && !(Clutter.get_debug_flags()[1] & flag)) {
            Clutter.add_debug_flags(0, flag, 0);
            this._ownsFlag = true;
        } else if (!needed && this._ownsFlag) {
            Clutter.remove_debug_flags(0, flag, 0);
            this._ownsFlag = false;
        }
    }

    destroy() {
        for (const actor of [...this._actors.keys()]) this.set(actor, false);
    }
}
