// `context.bind(trigger, action)`: how a function or a `Binding` becomes a binding of a collection, and the two small
// parsers of a seat's device names that the context uses.

import { GestureBinding, KeyBinding } from "gnoblin/bindings/binding";
import type { Binding, BindingCollection } from "gnoblin/bindings/binding";
import { parseTrigger } from "gnoblin/bindings/trigger";
import type { GestureTriggerParts } from "gnoblin/bindings/trigger_types";
import type { BindingEvent, DragEvent } from "gnoblin/bindings/types";
import { PINCH_COMMIT, SWIPE_COMMIT } from "gnoblin/input/gesture";

/** Numbers the bindings that a bare function made, so each has an id of its own. */
let bindingSerial = 0;

/** A function bound to a gesture: it runs once, when the gesture ends having gone far enough its way. */
class GestureActionBinding extends GestureBinding {
    readonly #action: (event: BindingEvent) => void | false;

    constructor(id: string, trigger: string, action: (event: BindingEvent) => void | false) {
        super({ id, trigger });
        this.#action = action;
    }

    onStart(): object {
        return {};
    }

    onEnd(event: DragEvent): void {
        const trigger = this.parsed as GestureTriggerParts;
        const commit = trigger.gesture === "pinch" ? PINCH_COMMIT : SWIPE_COMMIT;
        if ((event.progress ?? 0) >= commit) {
            this.#action(event);
        }
    }
}

/**
 * Binds `action` to `trigger` in `collection`. A function becomes a `KeyBinding` (a key or a click) or a
 * gesture action; `false` is a function that does nothing, so the trigger is claimed and swallowed. A drag needs the
 * hooks of a `Binding`, so a bare function for one is a mistake. A `Binding`
 * instance is added as it is, moved to `trigger` when it spells another.
 */
export function bindIn(
    collection: BindingCollection,
    trigger: string,
    action: ((event: BindingEvent) => void | false) | Binding | false,
    what: string,
): Binding {
    // `false` claims the trigger and does nothing with it: the chord is withheld from clients and nothing runs,
    // as the Lua shell's `seat:bind({ chord = false })` did.
    if (action === false) {
        return bindIn(collection, trigger, () => undefined, what);
    }
    if (typeof action === "function") {
        const parsed = parseTrigger(trigger, `${what}.bind`);
        if (parsed.kind === "pointer" && parsed.action === "drag") {
            throw new TypeError(
                `${what}.bind: "${parsed.text}" is a drag; bind a PointerDragBinding with onStart, onUpdate and onEnd`,
            );
        }
        bindingSerial += 1;
        const id = `bind-${bindingSerial}:${parsed.text}`;
        const binding =
            parsed.kind === "gesture"
                ? new GestureActionBinding(id, trigger, action)
                : new KeyBinding({ id, trigger, action });
        return collection.add(binding);
    }
    if (typeof action !== "object" || action === null || typeof action.onStart !== "function") {
        throw new TypeError(`${what}.bind: the action must be a function or a Binding`);
    }
    if (action.trigger !== trigger) {
        action.trigger = trigger;
    }
    return collection.add(action);
}

/** A device name is one non-empty line: the compositor matches it exactly against what a device reports. */
export function checkDeviceName(name: unknown, what: string): void {
    if (typeof name !== "string" || name === "" || name.includes("\n")) {
        throw new TypeError(`${what}: a device name is a non-empty line of text from devices, got ${String(name)}`);
    }
}

/** One `id:name:caps` item of a seat's device list: the id up to the first colon, the capabilities after the last. */
export function splitDevice(item: string): { id: string; name: string; capabilities: string[] } | null {
    const first = item.indexOf(":");
    const last = item.lastIndexOf(":");
    if (first <= 0 || last <= first) {
        return null;
    }
    const caps = item.slice(last + 1);
    return {
        id: item.slice(0, first),
        name: item.slice(first + 1, last),
        capabilities: caps === "" ? [] : caps.split(","),
    };
}
