// Shared animation definitions and sampler for window, layer-surface, and
// preview animations. All animated position values are actor translations;
// this module never changes the underlying window geometry.
import Clutter from "gi://Clutter";

const GNOME = {
    minimize: { duration: 400, ease: "ease-out-expo" },
    restore: { duration: 400, ease: "ease-out-expo" },
    open: { duration: 150, ease: "ease-out-expo" },
    close: { duration: 150, ease: "ease-out-quad" },
    "dialog-open": { duration: 100, ease: "ease-out-quad" },
    "dialog-close": { duration: 100, ease: "ease-out-quad" },
    "workspace-switch": { duration: 250, ease: "ease-out-cubic" },
    resize: { duration: 250, ease: "ease-out-quad" },
    "tile-preview-open": { duration: 250, ease: "ease-out-quad" },
    "tile-preview-close": { duration: 250, ease: "ease-out-quad" },
    "dialog-dim": { duration: 500, ease: "ease-out-quad" },
    "dialog-undim": { duration: 250, ease: "ease-out-quad" },
    "layer-companion-close": { duration: 180, ease: "ease-in-quad" },
};
const LAYER_DEFAULTS = {
    "layer-open": { duration: 250, ease: "ease-out-cubic" },
    "layer-close": { duration: 250, ease: "ease-out-cubic" },
};
const DEFAULTS = { ...GNOME, ...LAYER_DEFAULTS };
const PRESET_NAMES = [
    "gnome-minimize",
    "gnome-restore",
    "gnome-open",
    "gnome-close",
    "gnome-dialog-open",
    "gnome-dialog-close",
    "gnome-workspace-switch",
    "gnoblin-layer-open",
    "gnoblin-layer-close",
    "gnome",
    "zoom",
    "fade",
    "slide",
    "none",
    "gnoblin-console-open",
    "gnoblin-console-close",
    "gnoblin-shadow-change",
    "gnome-resize",
    "gnome-tile-preview-open",
    "gnome-tile-preview-close",
    "gnome-dialog-dim",
    "gnome-dialog-undim",
    "gnoblin-layer-companion-close",
];

/** List shipped animation names with the lifecycle event each one serves. */
export function presets() {
    return PRESET_NAMES.map((name) => ({
        name,
        event: name.startsWith("gnome-")
            ? name.slice(6)
            : name === "gnoblin-layer-open"
              ? "layer-open"
              : name === "gnoblin-layer-close"
                ? "layer-close"
                : name === "gnoblin-console-open"
                  ? "console-open"
                  : name === "gnoblin-console-close"
                    ? "console-close"
                    : name === "gnoblin-shadow-change"
                      ? "shadow-change"
                      : name === "gnoblin-layer-companion-close"
                        ? "layer-companion-close"
                        : null,
        builtin: true,
    }));
}

const PIVOTS = {
    center: [0.5, 0.5],
    "bottom-center": [0.5, 1],
    "top-center": [0.5, 0],
    "top-left": [0, 0],
    "top-right": [1, 0],
    "bottom-left": [0, 1],
    "bottom-right": [1, 1],
};
const activeRuns = new WeakMap();

function actorSnapshot(actor) {
    const pivot = actor.get_pivot_point?.() ?? [0, 0];
    return {
        x: actor.translation_x ?? 0,
        y: actor.translation_y ?? 0,
        scale_x: actor.scale_x ?? 1,
        scale_y: actor.scale_y ?? 1,
        width: actor.width,
        height: actor.height,
        rotation: actor.rotation_angle_z ?? actor.rotation_angle ?? 0,
        opacity: (actor.opacity ?? 255) / 255,
        pivot_x: pivot[0],
        pivot_y: pivot[1],
    };
}

function normalizedFrame(frame = {}) {
    const result = { ...frame };
    if (result["scale-x"] !== undefined) {
        result.scale_x ??= result["scale-x"];
        delete result["scale-x"];
    }
    if (result["scale-y"] !== undefined) {
        result.scale_y ??= result["scale-y"];
        delete result["scale-y"];
    }
    if (result.scale !== undefined) {
        result.scale_x ??= result.scale;
        result.scale_y ??= result.scale;
        delete result.scale;
    }
    if (result.rotation !== undefined) result.rotation = Number(result.rotation);
    if (result.opacity !== undefined) result.opacity = Number(result.opacity);
    return result;
}

