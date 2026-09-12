import GObject from "gi://GObject";
import Meta from "gi://Meta";
import Shell from "gi://Shell";

// Protocol state belongs to individual wl_surfaces, including subsurfaces.
// Apply it synchronously with the native transaction, before the frame paints.
export class BackgroundEffects {
    constructor(changed) {
        this._effects = new Map();
        this._changed = changed;
        this._signal = 0;
        if (
            !Meta.gnoblin_background_effect_get_region ||
            !Shell.BlurEffect.prototype.set_clip_region ||
            !GObject.signal_lookup("gnoblin-background-effect-changed", Meta.Display)
        )
            return;
        this._signal = global.display.connect("gnoblin-background-effect-changed", (_display, actor) =>
            this._sync(actor),
        );
    }

    refresh() {
        if (this._signal) for (const actor of global.get_window_actors()) this._walk(actor);
    }

    _walk(actor) {
        this._sync(actor);
        for (const child of actor.get_children()) this._walk(child);
    }

    owns(actor) {
        if (!this._signal) return false;
        if (Meta.gnoblin_background_effect_get_region(actor) !== null) return true;
        return actor.get_children().some((child) => this.owns(child));
    }

    _sync(actor) {
        const region = Meta.gnoblin_background_effect_get_region(actor);
        if (region === null) return;
        let entry = this._effects.get(actor);
        if (!entry) {
            entry = { effect: null, destroy: actor.connect("destroy", () => this._effects.delete(actor)) };
            this._effects.set(actor, entry);
        }
        if (!region.is_empty()) {
            if (!entry.effect) {
                entry.effect = new Shell.BlurEffect({
                    mode: Shell.BlurMode.BACKGROUND_MASKED,
                    brightness: 1,
                    radius: 24,
                    "mask-opacity": 0,
                });
                actor.add_effect_with_name("gnoblin-standard-background-blur", entry.effect);
            }
            entry.effect.set_clip_region(region);
        } else if (entry.effect) {
            actor.remove_effect(entry.effect);
            entry.effect = null;
        }
        for (let parent = actor; parent; parent = parent.get_parent()) {
            if (parent.meta_window) {
                this._changed(parent);
                break;
            }
        }
    }

    setRadius(actor, radius) {
        const entry = this._effects.get(actor);
        if (entry?.effect) entry.effect.radius = radius;
        for (const child of actor.get_children()) this.setRadius(child, radius);
    }

    destroy() {
        if (this._signal) global.display.disconnect(this._signal);
        for (const [actor, entry] of this._effects) {
            actor.disconnect(entry.destroy);
            if (entry.effect) actor.remove_effect(entry.effect);
        }
        this._effects.clear();
    }
}
