// Finds live windows, workspaces and outputs by field values or a predicate.

import { camel } from "gnoblin-core/session/names";

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

/** Scalar fields that each kind can match. */
const FIELDS: Record<string, readonly string[]> = {
    Window: [
        "id",
        "appId",
        "title",
        "alive",
        "focused",
        "fullscreen",
        "secure",
        "unresponsive",
        "floating",
        "maximised",
        "minimised",
        "tiled",
        "visible",
        "x",
        "y",
        "width",
        "height",
        "opacity",
        "z",
    ],
    Workspace: ["id", "name", "alive", "index", "active"],
};

/** Returns the kinds available from this shell. */
export function findKinds(source: FindSource): string[] {
    return [
        "Window",
        ...(source.workspaces() === null ? [] : ["Workspace"]),
        ...(source.outputs() === null ? [] : ["Output"]),
    ];
}

/**
 * Returns matching live objects in list order. Table filters compare scalar fields for equality; predicate filters
 * keep objects for which the function returns a truthy value. Unknown kinds or fields and invalid values throw.
 */
export function find(source: FindSource, kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)) {
    const kinds = findKinds(source);
    if (!kinds.includes(kind)) {
        throw new TypeError(`gnoblin.find: no kind named ${JSON.stringify(kind)}; the kinds are ${kinds.join(", ")}`);
    }
    const objects =
        kind === "Window"
            ? source.windows()
            : kind === "Workspace"
              ? (source.workspaces() ?? [])
              : (source.outputs() ?? []);
    if (filter === undefined) {
        return [...objects];
    }
    if (typeof filter === "function") {
        return objects.filter((object) => filter(object as never));
    }
    if (typeof filter !== "object" || filter === null || Array.isArray(filter)) {
        throw new TypeError("gnoblin.find: the filter is a table of field values or a predicate function");
    }
    // Convert filter keys and validate them before searching.
    const wanted = Object.entries(filter).map(([key, value]) => {
        const field = camel(key);
        const fields = FIELDS[kind] ?? scalarFields(objects[0]);
        if (!fields.includes(field)) {
            throw new TypeError(
                `gnoblin.find: ${kind} has no scalar field ${JSON.stringify(key)}; the fields are ${fields.join(", ")}`,
            );
        }
        if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") {
            throw new TypeError(`gnoblin.find: the value of ${key} must be a string, a number or a boolean`);
        }
        return [field, value] as const;
    });
    return objects.filter((object) => wanted.every(([field, value]) => Reflect.get(object, field) === value));
}

/** Lists scalar properties for kinds without a fixed field list. */
function scalarFields(sample: object | undefined): string[] {
    if (sample === undefined) {
        return [];
    }
    const names = new Set<string>();
    for (let prototype: object | null = sample; prototype !== null && prototype !== Object.prototype;) {
        for (const name of Object.getOwnPropertyNames(prototype)) {
            const value = Object.getOwnPropertyDescriptor(prototype, name)?.get ? Reflect.get(sample, name) : undefined;
            if (["string", "number", "boolean"].includes(typeof value)) {
                names.add(name);
            }
        }
        prototype = Object.getPrototypeOf(prototype) as object | null;
    }
    return [...names];
}
