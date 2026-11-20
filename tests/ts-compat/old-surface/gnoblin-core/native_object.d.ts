/**
 * The abstract base of the shell's objects. It cannot be constructed itself; a subclass supplies `id` and `alive`.
 */
export declare class NativeObject {
    constructor();
    /**
     * Writes several properties in order, each through the property's own setter, so each is checked as a single
     * write is. Every name is checked first: an unknown name, a method or a read-only property throws a `TypeError`
     * before anything is written. A value a setter refuses throws that setter's error and leaves the properties
     * after it unwritten. A subclass that can animate (`Client`, `Workspace`) overrides this to take `options`; here
     * `options` is refused, because a plain object has nothing to interpolate.
     */
    set(properties: object, options?: unknown): void;
}
