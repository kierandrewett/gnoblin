// `Display`, the logical display that presents one or more outputs, with the checked state the manager reads through
// `displayState`.

import type { DisplayInit, DisplayModeName, DisplayOutputCollection, OutputView } from "gnoblin-std/display/types";
import { nextDisplayNumber, shell } from "gnoblin-std/display/state";
import { OutputProfile } from "gnoblin-std/display/output_profile";
import { Viewport, viewportState } from "gnoblin-std/display/viewport";

// ---- Display ------------------------------------------------------------------------------------------------------

/** The manager's checked state for one display. */
export interface DisplayState {
    /** Stable identifier used in collection errors and `toString()`. */
    id: string;
    /** Presentation mode: one output, mirrored outputs, or a shared span. */
    mode: DisplayModeName;
    /** Profile or output selectors, in the order resolved by the manager. */
    selectors: readonly (OutputProfile | OutputView)[];
    /** Viewport associated with this display. */
    viewport: Viewport;
    /** A display the shell made for an output nobody else presents. It is in `gnoblin.displays` until one is. */
    implicit: boolean;
    /** The connected outputs the display presents now, in selector order. */
    live: OutputView[];
    /** The workspace assigned while the display could not show it (parked, or not in the registry yet). */
    wanted: object | null;
    /** The mirrored outputs this display made, and what each mirrors. */
    mirrored: Map<OutputView, OutputView>;
    /** The output this display spans from, while it does. */
    spanLead: OutputView | null;
    /** The `Collection<Output>` of `live`, once someone asked for it. */
    outputs: { collection: DisplayOutputCollection; set(outputs: readonly OutputView[]): void } | null;
}

const displays = new WeakMap<Display, DisplayState>();

/** The state of a display, for the manager. */
export function displayState(display: Display): DisplayState {
    const state = displays.get(display);
    if (state === undefined) {
        throw new TypeError("not a Display");
    }
    return state;
}

const DISPLAY_MODES: readonly string[] = ["single", "mirror", "span"];

function checkMode(mode: unknown, count: number, target: string): DisplayModeName {
    if (typeof mode !== "string" || !DISPLAY_MODES.includes(mode)) {
        throw new TypeError(`${target}.mode: must be "single", "mirror" or "span"`);
    }
    if (mode === "single" && count > 1) {
        throw new TypeError(`${target}.mode: "single" presents one output; use "mirror" or "span" for ${count}`);
    }
    return mode as DisplayModeName;
}

/**
 * A logical presentation: one viewport shown on one output (`single`), copied to several (`mirror`) or stretched over
 * their union (`span`). An output belongs to one display at a time: `gnoblin.displays.add` refuses a display that
 * shares an output with another and names both. With no matching output connected the display is parked and its
 * workspace stays intact.
 *
 * The shell makes a `single` display for every output no script display presents, so `gnoblin.displays` always lists
 * what each monitor shows.
 */
export class Display {
    /**
     * @throws TypeError when `outputs` is empty or holds something else than profiles and outputs, `mode` is unknown or
     *   `viewport` already belongs to another display
     */
    constructor(init: DisplayInit) {
        if (init === null || typeof init !== "object") {
            throw new TypeError("Display: needs an object with outputs and a viewport");
        }
        const { id, outputs, mode, viewport, ...unknown } = init;
        const extra = Object.keys(unknown);
        if (extra.length > 0) {
            throw new TypeError(`Display.${extra[0]}: not a property of a display`);
        }
        if (!Array.isArray(outputs) || outputs.length === 0) {
            throw new TypeError("Display.outputs: must list at least one OutputProfile or Output");
        }
        for (const selector of outputs) {
            if (!(selector instanceof OutputProfile) && !shell().isOutput(selector)) {
                throw new TypeError("Display.outputs: each entry must be an OutputProfile or an Output");
            }
        }
        if (id !== undefined && (typeof id !== "string" || id === "")) {
            throw new TypeError("Display.id: must be a non-empty string");
        }
        if (viewport !== undefined && !(viewport instanceof Viewport)) {
            throw new TypeError("Display.viewport: must be a Viewport");
        }
        const own = viewport ?? new Viewport();
        const owner = viewportState(own).display;
        if (owner !== null) {
            throw new TypeError(`Display.viewport: already shown by display "${owner.id}"`);
        }
        displays.set(this, {
            id: id ?? `display-${nextDisplayNumber()}`,
            mode: checkMode(mode ?? "single", outputs.length, "Display"),
            selectors: Object.freeze([...outputs]),
            viewport: own,
            implicit: false,
            live: [],
            wanted: viewportState(own).content,
            mirrored: new Map(),
            spanLead: null,
            outputs: null,
        });
        viewportState(own).display = this;
    }

    /** Stable display name. */
    get id(): string {
        return displayState(this).id;
    }

    /** The display mode; changing it takes effect on the next reconciliation. */
    get mode(): DisplayModeName {
        return displayState(this).mode;
    }
    /** Sets the mode used on the next reconciliation. */
    set mode(value: DisplayModeName) {
        const state = displayState(this);
        state.mode = checkMode(value, state.selectors.length, `Display "${state.id}"`);
        shell().changed();
    }

    /**
     * The connected outputs the display presents now: a live `Collection<Output>`, empty while the display is
     * parked.
     */
    get outputs(): DisplayOutputCollection {
        return shell().outputs(this);
    }

    /** The viewport shown by this display. */
    get viewport(): Viewport {
        return displayState(this).viewport;
    }

    /** Formats the display as `Display#id`. */
    toString(): string {
        return `Display#${displayState(this).id}`;
    }
}

/** Makes the display the shell keeps for an output no script display presents. Used by the manager only. */
export function implicitDisplay(output: OutputView, id: string): Display {
    const display = new Display({ id, outputs: [output] });
    displayState(display).implicit = true;
    return display;
}
