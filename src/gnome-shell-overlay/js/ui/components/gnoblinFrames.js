// Configuration adapter only. Rendering and input live in native code.
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import { tuple } from "./gnoblinFramePolicy.js";

export class WindowFrame {
    constructor(actor, refresh) {
        this.actor = actor;
        this.window = actor.meta_window;
        this.signal = this.window.connect("gnoblin-frame-changed", refresh);
    }
    update(config, corners) {
        const window = this.actor.meta_window;
        const policy = tuple(config);
        const key = JSON.stringify(policy);
        if (key !== this.policy) {
            Meta.gnoblin_window_frame_set(window, new GLib.Variant("(iiiiiiiii)", policy));
            this.policy = key;
        }
        const options = {};
        for (const name of ["renderer", "style", "background", "foreground", "inactive-background"])
            options[name] = new GLib.Variant("s", config[name]);
        options["button-layout"] = new GLib.Variant("as", config["button-layout"]);
        const rounded = corners.mode !== "off" && (!window.is_maximized() || corners["keep-maximized"]);
        options.radius = new GLib.Variant("d", rounded ? corners.radius : 0);
        options.exponent = new GLib.Variant("d", 2 + corners.smoothing * 4);
        Meta.gnoblin_window_frame_style(window, new GLib.Variant("a{sv}", options));
        this.actor._gnoblinFrameLayout = Meta.gnoblin_window_frame_get(window).recursiveUnpack();
    }
    destroy() {
        this.window.disconnect(this.signal);
        this.window = null;
        delete this.actor._gnoblinFrameLayout;
    }
}
