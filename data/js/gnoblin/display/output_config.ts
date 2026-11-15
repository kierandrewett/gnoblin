// `OutputConfig` and `outputRequest`: what an output is told to do, checked and written as the request lines the
// compositor's `output apply` takes.

/** The transforms an output takes: the compositor's names for a rotation and a flip. */
export type OutputTransform =
    "normal" | "90" | "180" | "270" | "flipped" | "flipped-90" | "flipped-180" | "flipped-270";

/**
 * What an output is told to do, as the argument of `Output.apply` and the settings of an `OutputProfile`. A key left
 * out keeps the output's value. The compositor applies it through its transactional engine and keeps it as the
 * output's profile, so the same monitor on the same port gets it back after an unplug. `primary` is not a setting: the
 * layout derives the primary output, and a request that names it throws.
 */
export interface OutputConfig {
    /** `"3440x1440@144"`, or `"preferred"` to keep what the backend drives. */
    mode?: string;
    /** From 0.5 to 4. */
    scale?: number;
    /**
     * Owner-requested addition beyond the display specification. The output is composed at this multiple of its mode
     * (`mode * supersample` pixels, clients told `scale * supersample`) and shown at its mode, so the desktop keeps its
     * logical size and looks anti-aliased, and screenshots come out hi-res. A number from 1 to 16; 1, 2 and 4 are the
     * supported ones, any other is accepted when `mode * supersample` is a whole size, with one notice.
     */
    supersample?: number;
    transform?: OutputTransform;
    /** The output's top-left corner in the one desktop coordinate space, `[x, y]`. Left out, it follows the row. */
    position?: readonly [number, number] | { readonly x: number; readonly y: number };
    /** `false` switches the output off: it shows black and has no place in the desktop. */
    enabled?: boolean;
    /** The connector this output shows an exact copy of, letterboxed to fit its own panel. */
    mirrorOf?: string;
}

/** The keys `outputRequest` accepts, for the error text of an unknown one. */
const OUTPUT_KEYS: readonly string[] = ["mode", "scale", "supersample", "transform", "position", "enabled", "mirrorOf"];

/** The scale range the compositor accepts for an output. */
const MIN_OUTPUT_SCALE = 0.5;
const MAX_OUTPUT_SCALE = 4;

/** The supersample range the compositor accepts for an output. */
const MIN_SUPERSAMPLE = 1;
const MAX_SUPERSAMPLE = 16;

/** The factors the compositor recommends: whole powers of two give an exact box average. */
const RECOMMENDED_SUPERSAMPLE: readonly number[] = [1, 2, 4];

/**
 * The notice a supersample factor earns when it is not a recommended one, or null. The text is the compositor's own
 * (`gnoblin_output::supersample::notice`), so the log line and the script's diagnostic say the same thing.
 */
export function supersampleNotice(factor: number): string | null {
    return RECOMMENDED_SUPERSAMPLE.includes(factor) ? null : `supersample ${factor} is not recommended: use 1, 2 or 4`;
}

/** The same names as `OutputTransform`, as a list, to check a value at run time. */
const TRANSFORMS: readonly string[] = [
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];

/**
 * The compositor's request text for `setting` (one `key=value` per line), checking each value and naming `path` and the
 * key in the error. `mirrorOf` is already a connector here; `Output.mirror` turns an `Output` into one.
 */
export function outputRequest(
    setting: Omit<OutputConfig, "mirrorOf"> & { mirrorOf?: string | null },
    path: string,
): string {
    if (setting === null || typeof setting !== "object") {
        throw new TypeError(`${path} must be an object with ${OUTPUT_KEYS.join(", ")}`);
    }
    const lines: string[] = [];
    for (const key of Object.keys(setting)) {
        const value = (setting as Record<string, unknown>)[key];
        const where = `${path}.${key}`;
        switch (key) {
            case "mode":
                if (typeof value !== "string" || !(value === "preferred" || /^\d+x\d+@\d+(\.\d+)?$/.test(value))) {
                    throw new TypeError(
                        `${where} must be "preferred" or "WIDTHxHEIGHT@HERTZ", such as "3440x1440@144"`,
                    );
                }
                lines.push(`mode=${value}`);
                break;
            case "scale":
                if (
                    typeof value !== "number" ||
                    !Number.isFinite(value) ||
                    value < MIN_OUTPUT_SCALE ||
                    value > MAX_OUTPUT_SCALE
                ) {
                    throw new RangeError(`${where} must be a number from ${MIN_OUTPUT_SCALE} to ${MAX_OUTPUT_SCALE}`);
                }
                lines.push(`scale=${value}`);
                break;
            case "supersample":
                if (
                    typeof value !== "number" ||
                    !Number.isFinite(value) ||
                    value < MIN_SUPERSAMPLE ||
                    value > MAX_SUPERSAMPLE
                ) {
                    throw new RangeError(
                        `${where} must be a number from ${MIN_SUPERSAMPLE} to ${MAX_SUPERSAMPLE}, such as 1, 2 or 4`,
                    );
                }
                lines.push(`supersample=${value}`);
                break;
            case "transform":
                if (typeof value !== "string" || !TRANSFORMS.includes(value)) {
                    throw new TypeError(`${where} must be one of ${TRANSFORMS.join(", ")}`);
                }
                lines.push(`transform=${value}`);
                break;
            case "position": {
                const [x, y] = Array.isArray(value)
                    ? (value as unknown[])
                    : value !== null && typeof value === "object"
                      ? [(value as { x?: unknown }).x, (value as { y?: unknown }).y]
                      : [];
                if (!Number.isInteger(x) || !Number.isInteger(y)) {
                    throw new TypeError(`${where} must be [x, y] or { x, y } with whole numbers`);
                }
                lines.push(`position=${String(x)},${String(y)}`);
                break;
            }
            case "enabled":
                if (typeof value !== "boolean") {
                    throw new TypeError(`${where} must be true or false`);
                }
                lines.push(`enabled=${String(value)}`);
                break;
            case "mirrorOf":
                if (value !== null && (typeof value !== "string" || value === "")) {
                    throw new TypeError(`${where} must be a connector name such as "eDP-1", or null for no mirror`);
                }
                lines.push(`mirror_of=${value ?? ""}`);
                break;
            case "primary":
                // Refused by name instead of left to the generic message: it is an output property, and a script that
                // wrote it would otherwise read the "unknown key" text as a typo.
                throw new TypeError(`${where} cannot be set: the layout derives the primary output`);
            default:
                throw new TypeError(`${where} is not an output setting; expected one of ${OUTPUT_KEYS.join(", ")}`);
        }
    }
    return lines.join("\n");
}

/** What `outputRequest` takes: the keys of one output's request. */
export type OutputSetting = Parameters<typeof outputRequest>[0];
