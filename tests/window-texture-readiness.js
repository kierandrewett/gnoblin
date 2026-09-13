// Run with the installed Gnoblin GI paths; no display is required.
import Meta from "gi://Meta";
import { WindowBorders, WindowCorners } from "../src/gnome-shell-overlay/js/ui/components/gnoblinCorners.js";
import { defaults, borderDefaults } from "../src/gnome-shell-overlay/js/ui/components/gnoblinCornerGeometry.js";

// A mapped window can have its final geometry before its first buffer arrives.
// This is the state captured in the Spotify login crash.
for (const [Effect, config] of [
    [WindowCorners, defaults],
    [WindowBorders, borderDefaults],
]) {
    let texture = null;
    let captures = 0;
    const rect = { x: 0, y: 0, width: 400, height: 300 };
    const surface = { width: 400, height: 300, get_effects: () => [] };
    const actor = {
        mapped: true,
        opacity: 255,
        meta_window: {
            get_frame_rect: () => rect,
            get_buffer_rect: () => rect,
            get_window_type: () => Meta.WindowType.NORMAL,
            is_override_redirect: () => false,
            get_tile_match: () => null,
        },
        get_resource_scale: () => 1,
        get_texture: () => ({ get_texture: () => texture, is_opaque: () => false }),
        get_effects: () => [],
        get_children: () => [],
        get_image: () => {
            captures++;
            return null;
        },
    };
    const instance = { actor, surface, toolkit: {}, remove() {} };
    Effect.prototype.update.call(instance, config);
    if (captures !== 0) throw new Error(`${Effect.name}: captured a mapped window without a buffer`);
    texture = {};
    actor.opacity = 128;
    Effect.prototype.update.call(instance, config);
    if (captures !== 0) throw new Error(`${Effect.name}: sampled the opening fade`);
    actor.opacity = 255;
    Effect.prototype.update.call(instance, config);
    if (captures !== 1) throw new Error(`${Effect.name}: did not retry when the buffer arrived`);
    texture = null;
    Effect.prototype.update.call(instance, config);
    if (captures !== 1) throw new Error(`${Effect.name}: captured after the buffer was removed`);
}
print("PASS: corners and borders defer image capture until a window buffer exists");