function targetGeometry(context, { preferIcon = false } = {}) {
    const target = context.targetGeom;
    if (!preferIcon && Array.isArray(target)) {
        if (!target[0]) return null;
        return target[1];
    }
    if (!preferIcon && target && target.x !== undefined) return target;
    const window = context.window;
    if (window?.get_icon_geometry) {
        const [success, rect] = window.get_icon_geometry();
        if (success) return rect;
    }
    return null;
}

function bounds(context) {
    const actor = context.actor;
    const window = context.window ?? actor?.meta_window;
    const rect = actor && { x: actor.x, y: actor.y, width: actor.width, height: actor.height };
    const monitor = context.monitor;
    return { actor, window, rect, monitor };
}

function gnomeSpec(name, event, context) {
    const { actor, window, rect, monitor } = bounds(context);
    const defaults = DEFAULTS[event] ?? GNOME.open;
    let from = {},
        to = {},
        origin = "center",
        target = "none";
    const dialog = event.startsWith("dialog-");

    if (event === "minimize" || event === "restore") {
        const geom =
            name === "zoom"
                ? targetGeometry(context)
                : ["gnome-minimize", "gnome-restore", "gnome"].includes(name)
                  ? targetGeometry(context, { preferIcon: true })
                  : null;
        const monitorSized = window?.is_monitor_sized?.() ?? false;
        const useFade =
            name === "fade" || (monitorSized && ["gnome-minimize", "gnome-restore", "gnome"].includes(name));
        if (useFade) {
            from = event === "minimize" ? { opacity: 1 } : { opacity: 0 };
            to = event === "minimize" ? { opacity: 0 } : { opacity: 1 };
        } else {
            origin = "top-left";
            const actorX = actor?.x ?? rect?.x ?? 0;
            const actorY = actor?.y ?? rect?.y ?? 0;
            let destX, destY, sx, sy;
            if (geom && rect?.width && rect?.height) {
                destX = geom.x - actorX;
                destY = geom.y - actorY;
                sx = geom.width / rect.width;
                sy = geom.height / rect.height;
                target = name === "zoom" ? "dock" : "icon";
            } else if (monitor && rect?.width && rect?.height) {
                destX = monitor.x - actorX + (context.rtl ? monitor.width : 0);
                destY = monitor.y - actorY;
                sx = sy = 0;
                target = "monitor";
            } else {
                return {
                    name,
                    event,
                    duration: defaults.duration,
                    ease: defaults.ease,
                    from: {},
                    to: {},
                    origin,
                    target: "none",
                };
            }
            if (event === "minimize") {
                from = { x: 0, y: 0, scale_x: 1, scale_y: 1, opacity: 1 };
                to = { x: destX, y: destY, scale_x: sx, scale_y: sy, opacity: 0 };
            } else {
                from = { x: destX, y: destY, scale_x: sx, scale_y: sy, opacity: 1 };
                to = { x: 0, y: 0, scale_x: 1, scale_y: 1, opacity: 1 };
            }
        }
    } else if (event === "open") {
        origin = "bottom-center";
        from = { scale_x: 0.01, scale_y: 0.05, opacity: 0 };
        to = { scale_x: 1, scale_y: 1, opacity: 1 };
    } else if (event === "close") {
        from = { scale_x: 1, scale_y: 1, opacity: 1 };
        to = { scale_x: 0.8, scale_y: 0.8, opacity: 0 };
    } else if (event === "dialog-open") {
        from = { scale_y: 0, opacity: 0 };
        to = { scale_x: 1, scale_y: 1, opacity: 1 };
    } else if (event === "dialog-close") {
        from = { scale_y: 1 };
        to = { scale_y: 0 };
    } else if (event === "layer-open" || event === "layer-close") {
        const offset = context.offset ?? [0, 0];
        const opening = event === "layer-open";
        const fade = offset[0] === 0 && offset[1] === 0;
        from = opening ? { x: offset[0], y: offset[1], opacity: fade ? 0 : 1 } : { x: 0, y: 0, opacity: 1 };
        to = opening ? { x: 0, y: 0, opacity: 1 } : { x: offset[0], y: offset[1], opacity: fade ? 0 : 1 };
        target = "monitor";
    } else if (event === "console-open" || event === "console-close") {
        const opening = event === "console-open";
        const height = context.height ?? actor?.height ?? 0;
        from = { y: opening ? -height : 0 };
        to = { y: opening ? 0 : -height };
        origin = "top-left";
    } else if (event === "shadow-change") {
        return {
            name,
            event,
            duration: context.duration ?? 0,
            ease: context.easing ?? "ease-out-cubic",
            from: { progress: 0 },
            to: { progress: 1 },
            origin,
            target: "shadow",
        };
    } else if (event === "layer-companion-close") {
        const offset = context.offset ?? 0;
        from = { y: context.fromY ?? 0 };
        to = { y: context.toY ?? from.y + offset };
    } else if (event === "dialog-dim" || event === "dialog-undim") {
        const undim = event === "dialog-undim";
        from = { progress: undim ? 1 : 0 };
        to = { progress: undim ? 0 : 1 };
    } else if (event === "tile-preview-open" || event === "tile-preview-close" || event === "resize") {
        from = context.from ?? {};
        to = context.to ?? {};
    }
    return { name, event, duration: defaults.duration, ease: defaults.ease, from, to, origin, target };
}

