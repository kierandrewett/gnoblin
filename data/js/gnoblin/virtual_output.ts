// Virtual output descriptions for scripts that request additional compositor outputs.
// A viewer-attached output follows the headset; see docs/xr.md for its purpose.

/** Where a virtual output sits. `viewer` keeps it in front of the headset, so it follows the head. */
export type VirtualOutputAttachment = "viewer";

/** What a virtual output is: a name, a size in pixels, and optionally where it is attached. */
export interface VirtualOutputOptions {
    /** The connector name the output gets, such as `XR-HUD`. A layer-shell client picks an output by it. */
    name: string;
    /** The size in pixels. Whole numbers above zero. */
    width: number;
    /** Output height in whole pixels. */
    height: number;
    /** The scale of the output's pixels. Default 1. */
    scale?: number;
    /** `viewer` attaches the output to the headset. Without it the output has no place in the room. */
    attachment?: VirtualOutputAttachment;
}

/**
 * A description of an output with no display behind it. Adding it to `gnoblin.outputs` currently throws a
 * `GnoblinError` with code `unsupported`.
 */
export class VirtualOutput {
    /** Connector name assigned to the output. */
    readonly name: string;
    /** Output width in whole pixels. */
    readonly width: number;
    /** Output height in whole pixels. */
    readonly height: number;
    /** Pixel scale, defaulting to 1. */
    readonly scale: number;
    /** Attachment point, or `null` when unattached. */
    readonly attachment: VirtualOutputAttachment | null;

    /**
     * Throws a `TypeError` for a name that is not text or an attachment that is not `viewer`, and a `RangeError` for
     * a size or scale that is not above zero.
     */
    constructor(options: VirtualOutputOptions) {
        if (typeof options?.name !== "string" || options.name === "") {
            throw new TypeError("VirtualOutput: name must be a non-empty string");
        }
        for (const side of ["width", "height"] as const) {
            if (!Number.isInteger(options[side]) || options[side] < 1) {
                throw new RangeError(`VirtualOutput: ${side} must be a whole number of pixels above 0`);
            }
        }
        const scale = options.scale ?? 1;
        if (typeof scale !== "number" || !Number.isFinite(scale) || scale <= 0) {
            throw new RangeError("VirtualOutput: scale must be a number above 0");
        }
        if (options.attachment !== undefined && options.attachment !== "viewer") {
            throw new TypeError(`VirtualOutput: attachment "${String(options.attachment)}" must be "viewer"`);
        }
        this.name = options.name;
        this.width = options.width;
        this.height = options.height;
        this.scale = scale;
        this.attachment = options.attachment ?? null;
    }
}
