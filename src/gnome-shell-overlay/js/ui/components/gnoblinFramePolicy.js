// Frame extents and crop are logical pixels, clockwise from the top.
export const defaults = Object.freeze({
    mode: "off",
    extents: [32, 1, 1, 1],
    crop: [0, 0, 0, 0],
    background: "#242424",
    foreground: "#eeeeee",
    "inactive-background": "#303030",
    "button-layout": ["minimize", "maximize", "close"],
    renderer: "native",
    style: "default",
});
const modes = ["off", "auto", "prefer-server", "replace"];
export function validateRenderers(services = {}) {
    if (!services || typeof services !== "object" || Array.isArray(services))
        throw new Error("frame-renderers must be a table of named argv arrays");
    for (const [name, argv] of Object.entries(services)) {
        if (
            name === "native" ||
            !/^[a-zA-Z0-9_-]{1,64}$/.test(name) ||
            !Array.isArray(argv) ||
            argv.length < 1 ||
            argv.length > 32 ||
            argv.some((arg) => typeof arg !== "string" || arg.includes("\0")) ||
            !argv[0].startsWith("/")
        )
            throw new Error(
                `frame-renderers.${name}: expected argv with an absolute executable path; native is reserved`,
            );
    }
}
export function validate(frame) {
    if (!frame || typeof frame !== "object" || Array.isArray(frame)) throw new Error("frame must be a table");
    for (const [key, value] of Object.entries(frame)) {
        if (!Object.hasOwn(defaults, key)) throw new Error(`Unknown frame property: ${key}`);
        if (key === "mode") {
            if (!modes.includes(value)) throw new Error("Invalid frame mode");
        } else if (key === "extents" || key === "crop") {
            if (
                !Array.isArray(value) ||
                value.length !== 4 ||
                value.some((n) => !Number.isInteger(n) || n < 0 || n > 256)
            )
                throw new Error(`frame.${key} needs four integers in 0..256`);
        } else if (key === "renderer" || key === "style") {
            if (typeof value !== "string" || !/^[a-zA-Z0-9_-]{1,64}$/.test(value))
                throw new Error(`frame.${key} must be a name, not a JavaScript path`);
        } else if (key === "button-layout") {
            if (
                !Array.isArray(value) ||
                value.some((v) => !["close", "maximize", "minimize"].includes(v)) ||
                new Set(value).size !== value.length
            )
                throw new Error("Invalid frame button-layout");
        } else if (typeof value !== "string" || !/^#[\da-f]{6}([\da-f]{2})?$/i.test(value)) {
            throw new Error(`frame.${key} needs a hex color`);
        }
    }
}
export function tuple(frame) {
    return [modes.indexOf(frame.mode), ...frame.crop, ...frame.extents];
}