/** Resolve a preset or custom record into an immutable animation description. */
export function resolve(name, event, context = {}, custom = null) {
    event ??= custom?.event ?? (name?.startsWith("gnome-") ? name.slice(6) : undefined);
    event ??= "open";
    name ??= "gnome-" + event;
    if (name.startsWith("gnome-") || name === "gnome") {
        const spec = gnomeSpec(name, event, context);
        if (custom?.duration !== undefined) spec.duration = custom.duration;
        if (custom?.ease !== undefined) spec.ease = custom.ease;
        if (custom?.from !== undefined) spec.from = { ...spec.from, ...normalizedFrame(custom.from) };
        if (custom?.to !== undefined) spec.to = { ...spec.to, ...normalizedFrame(custom.to) };
        if (custom?.keyframes !== undefined)
            spec.keyframes = custom.keyframes.map((frame) => ({ ...normalizedFrame(frame), at: Number(frame.at) }));
        if (custom?.origin !== undefined) spec.origin = custom.origin;
        if (custom?.target !== undefined) spec.target = custom.target;
        return spec;
    }
    if (name === "none") return { name, event, duration: 0, ease: "linear", from: {}, to: {} };
    if (name === "gnoblin-layer-open" || name === "gnoblin-layer-close") {
        const spec = gnomeSpec(name, event, context);
        if (custom?.duration !== undefined) spec.duration = custom.duration;
        if (custom?.ease !== undefined) spec.ease = custom.ease;
        if (custom?.from !== undefined) spec.from = { ...spec.from, ...normalizedFrame(custom.from) };
        if (custom?.to !== undefined) spec.to = { ...spec.to, ...normalizedFrame(custom.to) };
        if (custom?.keyframes !== undefined)
            spec.keyframes = custom.keyframes.map((frame) => ({ ...normalizedFrame(frame), at: Number(frame.at) }));
        if (custom?.origin !== undefined) spec.origin = custom.origin;
        if (custom?.target !== undefined) spec.target = custom.target;
        return spec;
    }
    if (name === "gnoblin-console-open" || name === "gnoblin-console-close" || name === "gnoblin-shadow-change") {
        const spec = gnomeSpec(name, event, context);
        spec.duration = Math.max(
            0,
            Number(custom?.duration ?? context.duration ?? (name.includes("console") ? 140 : 0)),
        );
        spec.ease =
            custom?.ease ??
            custom?.easing ??
            context.easing ??
            (name.includes("console") ? "ease-out-quad" : "ease-out-cubic");
        if (custom?.from) spec.from = normalizedFrame(custom.from);
        if (custom?.to) spec.to = normalizedFrame(custom.to);
        if (custom?.keyframes)
            spec.keyframes = custom.keyframes.map((frame) => ({ ...normalizedFrame(frame), at: Number(frame.at) }));
        return spec;
    }
    if (name === "gnoblin-layer-companion-close") {
        const spec = gnomeSpec(name, event, context);
        spec.duration = custom?.duration ?? 180;
        spec.ease = custom?.ease ?? "ease-in-quad";
        if (custom?.from) spec.from = normalizedFrame(custom.from);
        if (custom?.to) spec.to = normalizedFrame(custom.to);
        return spec;
    }
    if (["zoom", "fade", "slide"].includes(name)) {
        const spec = gnomeSpec(name, event, context);
        if (name === "zoom" && event === "minimize") spec.target = "dock";
        if (name === "fade") {
            const fadeIn = ["restore", "open", "dialog-open", "layer-open"].includes(event);
            spec.from = fadeIn ? { opacity: 0 } : { opacity: 1 };
            spec.to = fadeIn ? { opacity: 1 } : { opacity: 0 };
        }
        return spec;
    }
    const record = custom ?? {};
    const preset =
        name === "gnoblin-console-open" || name === "gnoblin-console-close"
            ? { duration: 140, ease: "ease-out-quad" }
            : name === "gnoblin-shadow-change"
              ? { duration: context.duration ?? 0, ease: context.easing ?? "ease-out-cubic" }
              : (DEFAULTS[event] ?? GNOME.open);
    return {
        name,
        event,
        duration: Math.max(0, Number(record.duration ?? preset.duration)),
        ease: record.ease ?? record.easing ?? preset.ease,
        from: normalizedFrame(record.from ?? (event === "shadow-change" ? { progress: 0 } : {})),
        to: normalizedFrame(record.to ?? (event === "shadow-change" ? { progress: 1 } : {})),
        keyframes: Array.isArray(record.keyframes)
            ? record.keyframes.map((frame) => ({ ...normalizedFrame(frame), at: Number(frame.at) }))
            : undefined,
        origin: record.origin ?? "center",
        target: record.target ?? "none",
    };
}

