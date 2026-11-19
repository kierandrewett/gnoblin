// `Display`, the logical display that presents one or more outputs, with the checked state the manager reads through
// `displayState`.

import type { DisplayInit, DisplayModeName, DisplayOutputCollection, OutputView } from "gnoblin-std/display/types";
import { nextDisplayNumber, shell } from "gnoblin-std/display/state";
import { OutputProfile, profileState } from "gnoblin-std/display/output_profile";
import { Viewport, viewportState } from "gnoblin-std/display/viewport";
import { Display as NativeDisplay } from "gnoblin-std/generated/classes";
import { PresentKind, SelectKind } from "gnoblin-std/generated/constants";

// ---- Display ------------------------------------------------------------------------------------------------------

/**
 * What a display keeps in the script's own terms: its name, how it was asked to present, the selectors as written, its viewport and
 * the library's display, which reconciles them with the connected outputs (`libgnoblin-std`, `displays.rs`).
 */
export interface DisplayState {
    /** Stable identifier used in collection errors and `toString()`. */
    id: string;
    /** Presentation mode: one output, mirrored outputs, or a shared span. */
    mode: DisplayModeName;
    /** Profile or output selectors, as the script wrote them. */
    selectors: readonly (OutputProfile | OutputView)[];
    /** Viewport associated with this display. */
    viewport: Viewport;
    /** A display the library made for an output no display of the script presents. */
    implicit: boolean;
    /** The library's display. */
    native: NativeDisplay;
    /** The `Collection<Output>` of the outputs the display presents, once someone asked for it. */
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
        const name = id ?? `display-${nextDisplayNumber()}`;
        const kind = checkMode(mode ?? "single", outputs.length, "Display");
        const wanted = viewportState(own).content;
        const native = NativeDisplay.new({
            mode: PRESENT[kind],
            id: name,
            selectors: outputs.map((selector) =>
                selector instanceof OutputProfile
                    ? { kind: SelectKind.PROFILE, target: profileState(selector).native.handle }
                    : { kind: SelectKind.OUTPUT, target: shell().outputNode(selector) },
            ),
            wanted: wanted === null ? 0 : shell().workspaceHandle(wanted),
        });
        if (native === null) {
            throw new TypeError("Display: the library made no display");
        }
        adopt(this, {
            id: name,
            mode: kind,
            selectors: Object.freeze([...outputs]),
            viewport: own,
            implicit: false,
            native,
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
        state.native.setMode(PRESENT[state.mode]);
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

/** The library's presentation mode of a mode name. */
const PRESENT: Readonly<Record<DisplayModeName, number>> = {
    single: PresentKind.SINGLE,
    mirror: PresentKind.MIRROR,
    span: PresentKind.SPAN,
};

/** The name of a presentation mode the library reports. */
export function modeNameOf(mode: number): DisplayModeName {
    return mode === PresentKind.MIRROR ? "mirror" : mode === PresentKind.SPAN ? "span" : "single";
}

/** The displays by the library's handle. The library makes a display for every output no display of the script presents. */
const byHandle = new Map<number, WeakRef<Display>>();

function adopt(display: Display, state: DisplayState): void {
    displays.set(display, state);
    byHandle.set(state.native.handle, new WeakRef(display));
}

/** The display of the library's handle, if the script has an object for it. */
export function displayOfHandle(handle: number): Display | undefined {
    return byHandle.get(handle)?.deref();
}

/** Forgets a display the library dropped for good. */
export function forgetHandle(handle: number): void {
    byHandle.delete(handle);
}

/**
 * The object for a display the library made (the one it keeps for an output no display of the script presents): it presents
 * `output` on a viewport of its own.
 */
export function wrapDisplay(native: NativeDisplay, id: string, mode: DisplayModeName, output: OutputView): Display {
    const display = Object.create(Display.prototype) as Display;
    const viewport = new Viewport();
    adopt(display, {
        id,
        mode,
        selectors: Object.freeze([output]),
        viewport,
        implicit: true,
        native,
        outputs: null,
    });
    viewportState(viewport).display = display;
    return display;
}
