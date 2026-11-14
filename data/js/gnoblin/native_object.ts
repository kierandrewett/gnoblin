// NativeObject: the base every shell object derives from (owner-spec.md section 4).
//
// The specification declares `abstract class NativeObject { id; alive; set(); addListener(); removeListener() }` and
// makes the compositor-backed classes extend it, so a script can ask `value instanceof NativeObject` and a class it
// writes itself (a binding, a listener holder) can derive from it. The members a class gets for free are the ones that
// need no native state: `set`, which writes several properties through the object's own checked setters. `id`, `alive`
// and the listener methods belong to the objects that have a native identity (a `Client`, a `Workspace`, an
// `Output`, an `InputContext`), and each declares them itself.

/**
 * The abstract base of the shell's objects. It cannot be constructed itself; a subclass supplies `id` and `alive`.
 */
export class NativeObject {
    constructor() {
        if (new.target === NativeObject) {
            throw new TypeError("NativeObject is abstract: extend it");
        }
    }

    /**
     * Writes several properties in order, each through the property's own setter, so each is checked as a single
     * write is. Every name is checked first: an unknown name, a method or a read-only property throws a `TypeError`
     * before anything is written. A value a setter refuses throws that setter's error and leaves the properties
     * after it unwritten. A subclass that can animate (`Client`, `Workspace`) overrides this to take `options`; here
     * `options` is refused, because a plain object has nothing to interpolate.
     */
    set(properties: object, options?: unknown): void {
        const label = this.constructor.name;
        if (properties === null || typeof properties !== "object" || Array.isArray(properties)) {
            throw new TypeError(`${label}.set: expects an object of properties`);
        }
        if (options !== undefined) {
            throw new TypeError(`${label}.set: this object takes no transition options`);
        }
        const slots = this as unknown as Record<string, unknown>;
        const writable = new Map<string, unknown>(Object.entries(properties));
        for (const name of writable.keys()) {
            if (!(name in this) || typeof slots[name] === "function") {
                throw new TypeError(`${label}.set: no property "${name}"`);
            }
            if (!isWritable(this, name)) {
                throw new TypeError(`${label}.set: "${name}" is read-only`);
            }
        }
        for (const [name, value] of writable) {
            slots[name] = value;
        }
    }
}

/** Whether assigning `name` on `target` would be accepted: an own or inherited data property, or an accessor with a setter. */
function isWritable(target: object, name: string): boolean {
    for (let at: object | null = target; at !== null; at = Object.getPrototypeOf(at)) {
        const descriptor = Object.getOwnPropertyDescriptor(at, name);
        if (descriptor !== undefined) {
            return descriptor.set !== undefined || descriptor.writable === true;
        }
    }
    return false;
}