function cubicBezier(t, x1, y1, x2, y2) {
    const curve = (u, a, b) => 3 * (1 - u) ** 2 * u * a + 3 * (1 - u) * u ** 2 * b + u ** 3;
    let lo = 0,
        hi = 1,
        u = t;
    for (let i = 0; i < 12; i++) {
        const x = curve(u, x1, x2);
        if (Math.abs(x - t) < 1e-5) break;
        if (x < t) lo = u;
        else hi = u;
        u = (lo + hi) / 2;
    }
    return curve(u, y1, y2);
}

export function easeProgress(ease, t) {
    t = Math.max(0, Math.min(1, t));
    if (typeof ease === "object" && ease?.type === "cubic-bezier")
        return cubicBezier(t, ease.x1, ease.y1, ease.x2, ease.y2);
    switch (ease) {
        case "linear":
            return t;
        case "ease-in-quad":
            return t * t;
        case "ease-out-quad":
            return 1 - (1 - t) ** 2;
        case "ease-in-out-cubic":
            return t < 0.5 ? 4 * t ** 3 : 1 - (-2 * t + 2) ** 3 / 2;
        case "ease-out-expo":
            return t === 1 ? 1 : 1 - 2 ** (-10 * t);
        case "ease-out-cubic":
            return 1 - (1 - t) ** 3;
        case "ease-in-cubic":
            return t ** 3;
        case "ease-out-back": {
            const c1 = 1.70158,
                c3 = c1 + 1;
            return 1 + c3 * (t - 1) ** 3 + c1 * (t - 1) ** 2;
        }
        default:
            return t;
    }
}

function lerp(a, b, t) {
    return a + (b - a) * t;
}

