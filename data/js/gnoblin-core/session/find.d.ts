/** A scalar value that a `gnoblin.find` table can match. */
export type Scalar = string | number | boolean;
/** The live object lists searched by `gnoblin.find`. */
export interface FindSource {
    /** Returns the current windows in shell order. */
    windows(): object[];
    /** The current workspaces in shell order, or null when this shell has none. */
    workspaces(): object[] | null;
    /** The live outputs, or null when this shell has none. */
    outputs(): object[] | null;
}
/** Returns the kinds available from this shell. */
export declare function findKinds(source: FindSource): string[];
/**
 * Returns matching live objects in list order. Table filters compare scalar fields for equality; predicate filters
 * keep objects for which the function returns a truthy value. Unknown kinds or fields and invalid values throw.
 */
export declare function find(source: FindSource, kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)): object[];
