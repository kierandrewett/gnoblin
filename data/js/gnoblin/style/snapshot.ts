// Putting styles back: a failed `requestTransition` restores what its callback changed.

import type { StyleElement } from "gnoblin/style/element";
import { own, storeOf } from "gnoblin/style/lists";
import type { ListName, ScalarName } from "gnoblin/style/lists";
import { styleState } from "gnoblin/style/state";
import type { Style } from "gnoblin/style/style";

/**
 * Takes the state of the styles `current()` yields (the scalars, the lists and each list element's fields) and returns
 * what puts it back. A style that `current()` yields only later (made after the snapshot) is emptied. A failed
 * `requestTransition` uses it so that the style writes of its callback leave nothing behind. A list handed out before is
 * not the list that comes back: the script reads it again.
 */
export function snapshotStyles(current: () => Iterable<Style>): () => void {
    interface Saved {
        scalars: Map<ScalarName, unknown>;
        lists: Map<ListName, { element: StyleElement; fields: Record<string, unknown> }[]>;
    }
    const saved = new Map<Style, Saved>();
    for (const style of current()) {
        const store = storeOf(style);
        saved.set(style, {
            scalars: new Map(store.scalars),
            lists: new Map(
                [...store.lists].map(([name, list]) => [
                    name,
                    list.map((element) => ({ element, fields: element.toJSON() })),
                ]),
            ),
        });
    }
    return () => {
        for (const style of current()) {
            const store = storeOf(style);
            const before = saved.get(style);
            store.scalars = new Map(before?.scalars);
            store.lists = new Map();
            store.seen = new Map();
            store.views = new Map();
            for (const [name, entries] of before?.lists ?? []) {
                own(
                    store,
                    name,
                    entries.map((entry) => entry.element),
                );
                for (const { element, fields } of entries) {
                    // A filter writes its `kind` out for the reader of `toJSON`; it is not a field to write back.
                    const { kind: _kind, ...values } = fields;
                    styleState.building += 1;
                    try {
                        Object.assign(element, values);
                    } finally {
                        styleState.building -= 1;
                    }
                }
            }
        }
        styleState.epoch += 1;
    };
}