/** Sample the shared animation definition at linear timeline progress 0..1. */
export function sample(spec, progress) {
    const t = Math.max(0, Math.min(1, progress));
    const byTime = new Map([
        [0, { at: 0, ...spec.from }],
        [1, { at: 1, ...spec.to }],
    ]);
    for (const frame of spec.keyframes ?? []) {
        const at = Math.max(0, Math.min(1, Number(frame.at)));
        byTime.set(at, { ...byTime.get(at), ...frame, at });
    }
    const frames = [...byTime.values()];
    frames.sort((a, b) => a.at - b.at);
    const out = {};
    const keys = new Set(frames.flatMap((frame) => Object.keys(frame)));
    keys.delete("at");
    keys.delete("ease");
    for (const key of keys) {
        const points = frames.filter((frame) => frame[key] !== undefined);
        let left = points[0],
            right = points.at(-1);
        for (let i = 0; i < points.length - 1; i++) {
            if (t >= points[i].at && t <= points[i + 1].at) {
                left = points[i];
                right = points[i + 1];
                break;
            }
        }
        if (t <= left.at || left === right) {
            out[key] = left[key];
            continue;
        }
        if (t >= right.at) {
            out[key] = right[key];
            continue;
        }
        const span = right.at - left.at;
        const local = easeProgress(right.ease ?? spec.ease, span <= 0 ? 1 : (t - left.at) / span);
        out[key] =
            typeof left[key] === "number" && typeof right[key] === "number"
                ? lerp(left[key], right[key], local)
                : local < 1
                  ? left[key]
                  : right[key];
    }
    return out;
}

/** Run a sampled spec without an actor, for shader uniforms and scalar blends. */
export function runValues(spec, options = {}) {
    const timeline = Clutter.Timeline.new_for_actor(
        options.actor ?? global.stage,
        Math.max(1, Math.round(spec.duration)),
    );
    timeline.set_progress_mode(Clutter.AnimationMode.LINEAR);
    let elapsed = 0,
        done = false;
    const render = (progress) => {
        if (done) return;
        options.onFrame?.(sample(spec, progress), progress);
        if (progress >= 1) finish(true);
    };
    const finish = (finished) => {
        if (done) return;
        done = true;
        timeline.stop();
        options.onComplete?.(finished);
    };
    timeline.connect("new-frame", () => {
        elapsed = timeline.get_elapsed_time();
        render(spec.duration ? elapsed / spec.duration : 1);
    });
    timeline.connect("completed", () => render(1));
    const controller = {
        seek(progress) {
            elapsed = Math.max(0, Math.min(1, progress)) * spec.duration;
            timeline.advance(Math.round(elapsed));
            render(spec.duration ? elapsed / spec.duration : 1);
        },
        step(ms) {
            elapsed = Math.max(0, Math.min(spec.duration, elapsed + Number(ms)));
            timeline.advance(Math.round(elapsed));
            render(spec.duration ? elapsed / spec.duration : 1);
        },
        play() {
            if (!done) timeline.start();
        },
        pause() {
            if (!done) timeline.pause();
        },
        cancel() {
            finish(false);
        },
        get finished() {
            return done;
        },
    };
    render(0);
    if (spec.duration <= 0) render(1);
    else if (!options.paused) timeline.start();
    return controller;
}

function apply(actor, values) {
    if (values.x !== undefined) actor.translation_x = values.x;
    if (values.y !== undefined) actor.translation_y = values.y;
    if (values.scale_x !== undefined) actor.scale_x = values.scale_x;
    if (values.scale_y !== undefined) actor.scale_y = values.scale_y;
    if (values.width !== undefined) actor.width = values.width;
    if (values.height !== undefined) actor.height = values.height;
    if (values.rotation !== undefined) {
        if ("rotation_angle_z" in actor) actor.rotation_angle_z = values.rotation;
        else actor.rotation_angle = values.rotation;
    }
    if (values.opacity !== undefined) actor.opacity = Math.max(0, Math.min(255, Math.round(values.opacity * 255)));
    if (values.pivot_x !== undefined || values.pivot_y !== undefined)
        actor.set_pivot_point?.(values.pivot_x ?? 0, values.pivot_y ?? 0);
}

