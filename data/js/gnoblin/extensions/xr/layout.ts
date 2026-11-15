// Panel arrangement and default bindings for an XR room.

import { BindingCollection } from "gnoblin/bindings/binding";
import { arrangeRing, RING_DEFAULTS } from "gnoblin/extensions/xr/ring";
import type { RingResult, RingView, XRRingParams } from "gnoblin/extensions/xr/ring";
import {
    parseControllerTrigger,
    ViewMoveBinding,
    ViewResizeBinding,
    ViewRotateBinding,
} from "gnoblin/extensions/xr/bindings";

/** What `new SpatialLayout` takes; every distance is in metres. */
export interface SpatialLayoutOptions {
    /** The distance from the viewer to the centre of a panel, in metres. Default 1.6. */
    radius?: number;
    /** The height of a panel's centre above the floor, in metres. Default 1.4. */
    eye?: number;
    /** The angle between neighbouring panels, in radians. Default 3 degrees. */
    spacing?: number;
    /** The metres of panel for one pixel of window. Default 0.00075. */
    metresPerPixel?: number;
    /** The focused panel is this much larger. Default 1.15. */
    focusScale?: number;
    /** ... and this many metres closer. Default 0.1. */
    focusPull?: number;
    /** The most the panels reach to either side of the front, in radians. Default 150 degrees. */
    halfArc?: number;
    /** Panels on the ring; the rest stack behind it. Default 12. */
    maxPanels?: number;
    /** Each tier of stacked panels is this many metres further back. Default 0.25. */
    stackStep?: number;
    /** The triggers of the stock bindings by id; `null` leaves a binding out. */
    triggers?: { move?: string | null; resize?: string | null; rotate?: string | null };
}

const LAYOUT_TRIGGERS = { move: "Grip+Drag", resize: "Grip+Trigger+Drag", rotate: "Trigger+Thumbstick+Drag" } as const;

/**
 * Places room panels on a ring around the viewer and provides bindings to move, resize and rotate them. `bindings`
 * is the layout's collection: remove a binding to turn it off, or add another binding in its place.
 */
export class SpatialLayout {
    /** The collection of bindings that move, resize and rotate room panels. */
    readonly bindings = new BindingCollection();
    /** Ring dimensions used to place panels, in millimetres and radians. */
    readonly ring: XRRingParams;

    constructor(options: SpatialLayoutOptions = {}) {
        const metre = 1000;
        const ring: XRRingParams = { ...RING_DEFAULTS };
        const set = (key: keyof XRRingParams, value: number | undefined, factor = 1): void => {
            if (value === undefined) return;
            if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
                throw new RangeError(`SpatialLayout: ${key} must be a number above 0, got ${String(value)}`);
            }
            ring[key] = value * factor;
        };
        set("radius", options.radius, metre);
        set("eye", options.eye, metre);
        set("gap", options.spacing);
        set("mmPerPx", options.metresPerPixel, metre);
        set("focusScale", options.focusScale);
        set("focusPull", options.focusPull, metre);
        set("halfArc", options.halfArc);
        set("maxPanels", options.maxPanels);
        set("stackStep", options.stackStep, metre);
        this.ring = ring;
        const triggers: Record<keyof typeof LAYOUT_TRIGGERS, string | null> = { ...LAYOUT_TRIGGERS };
        for (const name of Object.keys(options.triggers ?? {})) {
            if (!Object.hasOwn(LAYOUT_TRIGGERS, name)) {
                throw new TypeError(
                    `SpatialLayout: unknown trigger "${name}"; expected one of ${Object.keys(LAYOUT_TRIGGERS).join(", ")}`,
                );
            }
            const value = (options.triggers as Record<string, string | null | undefined>)[name];
            if (value === undefined) continue;
            if (value !== null) parseControllerTrigger(value, `SpatialLayout trigger "${name}"`);
            triggers[name as keyof typeof LAYOUT_TRIGGERS] = value;
        }
        if (triggers.move !== null) this.bindings.add(new ViewMoveBinding({ trigger: triggers.move }));
        if (triggers.resize !== null) this.bindings.add(new ViewResizeBinding({ trigger: triggers.resize }));
        if (triggers.rotate !== null) this.bindings.add(new ViewRotateBinding({ trigger: triggers.rotate }));
    }

    /** Places views around the ring and returns their poses and drawing order. */
    arrange<I>(views: readonly RingView<I>[], focus: I | null): RingResult {
        return arrangeRing(views, focus, this.ring);
    }
}
