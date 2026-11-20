/** Whether the library refused a value for being of the wrong kind for what it was written to. */
export declare function wrongKind(error: unknown): boolean;
/** Whether the library cannot hold writes of the node: it is not in the tree the library reads, or its class is not in the core's catalog. */
export declare function unknownNode(error: unknown): boolean;
/**
 * The refusal word of the host (`unknown-node`, `invalid-value`) for a library failure: what a write through `host.set` answers, which
 * `errorCodeOfRefusal` turns into a code at the one place that does.
 */
export declare function refusalOf(error: unknown): string;
/**
 * The error a script sees for `error`, which a table function threw: a `TypeError` for a value of the wrong kind, a `GnoblinError`
 * for the rest, with `fallback` as its target when the text names none. Anything that is not a library failure is returned as it is.
 */
export declare function stdFailure(error: unknown, fallback: string): unknown;
