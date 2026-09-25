// Layer-shell surfaces share the same animation definitions and sampler as
// ordinary windows. Their offsets are compositor translations, so the client
// never sees temporary geometry changes.
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import St from "gi://St";

import * as Main from "../main.js";
import * as Config from "./gnoblinConfig.js";
import { resolve, run } from "./gnoblinAnimation.js";

const interrupted = new WeakMap();
const controllers = new WeakMap();

// Return false for ordinary windows so stock GNOME effects continue.
export function animate(wm, shellwm, actor, opening) {
    if (global.session_mode !== "gnoblin") return false;
    const window = actor.meta_window;
    const anchor = Meta.gnoblin_layer_anchor(window);
    if (anchor < 0) return false;

    const event = opening ? "layer-open" : "layer-close";
    const policy = Config.layerAnimation(Config.windowProperties(window), opening);
    const { animation, duration, easing } = policy;
    const monitor = Main.layoutManager.monitors[window.get_monitor()];
    const offset = ["slide", "gnome", "gnoblin-layer-open", "gnoblin-layer-close"].includes(animation)
        ? Config.layerOffset(anchor, window.get_frame_rect(), monitor)
        : [0, 0];
    const previous = controllers.get(actor);
    // Retain the exact visual state if Mutter interrupts an entrance to begin
    // the matching exit.
    if (previous && !previous.finished) previous.cancel();
    const current = interrupted.get(actor) ?? {
        x: actor.translation_x,
        y: actor.translation_y,
        opacity: (actor.opacity ?? 255) / 255,
    };
    interrupted.delete(actor);

    const custom =
        Config.windowAnimation?.(Config.windowProperties(window), event) ??
        Config.getAnimation?.(animation, event) ??
        null;
    const spec = resolve(
        animation,
        event,
        {
            actor,
            window,
            monitor,
            offset,
        },
        custom,
    );
    if (custom?.from) spec.from = { ...spec.from, ...custom.from };
    if (custom?.to) spec.to = { ...spec.to, ...custom.to };
    if (animation === "fade") {
        spec.from = opening ? { opacity: 0 } : { opacity: 1 };
        spec.to = opening ? { opacity: 1 } : { opacity: 0 };
    }
    // Layer policies can set duration and easing independently of the shared
    // named animation. Legacy string rules preserve the existing settings.
    if (duration !== undefined) spec.duration = duration;
    else if (custom?.duration !== undefined) spec.duration = custom.duration;
    if (easing !== undefined) spec.ease = easing;
    else if (custom?.ease !== undefined) spec.ease = custom.ease;
    else if (custom?.easing !== undefined) spec.ease = custom.easing;
    if (!opening) {
        spec.from = { ...spec.from, ...current };
        wm._mapWindowDone(shellwm, actor);
    }

    const pending = opening ? wm._mapping : wm._destroying;
    const enabled =
        animation !== "none" &&
        spec.duration > 0 &&
        St.Settings.get().enable_animations &&
        wm._shouldAnimateActor(actor, [Meta.WindowType.DOCK, Meta.WindowType.DESKTOP]);

    let controller = null;
    const complete = (finished = true) => {
        if (controllers.get(actor) === controller) controllers.delete(actor);
        if (opening && !finished) {
            interrupted.set(actor, {
                x: actor.translation_x,
                y: actor.translation_y,
                opacity: (actor.opacity ?? 255) / 255,
            });
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                interrupted.delete(actor);
                return GLib.SOURCE_REMOVE;
            });
        }
        if (opening) wm._mapWindowDone(shellwm, actor);
        else wm._destroyWindowDone(shellwm, actor);
    };

    pending.add(actor);
    if (!enabled) {
        if (opening) actor.show();
        complete(true);
        return true;
    }

    actor.show();
    controller = run(actor, spec, { onComplete: complete });
    if (!controller.finished) controllers.set(actor, controller);
    return true;
}

export function cancel(actor) {
    const controller = controllers.get(actor);
    if (controller && !controller.finished) controller.cancel();
}
