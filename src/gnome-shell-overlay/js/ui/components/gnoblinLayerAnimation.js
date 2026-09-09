// Layer surfaces use compositor transforms; clients need no animation code.
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

import * as Main from '../main.js';
import * as Config from './gnoblinConfig.js';

const MODES = {
    'ease-out-cubic': Clutter.AnimationMode.EASE_OUT_CUBIC,
    'ease-out-quad': Clutter.AnimationMode.EASE_OUT_QUAD,
    'ease-in-out-cubic': Clutter.AnimationMode.EASE_IN_OUT_CUBIC,
    linear: Clutter.AnimationMode.LINEAR,
};
const interrupted = new WeakMap();

// Return false for ordinary windows so their normal GNOME effects continue.
export function animate(wm, shellwm, actor, opening) {
    if (global.session_mode !== 'gnoblin')
        return false;
    const window = actor.meta_window;
    const anchor = Meta.gnoblin_layer_anchor(window);
    if (anchor < 0)
        return false;

    const {animation, duration, easing} = Config.layerAnimation(Config.windowProperties(window), opening);
    const complete = (finished = true) => {
        if (opening && !finished) {
            // Mutter cancels map effects before emitting destroy. GNOME then
            // resets the actor; retain its visual position for that same event.
            interrupted.set(actor, [actor.translation_x, actor.translation_y, actor.opacity]);
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                interrupted.delete(actor);
                return GLib.SOURCE_REMOVE;
            });
        }
        if (opening)
            wm._mapWindowDone(shellwm, actor);
        else
            wm._destroyWindowDone(shellwm, actor);
    };
    const pending = opening ? wm._mapping : wm._destroying;
    const enabled = animation !== 'none' && duration > 0 &&
        St.Settings.get().enable_animations &&
        wm._shouldAnimateActor(actor, [Meta.WindowType.DOCK, Meta.WindowType.DESKTOP]);

    // Finish an interrupted entrance before taking ownership of the exit.
    const current = interrupted.get(actor) ?? [actor.translation_x, actor.translation_y, actor.opacity];
    interrupted.delete(actor);
    if (!opening)
        wm._mapWindowDone(shellwm, actor);
    pending.add(actor);
    if (!enabled) {
        if (opening)
            actor.show();
        complete();
        return true;
    }

    const monitor = Main.layoutManager.monitors[window.get_monitor()];
    const offset = animation === 'slide'
        ? Config.layerOffset(anchor, window.get_frame_rect(), monitor) : [0, 0];
    const fading = offset.every(value => value === 0);
    actor.translation_x = opening ? offset[0] : current[0];
    actor.translation_y = opening ? offset[1] : current[1];
    actor.opacity = opening ? (fading ? 0 : 255) : current[2];
    actor.show();
    actor.ease({
        translation_x: opening ? 0 : offset[0],
        translation_y: opening ? 0 : offset[1],
        opacity: !opening && fading ? 0 : 255,
        duration,
        mode: MODES[easing],
        onStopped: complete,
    });
    return true;
}
