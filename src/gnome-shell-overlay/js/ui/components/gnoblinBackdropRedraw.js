import Clutter from 'gi://Clutter';

// Older native builds cannot expand damage around blur dependencies. Keep
// their full-redraw fallback until the installed native effect is loaded.
// Neither policy schedules frames or disables occlusion culling.
export class BackdropRedraw {
    constructor() {
        this._actors = new Map();
        this._ownsFlag = false;
    }

    set(actor, enabled) {
        // Native effects maintain their own damage dependencies. Only legacy
        // effects need these lifetime handlers and the full-redraw fallback.
        enabled = enabled && !actor.get_effect?.('gnoblin-window-blur')?.uses_damage_tracking?.();
        if (enabled === this._actors.has(actor)) return;
        if (enabled) {
            const mapped = actor.connect('notify::mapped', () => this._sync());
            const destroy = actor.connect('destroy', () => this.set(actor, false));
            this._actors.set(actor, {mapped, destroy});
        } else {
            const entry = this._actors.get(actor);
            actor.disconnect(entry.mapped);
            actor.disconnect(entry.destroy);
            this._actors.delete(actor);
        }
        this._sync();
    }

    _sync() {
        let needed = false;
        for (const actor of this._actors.keys()) {
            if (actor.mapped) { needed = true; break; }
        }
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
        for (const [actor, entry] of this._actors) {
            actor.disconnect(entry.mapped);
            actor.disconnect(entry.destroy);
        }
        this._actors.clear();
        this._sync();
    }
}
