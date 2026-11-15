// What the style objects share: the counter the host reads to know that some style changed, the count of elements
// being built (which are not changes), and the private store of every style and the home of every list element.
// Each lives in a table here because a script must not reach it through the object it describes.

import type { Home, StyleElement } from "gnoblin/style/element";
import type { Store } from "gnoblin/style/lists";
import type { Style } from "gnoblin/style/style";

/** The one table of what the style objects share. */
export const styleState = {
    /** Bumped by every accepted change of any style object. The host re-resolves a client when it moved. */
    epoch: 0,
    /** Above zero while elements are built, which are not changes. */
    building: 0,
    /** Maps each list element to its list and error path; detached elements have their class name as the path. */
    homes: new WeakMap<StyleElement, Home>(),
    /** What each style object holds. A WeakMap, so that a script cannot reach it through the object. */
    stores: new WeakMap<Style, Store>(),
};

/**
 * A number that goes up by one for every accepted change of any style object. The host compares it between frames
 * and re-resolves a client's style when it moved.
 */
export function styleEpoch(): number {
    return styleState.epoch;
}
