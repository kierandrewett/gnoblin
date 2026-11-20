import type { DisplayInit, DisplayModeName, DisplayOutputCollection, OutputView } from "gnoblin-std/display/types";
import { OutputProfile } from "gnoblin-std/display/output_profile";
import { Viewport } from "gnoblin-std/display/viewport";
import { Display as NativeDisplay } from "gnoblin-std/emitted/classes";
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
    outputs: {
        collection: DisplayOutputCollection;
        set(outputs: readonly OutputView[]): void;
    } | null;
}
/** The state of a display, for the manager. */
export declare function displayState(display: Display): DisplayState;
/**
 * A logical presentation: one viewport shown on one output (`single`), copied to several (`mirror`) or stretched over
 * their union (`span`). An output belongs to one display at a time: `gnoblin.displays.add` refuses a display that
 * shares an output with another and names both. With no matching output connected the display is parked and its
 * workspace stays intact.
 *
 * The shell makes a `single` display for every output no script display presents, so `gnoblin.displays` always lists
 * what each monitor shows.
 */
export declare class Display {
    /**
     * @throws TypeError when `outputs` is empty or holds something else than profiles and outputs, `mode` is unknown or
     *   `viewport` already belongs to another display
     */
    constructor(init: DisplayInit);
    /** Stable display name. */
    get id(): string;
    /** The display mode; changing it takes effect on the next reconciliation. */
    get mode(): DisplayModeName;
    /** Sets the mode used on the next reconciliation. */
    set mode(value: DisplayModeName);
    /**
     * The connected outputs the display presents now: a live `Collection<Output>`, empty while the display is
     * parked.
     */
    get outputs(): DisplayOutputCollection;
    /** The viewport shown by this display. */
    get viewport(): Viewport;
    /** Formats the display as `Display#id`. */
    toString(): string;
}
/** The name of a presentation mode the library reports. */
export declare function modeNameOf(mode: number): DisplayModeName;
/** The display of the library's handle, if the script has an object for it. */
export declare function displayOfHandle(handle: number): Display | undefined;
/** Forgets a display the library dropped for good. */
export declare function forgetHandle(handle: number): void;
/**
 * The object for a display the library made (the one it keeps for an output no display of the script presents): it presents
 * `output` on a viewport of its own.
 */
export declare function wrapDisplay(native: NativeDisplay, id: string, mode: DisplayModeName, output: OutputView): Display;
