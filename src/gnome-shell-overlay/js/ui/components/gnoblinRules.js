import Shell from 'gi://Shell';
import * as Config from './gnoblinConfig.js';

export class WindowRules {
    constructor() {
        this._config = Config.settings;
        this._actors = new Map();
        this._map = global.window_manager.connect('map', (_wm, actor) => this._apply(actor));
        this._focus = global.display.connect('notify::focus-window', () => this.refresh());
    }

    refresh(config = this._config) {
        this._config = config;
        for (const actor of global.get_window_actors())
            this._apply(actor);
    }

    _apply(actor) {
        const surface = actor.get_first_child();
        if (!surface || !actor.meta_window) return;
        let entry = this._actors.get(actor);
        if (!entry) {
            const title = actor.meta_window.connect('notify::title', () => this._apply(actor));
            const destroy = actor.connect('destroy', () => {
                actor.meta_window?.disconnect(title);
                this._actors.delete(actor);
            });
            entry = {surface, title, destroy, opacity: surface.opacity, blur: null};
            this._actors.set(actor, entry);
        }
        const effects = Config.windowEffects(Config.windowProperties(actor.meta_window), this._config);
        surface.opacity = Math.round(entry.opacity * effects.opacity);
        if (effects.blur > 0) {
            if (!entry.blur) {
                entry.blur = new Shell.BlurEffect({mode: Shell.BlurMode.BACKGROUND_MASKED, brightness: 1});
                surface.add_effect_with_name('gnoblin-window-blur', entry.blur);
            }
            entry.blur.radius = effects.blur;
        } else if (entry.blur) {
            surface.remove_effect(entry.blur);
            entry.blur = null;
        }
    }

    destroy() {
        global.window_manager.disconnect(this._map);
        global.display.disconnect(this._focus);
        for (const [actor, entry] of this._actors) {
            actor.disconnect(entry.destroy);
            actor.meta_window.disconnect(entry.title);
            entry.surface.opacity = entry.opacity;
            if (entry.blur) entry.surface.remove_effect(entry.blur);
        }
        this._actors.clear();
    }
}
