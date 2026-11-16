// The properties of a client that a timed write (`Client.set` with a duration) can move.

/** The properties the tree takes a timed write of, and the kind of value each holds. */
export const ANIMATED_KEYS: Readonly<Record<string, "number" | "vector">> = {
    x: "number",
    y: "number",
    width: "number",
    height: "number",
    opacity: "number",
    z: "number",
    rotation: "vector",
    scale: "vector",
};
