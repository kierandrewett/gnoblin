import type { Home, StyleElement } from "gnoblin-std/style/element";
import type { Store } from "gnoblin-std/style/lists";
import type { Style } from "gnoblin-std/style/style";
/** The one table of what the style objects share. */
export declare const styleState: {
    /** Bumped by every accepted change of any style object. The host re-resolves a client when it moved. */
    epoch: number;
    /** Above zero while elements are built, which are not changes. */
    building: number;
    /** Maps each list element to its list and error path; detached elements have their class name as the path. */
    homes: WeakMap<StyleElement, Home>;
    /** What each style object holds. A WeakMap, so that a script cannot reach it through the object. */
    stores: WeakMap<Style, Store>;
};
/**
 * A number that goes up by one for every accepted change of any style object. The host compares it between frames
 * and re-resolves a client's style when it moved.
 */
export declare function styleEpoch(): number;