/** Run the exact sampler used by seek/step previews. */
export function run(actor, spec, options = {}) {
    activeRuns.get(actor)?.cancel({ restore: true });
    const initial = actorSnapshot(actor);
    const pivot = Array.isArray(spec.origin) ? spec.origin : (PIVOTS[spec.origin] ?? PIVOTS.center);
    actor.set_pivot_point?.(pivot[0], pivot[1]);
    const from = { ...initial, ...spec.from };
    if (spec.from?.pivot_x === undefined) delete from.pivot_x;
    if (spec.from?.pivot_y === undefined) delete from.pivot_y;
    const frameKeys = new Set([
        ...Object.keys(spec.from ?? {}),
        ...Object.keys(spec.to ?? {}),
        ...(spec.keyframes ?? []).flatMap((frame) => Object.keys(frame)),
    ]);
    if (!frameKeys.has("width")) delete from.width;
    if (!frameKeys.has("height")) delete from.height;
    const normalized = { ...spec, from, to: { ...from, ...spec.to } };
    const timeline = Clutter.Timeline.new_for_actor(actor, Math.max(1, Math.round(spec.duration)));
    timeline.set_progress_mode(Clutter.AnimationMode.LINEAR);
    let elapsed = 0,
        done = false,
        manualAdvance = false;

    const render = (progress, finishAtEnd = true) => {
        if (done) return;
        const values = sample(normalized, progress);
        apply(actor, values);
        options.onFrame?.(values, progress);
        if (progress >= 1 && finishAtEnd) finish(true);
    };
    const finish = (finished) => {
        if (done) return;
        done = true;
        timeline.stop();
        if (activeRuns.get(actor) === controller) activeRuns.delete(actor);
        options.onComplete?.(finished);
    };
    timeline.connect("new-frame", () => {
        elapsed = timeline.get_elapsed_time();
        render(spec.duration ? elapsed / spec.duration : 1, !manualAdvance);
    });
    timeline.connect("completed", () => render(1, !manualAdvance));
    const controller = {
        seek(progress) {
            elapsed = Math.max(0, Math.min(1, progress)) * spec.duration;
            const requested = elapsed;
            manualAdvance = true;
            timeline.advance(Math.round(Math.min(requested, Math.max(0, spec.duration - 1))));
            elapsed = requested;
            render(spec.duration ? requested / spec.duration : 1, false);
            manualAdvance = false;
        },
        step(ms) {
            elapsed = Math.max(0, Math.min(spec.duration, elapsed + Number(ms)));
            const requested = elapsed;
            manualAdvance = true;
            timeline.advance(Math.round(Math.min(requested, Math.max(0, spec.duration - 1))));
            elapsed = requested;
            render(spec.duration ? requested / spec.duration : 1, false);
            manualAdvance = false;
        },
        play() {
            if (!done) timeline.start();
        },
        pause() {
            if (!done) timeline.pause();
        },
        cancel({ restore = false } = {}) {
            if (restore) {
                apply(actor, initial);
                actor.set_pivot_point?.(initial.pivot_x, initial.pivot_y);
            }
            finish(false);
        },
        get finished() {
            return done;
        },
    };
    activeRuns.set(actor, controller);
    // Avoid an implicit first-frame delay; paused previews start at progress 0.
    render(0);
    if (spec.duration <= 0) render(1);
    else if (!options.paused) timeline.start();
    return controller;
}

// Clutter's enum names are exported for integrations that need to bridge into
// existing APIs which accept AnimationMode values.
export const animationModes = {
    "ease-out-expo": Clutter.AnimationMode.EASE_OUT_EXPO,
    "ease-out-quad": Clutter.AnimationMode.EASE_OUT_QUAD,
    "ease-out-cubic": Clutter.AnimationMode.EASE_OUT_CUBIC,
    "ease-in-out-cubic": Clutter.AnimationMode.EASE_IN_OUT_CUBIC,
    linear: Clutter.AnimationMode.LINEAR,
};
