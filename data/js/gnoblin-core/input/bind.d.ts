import type { Binding, BindingCollection } from "gnoblin-core/bindings/binding";
import type { BindingEvent } from "gnoblin-core/bindings/types";
/**
 * Binds `action` to `trigger` in `collection`. A function becomes a `KeyBinding` (a key or a click) or a
 * gesture action; `false` is a function that does nothing, so the trigger is claimed and swallowed. A drag needs the
 * hooks of a `Binding`, so a bare function for one is a mistake. A `Binding`
 * instance is added as it is, moved to `trigger` when it spells another.
 */
export declare function bindIn(collection: BindingCollection, trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false, what: string): Binding;
/** A device name is one non-empty line: the compositor matches it exactly against what a device reports. */
export declare function checkDeviceName(name: unknown, what: string): void;
/** One `id:name:caps` item of a seat's device list: the id up to the first colon, the capabilities after the last. */
export declare function splitDevice(item: string): {
    id: string;
    name: string;
    capabilities: string[];
} | null;
